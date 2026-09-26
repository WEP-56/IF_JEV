//! 第 8–9 步：场景候选 → 受保护线的硬否决 → 导演评分 → 骰子抽取（docs/04 §2、docs/06 §6）。
//!
//! 这一层要防的是**写什么都一个样**：只取最高分会永远挑同一种场景，纯随机又会
//! 把该收的线一直拖着。所以是「一组加权项求和 → 按 `P(i) ∝ exp(scoreᵢ / T)` 抽取」——
//! 分高的场景大概率被选中，但不是必然。
//!
//! 评分项分两半，来源必须分清（docs/06 §6）：
//!
//! | 项 | 来源 | 缺判定时 |
//! |---|---|---|
//! | `semantic_fit` / `tension_gain` / `repetition` / `premature_resolution` | Jev | 0（不给分也不罚分） |
//! | `thread_pressure` / `overdue_bonus` / `character_balance` / `tendency_bonus` | 引擎算 | — |
//!
//! **受保护的故事线不进评分**：任何会让它的核心问题得到最终回答的场景，在
//! 请求发出之前就被引擎否决（docs/04 §2.1）。这是硬否决，不由概率决定——
//! 一旦落到「p 刚好超过阈值所以这条线提前收束了」，保护期就没有意义了。
//!
//! `q.scene.advances_thread` 这一问**故意不问**：它不在 [`SceneSignals`] 的任何一项里，
//! 问了也只会留下一批没人消费的判定记录。等导演评分真的用到「推进了哪条线」再加。

use std::collections::{BTreeMap, BTreeSet};

use if_domain::id::{SceneId, SubjectId, TendencyId, ThreadId};
use if_domain::narrative::{TendencyStatus, Thread};
use if_domain::projection::Projection;
use if_domain::rule::WorldSettings;
use if_domain::turn::{Judgment, JudgmentOutput, ViewKind};
use if_judge::{Judge, JudgeRequest, JudgeResponse};
use if_policy::director::{self, DirectorWeights, SceneSignals};
use if_policy::{scene_select_key, Dice, NO_SALT};

use crate::audit::Audit;
use crate::context::TurnContext;
use crate::question;
use crate::PipelineError;

/// 默认的出场平衡窗口（场景数）【初始值 6】。
pub const DEFAULT_BALANCE_WINDOW: u64 = 6;

/// 一个场景候选（T-scenes 的产物；T-impact 的候选已在第 7 步裁决完）。
#[derive(Clone, Debug, PartialEq)]
pub struct SceneProposal {
    pub id: SceneId,
    /// 一句话概括。所有问句都拿它当主语，所以它必须自足——「两人对峙」这种没有主语的
    /// 概括会让判定者只能猜。
    pub summary: String,
    /// 这条场景推进的故事线（用于算压力与逾期）。
    pub threads: Vec<ThreadId>,
    /// 这条场景会让哪些故事线的核心问题**得到最终回答**。
    /// 受保护的线出现在这里 = 直接否决（docs/04 §2 第 8 步）。
    pub resolves: Vec<ThreadId>,
    /// 这条场景会让哪些趋势爆发。引擎核对它们是否真的处于高压——
    /// 声明了但压力不够不算数。
    pub erupts: Vec<TendencyId>,
    /// 焦点主体（导演评分的出场平衡看它）。
    pub focus: Vec<SubjectId>,
    /// 在场主体。选中的场景会用它作为第 10 步场景计划的起点。
    pub present: Vec<SubjectId>,
}

impl SceneProposal {
    pub fn new(id: impl Into<SceneId>, summary: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            summary: summary.into(),
            threads: Vec::new(),
            resolves: Vec::new(),
            erupts: Vec::new(),
            focus: Vec::new(),
            present: Vec::new(),
        }
    }

    pub fn threads(mut self, threads: impl IntoIterator<Item = ThreadId>) -> Self {
        self.threads = threads.into_iter().collect();
        self
    }

    pub fn resolves(mut self, threads: impl IntoIterator<Item = ThreadId>) -> Self {
        self.resolves = threads.into_iter().collect();
        self
    }

    pub fn erupts(mut self, tendencies: impl IntoIterator<Item = TendencyId>) -> Self {
        self.erupts = tendencies.into_iter().collect();
        self
    }

    pub fn cast(mut self, focus: impl IntoIterator<Item = SubjectId>, present: impl IntoIterator<Item = SubjectId>) -> Self {
        self.focus = focus.into_iter().collect();
        self.present = present.into_iter().collect();
        self
    }
}

/// 一次场景选择的输入。
#[derive(Debug)]
pub struct SceneRequest<'a> {
    pub projection: &'a Projection,
    pub settings: &'a WorldSettings,
    pub ctx: &'a TurnContext,
    pub scenes: Vec<SceneProposal>,
    /// 导演风格 = 一组权重。默认按 `settings.director_style` 取。
    pub weights: DirectorWeights,
    /// 叙事温度【初始值 0.15】。越小越接近「只取最高分」。
    pub temperature: f64,
    pub balance_window: u64,
    /// 重掷用的盐值（docs/03 §8）。同一个场景重放必须留空。
    pub salt: String,
}

impl<'a> SceneRequest<'a> {
    pub fn new(
        projection: &'a Projection,
        settings: &'a WorldSettings,
        ctx: &'a TurnContext,
        scenes: Vec<SceneProposal>,
    ) -> Self {
        Self {
            projection,
            settings,
            ctx,
            scenes,
            weights: DirectorWeights::for_style(&settings.director_style),
            temperature: if_policy::DIRECTOR_TEMPERATURE,
            balance_window: DEFAULT_BALANCE_WINDOW,
            salt: String::new(),
        }
    }

    pub fn weights(mut self, weights: DirectorWeights) -> Self {
        self.weights = weights;
        self
    }

    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn salt(mut self, salt: impl Into<String>) -> Self {
        self.salt = salt.into();
        self
    }
}

/// 被引擎硬否决的场景。
#[derive(Clone, Debug, PartialEq)]
pub struct SceneVeto {
    pub scene: SceneId,
    pub reason: String,
}

/// 一个场景的评分，以及它被抽中的概率。
#[derive(Clone, Debug, PartialEq)]
pub struct SceneScore {
    pub scene: SceneId,
    pub summary: String,
    pub score: f64,
    pub probability: f64,
    pub signals: SceneSignals,
}

/// 场景选择的产出。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneChoice {
    pub chosen: Option<SceneScore>,
    /// 选中场景的原始提议——第 10 步的 T-plan 需要它的 `focus` / `present`。
    pub chosen_proposal: Option<SceneProposal>,
    /// 未选之路（docs/04 §2 第 9 步）。按候选 ID 排序，与该场景的分数无关。
    pub road_not_taken: Vec<SceneScore>,
    pub vetoed: Vec<SceneVeto>,
    pub judgments: Vec<Judgment>,
    /// `scene_select@<叙述序号>`。它同时是复现这次抽取的键。
    pub decision_key: Option<String>,
    pub die: Option<f64>,
    pub warnings: Vec<String>,
}

impl SceneChoice {
    /// 抽中的场景提议。
    pub fn proposal(&self) -> Option<&SceneProposal> {
        self.chosen_proposal.as_ref()
    }

    /// 抽中的场景 ID。`None` 表示本回合不产生场景（docs/04 §3：没有候选时跳过）。
    pub fn scene_id(&self) -> Option<&SceneId> {
        self.chosen.as_ref().map(|score| &score.scene)
    }

    /// 本回合选中的场景没有出现。
    pub fn is_empty(&self) -> bool {
        self.chosen.is_none()
    }
}

/// 跑第 8–9 步。
///
/// `audit` 是本回合共用的判定序号游标（见 [`crate::audit`]）。
pub fn choose(
    judge: &dyn Judge,
    request: SceneRequest<'_>,
    audit: &mut Audit,
) -> Result<SceneChoice, PipelineError> {
    let mut warnings: Vec<String> = Vec::new();

    // 候选按 ID 排序后处理：结果必须与输入顺序无关，否则同一个回合换一个提交顺序
    // 就会选到别的场景。
    let mut scenes = request.scenes.clone();
    scenes.sort_by(|a, b| a.id.cmp(&b.id));

    // ---- 硬否决：受保护的线不许被收束（docs/04 §2.1）
    let mut live: Vec<SceneProposal> = Vec::new();
    let mut vetoed: Vec<SceneVeto> = Vec::new();
    for scene in scenes {
        let mut hit: Option<String> = None;
        for thread in &scene.resolves {
            match request.projection.threads.get(thread) {
                Some(thread) => {
                    if let Some(forbidden) = thread.forbidden_resolution() {
                        hit = Some(forbidden);
                        break;
                    }
                }
                None => warnings.push(format!(
                    "场景 {} 声明要收束故事线 {thread}，但世界上没有这条线",
                    scene.id
                )),
            }
        }
        match hit {
            Some(reason) => vetoed.push(SceneVeto {
                scene: scene.id,
                reason,
            }),
            None => live.push(scene),
        }
    }

    if live.is_empty() {
        warnings.push("没有可用的场景候选：全部被受保护的故事线否决，或提议为空".to_owned());
        return Ok(SceneChoice {
            vetoed,
            warnings,
            ..Default::default()
        });
    }

    // ---- 问句：一次请求问完所有候选（同一视图，导演视图）
    let mut view = request.ctx.view(ViewKind::Director);
    view.task = "T-scenes".to_owned();
    let compiled = if_views::compile(request.projection, &view);
    let mut ask = JudgeRequest::new(compiled);
    let recent = recent_goals(request.projection, 3);
    for (index, scene) in live.iter().enumerate() {
        ask.push(question::scene_fit(index, &scene.summary));
        ask.push(question::scene_tension(index, &scene.summary));
        ask.push(question::scene_repetitive(index, &scene.summary, &recent));
        for thread in &scene.resolves {
            if let Some(thread) = request.projection.threads.get(thread) {
                ask.push(question::scene_resolves_thread(
                    index,
                    &scene.summary,
                    &thread.title,
                    &thread.question,
                ));
            }
        }
    }

    let cancel = std::sync::atomic::AtomicBool::new(false);
    let response = judge.judge(&ask, &cancel)?;
    let turn = request.ctx.turn.clone();
    let judgments = response.to_judgments(&ask, &turn, || audit.take());
    if !response.missing.is_empty() {
        warnings.push(format!(
            "场景评分有 {} 个问题没有拿到判定（{}），按 0 分处理",
            response.missing.len(),
            response.missing.join("、")
        ));
    }

    // ---- 引擎侧的四项
    // 「当前是第几个场景」取自回合上下文，**不是** `projection.scenes.len()`：
    // `Thread::last_advanced` 记的是场景序号（docs/02 §10），逾期度必须与它同一个时钟；
    // 被否决而跳过的场景不会进投影，用长度会让逾期度永远低估。
    let current_scene = request.ctx.scene_index;
    let appearances = recent_appearances(request.projection, request.balance_window);
    let erupting: BTreeSet<&TendencyId> = request
        .projection
        .tendencies
        .values()
        .filter(|tendency| {
            tendency.status == TendencyStatus::Latent && tendency.pressure >= tendency.threshold
        })
        .map(|tendency| &tendency.id)
        .collect();

    let mut scored: Vec<SceneScore> = Vec::with_capacity(live.len());
    for (index, scene) in live.iter().enumerate() {
        let signals = signals_for(
            request.projection,
            scene,
            index,
            &response,
            &appearances,
            &erupting,
            current_scene,
            request.balance_window,
            &mut warnings,
        );
        scored.push(SceneScore {
            scene: scene.id.clone(),
            summary: scene.summary.clone(),
            score: request.weights.score(&signals),
            probability: 0.0,
            signals,
        });
    }

    let scores: Vec<f64> = scored.iter().map(|entry| entry.score).collect();
    let probabilities = director::probabilities(&scores, request.temperature);
    for (entry, probability) in scored.iter_mut().zip(probabilities) {
        entry.probability = probability;
    }

    // ---- 抽取
    let decision_key = scene_select_key(request.ctx.narrative_order);
    let dice = Dice::new(request.settings.seed);
    let salt = if request.salt.is_empty() {
        NO_SALT
    } else {
        request.salt.as_str()
    };
    let die = dice.roll_salted(&decision_key, salt);
    let picked = director::select_index(&scores, request.temperature, die);

    let mut chosen = None;
    let mut chosen_proposal = None;
    let mut road_not_taken = Vec::new();
    for (index, entry) in scored.into_iter().enumerate() {
        if Some(index) == picked {
            chosen = Some(entry);
            chosen_proposal = Some(live[index].clone());
        } else {
            road_not_taken.push(entry);
        }
    }

    Ok(SceneChoice {
        chosen,
        chosen_proposal,
        road_not_taken,
        vetoed,
        judgments,
        decision_key: Some(decision_key),
        die: Some(die),
        warnings,
    })
}

// ---------------------------------------------------------------- 内部

#[allow(clippy::too_many_arguments)]
fn signals_for(
    projection: &Projection,
    scene: &SceneProposal,
    index: usize,
    response: &JudgeResponse,
    appearances: &BTreeMap<SubjectId, u64>,
    erupting: &BTreeSet<&TendencyId>,
    current_scene: u64,
    window: u64,
    warnings: &mut Vec<String>,
) -> SceneSignals {
    let threads: Vec<&Thread> = scene
        .threads
        .iter()
        .filter_map(|id| projection.threads.get(id))
        .collect();

    let tendency_bonus = match scene
        .erupts
        .iter()
        .filter(|id| erupting.contains(*id))
        .count()
    {
        0 if scene.erupts.is_empty() => 0.0,
        0 => {
            warnings.push(format!(
                "场景 {} 声明要让趋势爆发，但它们都还没到爆发阈值",
                scene.id
            ));
            0.0
        }
        _ => 1.0,
    };

    // 未受保护的故事线：过早收束只是扣分，不否决（受保护的已在上面被拿掉）。
    let premature_resolution = scene
        .resolves
        .iter()
        .map(|thread| {
            let title = projection
                .threads
                .get(thread)
                .map(|thread| thread.title.clone())
                .unwrap_or_else(|| thread.as_str().to_owned());
            normalized(response, index, &format!("resolves.{title}")).unwrap_or(0.0)
        })
        .fold(0.0_f64, f64::max);

    SceneSignals {
        semantic_fit: normalized(response, index, "fit").unwrap_or(0.0),
        tension_gain: normalized(response, index, "tension").unwrap_or(0.0),
        thread_pressure: director::thread_pressure(threads.iter().copied()),
        overdue_bonus: threads
            .iter()
            .map(|thread| thread.overdue_bonus(current_scene))
            .fold(0.0_f64, f64::max),
        character_balance: director::character_balance(&scene.focus, appearances, window),
        tendency_bonus,
        repetition: normalized(response, index, "repetitive").unwrap_or(0.0),
        premature_resolution,
    }
}

/// `scene_{index}.{suffix}` 的归一化数值。
fn normalized(response: &JudgeResponse, index: usize, suffix: &str) -> Option<f64> {
    response
        .answers
        .get(&format!("scene_{index}.{suffix}"))
        .and_then(JudgmentOutput::as_normalized)
}

/// 最近几个场景的目标，给 `q.scene.repetitive` 做参照。
fn recent_goals(projection: &Projection, limit: usize) -> String {
    if limit == 0 || projection.scenes.is_empty() {
        return "（还没有最近的场景）".to_owned();
    }
    let mut scenes: Vec<&if_domain::narrative::Scene> = projection.scenes.values().collect();
    scenes.sort_by_key(|scene| scene.index);
    let start = scenes.len().saturating_sub(limit);
    let joined = scenes[start..]
        .iter()
        .map(|scene| scene.plan.goal.clone())
        .filter(|goal| !goal.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    if joined.is_empty() {
        "（还没有最近的场景）".to_owned()
    } else {
        joined
    }
}

/// 每个主体在**最近 `window` 个已发生的场景**里出场了几次（docs/06 §6 的 `balance`）。
///
/// 出场与否看场景计划的 `present`——那是引擎认可的「物理在场」（docs/02 §10.1），
/// 比模型的措辞可靠。
fn recent_appearances(projection: &Projection, window: u64) -> BTreeMap<SubjectId, u64> {
    let mut counts: BTreeMap<SubjectId, u64> = BTreeMap::new();
    if window == 0 {
        return counts;
    }
    let mut scenes: Vec<&if_domain::narrative::Scene> = projection.scenes.values().collect();
    scenes.sort_by_key(|scene| scene.index);
    let start = scenes.len().saturating_sub(window as usize);
    for scene in &scenes[start..] {
        for subject in &scene.plan.present {
            *counts.entry(subject.clone()).or_insert(0) += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests;
