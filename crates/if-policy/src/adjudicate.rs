//! 分层裁决与观察带（docs/06 §4–§5）。
//!
//! 这一层是「Jev 只输出分布，引擎按策略裁决」里的**引擎**部分。它不含任何模型调用：
//! 候选与概率都是输入，输出只有两样——裁决记录（谁发生了、谁被否决了、用的哪颗骰子），
//! 以及新建的趋势。
//!
//! 三条规则决定了它的形状：
//!
//! 1. **约束类不掷骰**（docs/06 §1）。只有发生类走命运骰子；L1 保护期内的改动也改走阈值。
//! 2. **互斥组只抽一次**（docs/06 §4）。组内的候选不再单独掷骰。
//! 3. **被否决但落在观察带内的候选不会消失**（docs/06 §5）——
//!    它们变成趋势，压力是 `p × 0.5`，之后靠推动量涨到爆发。

use std::collections::{BTreeMap, BTreeSet};

use if_domain::{
    Candidate, CandidateId, EventId, MechanicStep, PropositionId, Resolution, ResolutionOutcome,
    ResolutionPolicy, Tendency, TendencyId, WorldSettings, WorldTime,
};

use crate::dice::Dice;
use crate::layers::{self, LayerError, Layered};
use crate::threshold::{Strictness, ThresholdSense, ThresholdSpec, Thresholds, L1_TEMPLATE};

/// 一次裁决的确定性上下文。
///
/// 它只装影响结果的东西：种子（骰子）、世界时间段粒度、世界时间、场景序号、
/// 盐值、阈值表与严格度。同一个 `Policy` 配同一个输入，必须得到同一个结果——
/// 重放时不重新询问 Jev，靠的就是这一点（docs/03 §8）。
#[derive(Clone, Debug)]
pub struct Policy {
    dice: Dice,
    step: MechanicStep,
    at: WorldTime,
    scene_index: u64,
    salt: String,
    thresholds: Thresholds,
    strictness: Strictness,
}

impl Policy {
    /// 从世界设置建一个策略。种子属于世界，不属于全局设置（docs/03 §8）。
    pub fn new(settings: &WorldSettings, at: WorldTime, scene_index: u64) -> Self {
        Self {
            dice: Dice::new(settings.seed),
            step: settings.mechanic_step,
            at,
            scene_index,
            salt: String::new(),
            thresholds: Thresholds::default(),
            strictness: Strictness::default(),
        }
    }

    /// 换一个盐值，用于重掷（docs/03 §8）。
    pub fn with_salt(mut self, salt: impl Into<String>) -> Self {
        self.salt = salt.into();
        self
    }

    pub fn with_thresholds(mut self, thresholds: Thresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    pub fn with_strictness(mut self, strictness: impl Into<Strictness>) -> Self {
        self.strictness = strictness.into();
        self
    }

    pub fn dice(&self) -> Dice {
        self.dice
    }

    pub fn salt(&self) -> &str {
        &self.salt
    }

    pub fn step(&self) -> MechanicStep {
        self.step
    }

    pub fn at(&self) -> WorldTime {
        self.at
    }

    pub fn scene_index(&self) -> u64 {
        self.scene_index
    }

    pub fn thresholds(&self) -> &Thresholds {
        &self.thresholds
    }

    pub fn strictness(&self) -> Strictness {
        self.strictness
    }

    /// 按模板取（已施加一致性严格度的）规则。
    pub fn threshold(&self, template: &str) -> Option<ThresholdSpec> {
        self.thresholds.spec_with(template, self.strictness)
    }

    /// 抽查一条约束类判定。模板不存在时返回 `None`——调用方得自己决定这意味着什么，
    /// 悄悄当作「通过」会让拼错的模板 ID 永远不被发现。
    pub fn passes(&self, template: &str, probability: f64) -> Option<bool> {
        self.threshold(template)
            .map(|spec| spec.passes(probability))
    }

    /// 分层并裁决。
    pub fn run(&self, input: &AdjudicationInput) -> Result<Adjudication, LayerError> {
        let Layered { layers, deferred } = layers::layer(&input.candidates)?;
        let lookup = Lookup::build(input);
        let by_id: BTreeMap<&CandidateId, &Candidate> =
            input.candidates.iter().map(|c| (&c.id, c)).collect();

        let mut result = Adjudication {
            deferred,
            unstable: lookup.unstable.clone(),
            ..Default::default()
        };
        // 决策键 → （已选中的选项, 那次抽取的骰子值）。
        // 同一决策键的互斥组共用一次抽取（docs/06 §4），所以它跨越整个回合存活。
        let mut chosen: BTreeMap<String, (String, f64)> = BTreeMap::new();

        for (depth, ids) in layers.iter().enumerate() {
            let mut outcome = LayerOutcome {
                depth,
                ..Default::default()
            };
            for id in ids {
                let Some(candidate) = by_id.get(id) else {
                    continue;
                };
                let (resolution, tendency) =
                    resolve_candidate(self, candidate, &lookup, &mut chosen);
                if resolution.accepted() {
                    outcome.accepted.push(id.clone());
                } else {
                    outcome.rejected.push(id.clone());
                }
                if let Some(tendency) = tendency {
                    outcome.tendencies.push(tendency.id.clone());
                    result.tendencies.push(tendency);
                }
                result.resolutions.push(resolution);
            }
            result.layers.push(outcome);
        }

        // 因果深度超上限的候选：docs/06 §4 给了「留到下一回合**或**转为趋势」两条路。
        // 现在还没有「下一回合候选池」这个存储，所以走能落地的那条——
        // 有概率且落在观察带内的转成趋势，其余只留在 `deferred` 里等调用方处理。
        for id in result.deferred.clone() {
            let (Some(candidate), Some(probability)) =
                (by_id.get(&id), input.probabilities.get(&id))
            else {
                continue;
            };
            if Tendency::qualifies_for_watch(*probability) {
                let key = lookup.key_of(candidate);
                result.tendencies.push(watch_tendency(
                    candidate,
                    &key,
                    *probability,
                    lookup.source_event,
                ));
            }
        }

        if !result.unstable.is_empty() {
            result.warnings.push(format!(
                "{} 个候选没有稳定决策键，只能拿候选 ID 兜底；本回合能跑通，但跨世界线不可复现（docs/03 §8）",
                result.unstable.len()
            ));
        }

        Ok(result)
    }
}

// ---------------------------------------------------------------- 输入

/// 裁决输入：候选，加上判定者给出的概率。
///
/// 概率缺失是**正常情况**而不是错误——docs/06 §9 规定了降级：约束类视为不通过，
/// 发生类视为不发生。所以这里用 `BTreeMap` 查表，而不是要求每个候选都有值。
#[derive(Clone, Debug, Default)]
pub struct AdjudicationInput {
    pub candidates: Vec<Candidate>,
    /// 候选 → 概率（发生类与约束类）。
    pub probabilities: BTreeMap<CandidateId, f64>,
    /// 互斥组长 → 分布。组长是携带 `Exclusive` 的那个候选。
    pub distributions: BTreeMap<CandidateId, BTreeMap<String, f64>>,
    /// 有明确触发事件的候选。L1 保护期内改变受保护状态时必须（docs/06 §4）。
    pub triggered: BTreeSet<CandidateId>,
    /// 受 L1 保护的命题，由视图层按当前场景序号算出。
    pub protected: BTreeSet<PropositionId>,
    /// 命题 ID → 规范键。给没带显式决策键的候选推键用（docs/03 §8）。
    pub proposition_keys: BTreeMap<PropositionId, String>,
    /// 趋势的贡献事件，通常是本回合的触发事件。
    pub source_event: Option<EventId>,
}

impl AdjudicationInput {
    pub fn new(candidates: Vec<Candidate>) -> Self {
        Self {
            candidates,
            ..Default::default()
        }
    }

    pub fn with_probability(mut self, id: impl Into<CandidateId>, probability: f64) -> Self {
        self.probabilities.insert(id.into(), probability);
        self
    }

    pub fn with_distribution(
        mut self,
        id: impl Into<CandidateId>,
        distribution: BTreeMap<String, f64>,
    ) -> Self {
        self.distributions.insert(id.into(), distribution);
        self
    }

    pub fn triggering(mut self, id: impl Into<CandidateId>) -> Self {
        self.triggered.insert(id.into());
        self
    }

    pub fn with_proposition_key(
        mut self,
        id: impl Into<PropositionId>,
        key: impl Into<String>,
    ) -> Self {
        self.proposition_keys.insert(id.into(), key.into());
        self
    }

    pub fn protecting(mut self, props: impl IntoIterator<Item = PropositionId>) -> Self {
        self.protected.extend(props);
        self
    }

    pub fn from_event(mut self, event: impl Into<EventId>) -> Self {
        self.source_event = Some(event.into());
        self
    }

    /// 每个候选的稳定决策键。缺键时回退到 `affects` 的首个命题键——
    /// 状态候选的键本来就是它所影响的命题键。
    fn decision_keys(&self) -> BTreeMap<CandidateId, String> {
        let mut keys = BTreeMap::new();
        for candidate in &self.candidates {
            if let Some(key) =
                candidate.decision_key(|prop| self.proposition_keys.get(prop).cloned())
            {
                keys.insert(candidate.id.clone(), key);
            }
        }
        keys
    }
}

// ---------------------------------------------------------------- 输出

/// 一个因果层的裁决结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerOutcome {
    /// 因果深度，0 基。同一层内部的判定可以并行发出。
    pub depth: usize,
    pub accepted: Vec<CandidateId>,
    pub rejected: Vec<CandidateId>,
    /// 本层新产生的趋势。
    pub tendencies: Vec<TendencyId>,
}

/// 一次分层裁决的全部产出。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adjudication {
    /// 逐层结果，层号即因果深度。
    pub layers: Vec<LayerOutcome>,
    /// 因果深度超上限、本回合不裁决的候选（docs/06 §4）。
    pub deferred: Vec<CandidateId>,
    /// 本回合新建的趋势（docs/06 §5）。
    pub tendencies: Vec<Tendency>,
    /// 全部裁决记录，顺序与层序一致。
    pub resolutions: Vec<Resolution>,
    /// 没有稳定决策键、只能拿候选 ID 兜底的候选。
    pub unstable: BTreeSet<CandidateId>,
    /// 引擎侧警告，给裁定卡与日志看。
    pub warnings: Vec<String>,
}

impl Adjudication {
    /// 放行的候选，按层序。
    pub fn accepted(&self) -> Vec<&CandidateId> {
        self.layers.iter().flat_map(|l| l.accepted.iter()).collect()
    }

    /// 被否决的候选，按层序。
    pub fn rejected(&self) -> Vec<&CandidateId> {
        self.layers.iter().flat_map(|l| l.rejected.iter()).collect()
    }

    pub fn resolution(&self, id: &CandidateId) -> Option<&Resolution> {
        self.resolutions
            .iter()
            .find(|resolution| resolution.target == id.as_str())
    }

    /// 某个互斥组最终选中的选项。
    pub fn selected_option(&self, id: &CandidateId) -> Option<&str> {
        match &self.resolution(id)?.outcome {
            ResolutionOutcome::Selected { option } => Some(option),
            _ => None,
        }
    }

    /// 某个候选贡献的那条趋势（被否决且落在观察带内时才有）。
    pub fn tendency_of(&self, id: &CandidateId) -> Option<&Tendency> {
        let key = self.resolution(id)?.decision_key.as_deref()?;
        let wanted = format!("{}{key}", TendencyId::PREFIX);
        self.tendencies.iter().find(|t| t.id.as_str() == wanted)
    }
}

// ---------------------------------------------------------------- 内部

/// 裁决期间共享的查表。把六七个参数收成一个，免得函数签名比函数体长。
struct Lookup<'a> {
    probabilities: &'a BTreeMap<CandidateId, f64>,
    /// 决策键 → 互斥分布。
    ///
    /// 按**决策键**索引而不是候选 ID：互斥组的组长由 LLM 指定，分层排序会把组内候选
    /// 打散，组长未必排在组员前面。按键找之后，顺序不再影响结果。
    distributions: BTreeMap<String, BTreeMap<String, f64>>,
    triggered: &'a BTreeSet<CandidateId>,
    protected: &'a BTreeSet<PropositionId>,
    keys: BTreeMap<CandidateId, String>,
    unstable: BTreeSet<CandidateId>,
    source_event: Option<&'a EventId>,
}

impl<'a> Lookup<'a> {
    fn build(input: &'a AdjudicationInput) -> Self {
        let keys = input.decision_keys();
        let unstable = input
            .candidates
            .iter()
            .filter(|c| !keys.contains_key(&c.id))
            .map(|c| c.id.clone())
            .collect();

        // 按候选 ID 排序后遍历：同一个决策键上万一挂了不止一份分布，
        // 「取第一份」才是确定的。
        let mut ordered: Vec<&Candidate> = input.candidates.iter().collect();
        ordered.sort_by(|a, b| a.id.cmp(&b.id));
        let mut distributions = BTreeMap::new();
        for candidate in ordered {
            let Some(distribution) = input.distributions.get(&candidate.id) else {
                continue;
            };
            let key = keys
                .get(&candidate.id)
                .cloned()
                .unwrap_or_else(|| candidate.id.as_str().to_string());
            distributions.entry(key).or_insert_with(|| distribution.clone());
        }

        Self {
            probabilities: &input.probabilities,
            distributions,
            triggered: &input.triggered,
            protected: &input.protected,
            keys,
            unstable,
            source_event: input.source_event.as_ref(),
        }
    }

    /// 候选的决策键。没有稳定键时退回候选 ID——本回合能跑通，跨世界线不可复现。
    fn key_of(&self, candidate: &Candidate) -> String {
        self.keys
            .get(&candidate.id)
            .cloned()
            .unwrap_or_else(|| candidate.id.as_str().to_string())
    }

    fn distribution_of(&self, candidate: &Candidate) -> Option<&BTreeMap<String, f64>> {
        self.distributions.get(&self.key_of(candidate))
    }
}

fn resolve_candidate(
    policy: &Policy,
    candidate: &Candidate,
    lookup: &Lookup<'_>,
    chosen: &mut BTreeMap<String, (String, f64)>,
) -> (Resolution, Option<Tendency>) {
    let id = &candidate.id;
    let key = lookup.key_of(candidate);

    // ---- L1 保护期：不参与抽样，改用阈值，且必须有明确的触发事件（docs/06 §4）
    if candidate
        .affects
        .iter()
        .any(|prop| lookup.protected.contains(prop))
    {
        // 表里查不到就关死：宁可挡住对受保护状态的改动，也不要因为一张表的疏漏放行。
        let spec = policy
            .threshold(L1_TEMPLATE)
            .unwrap_or(ThresholdSpec::new(1.0, ThresholdSense::HigherPasses));
        // docs/06 §9：约束类缺判定视为不通过。
        let probability = lookup.probabilities.get(id).copied().unwrap_or(0.0);
        let triggered = lookup.triggered.contains(id);
        let passed = triggered && spec.passes(probability);
        // 不转趋势：趋势是「概率落进观察带」的产物，这里的否决来自缺依据或阈值不够，
        // 两者不是一回事——把后者塞进趋势，玩家会看到一个没有来源的压力条。
        return (Resolution::by_threshold(id.as_str(), passed), None);
    }

    // ---- 互斥组：整组只抽一次（docs/06 §4）
    if candidate.options().is_some() {
        if let Some((option, die)) = chosen.get(&key) {
            // 组内其他候选复用同一次抽取，连骰子值都一样——
            // 这就是「组内的候选不再单独掷骰」在记录里的样子。
            return (
                Resolution::by_categorical(id.as_str(), option.clone(), key, *die),
                None,
            );
        }
        let die = policy.dice().roll_salted(&key, policy.salt());
        let picked = lookup
            .distribution_of(candidate)
            .and_then(|distribution| policy.dice().categorical(&key, distribution, policy.salt()));
        let Some(option) = picked else {
            // docs/06 §9 只规定了约束类与发生类的降级，互斥类没写。
            // 这里的取舍是「不选任何一项」：随便挑一个会让不自洽悄悄流进正文。
            return (
                missing_judgment(id, &key, ResolutionPolicy::SeededCategorical),
                None,
            );
        };
        chosen.insert(key.clone(), (option.clone(), die));
        return (
            Resolution::by_categorical(id.as_str(), option, key, die),
            None,
        );
    }

    // ---- 发生类：带种子抽样（docs/06 §1）
    let Some(probability) = lookup.probabilities.get(id).copied() else {
        // docs/06 §9：发生类缺判定视为不发生。不掷骰——没有概率就没有抽样。
        return (
            missing_judgment(id, &key, ResolutionPolicy::SeededSample),
            None,
        );
    };
    let die = policy.dice().roll_salted(&key, policy.salt());
    let resolution = Resolution::by_die(id.as_str(), probability, key.clone(), die);
    // 被否决、但概率落在观察带内 → 转为趋势（docs/06 §5）。
    let tendency = if !resolution.accepted() && Tendency::qualifies_for_watch(probability) {
        Some(watch_tendency(
            candidate,
            &key,
            probability,
            lookup.source_event,
        ))
    } else {
        None
    };
    (resolution, tendency)
}

/// 判定缺失时的降级记录（docs/06 §9）：约束类不通过，发生类不发生。
///
/// 保留决策键，事后能对上「是哪一步缺了判定」。
fn missing_judgment(id: &CandidateId, key: &str, policy: ResolutionPolicy) -> Resolution {
    Resolution {
        target: id.as_str().to_string(),
        policy,
        decision_key: Some(key.to_string()),
        die: None,
        outcome: ResolutionOutcome::Accepted { accepted: false },
    }
}

/// 从被否决的候选建一条趋势。
///
/// ID 由决策键推出来，所以同一回合重跑必然得到同一个 ID——
/// 否则投影会随重放漂移，趋势的压力也就会每次都不一样。
fn watch_tendency(
    candidate: &Candidate,
    key: &str,
    probability: f64,
    source: Option<&EventId>,
) -> Tendency {
    let id = TendencyId::new(format!("{}{key}", TendencyId::PREFIX));
    let mut tendency = match source {
        Some(event) => Tendency::from_failed_candidate(
            id,
            candidate.content.clone(),
            probability,
            event.clone(),
        ),
        None => Tendency::latent(id, candidate.content.clone(), probability),
    };
    // 爆发后要对应的命题：优先用候选声明的第一个影响对象。
    tendency.target = candidate
        .affects
        .first()
        .map(|prop| prop.as_str().to_string())
        .unwrap_or_default();
    tendency
}

#[cfg(test)]
mod tests;
