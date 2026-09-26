//! 回合记录、候选、判定与裁决（docs/03 §3、docs/07 §5）。
//!
//! 判定与裁决不写进事件，而是记在回合记录里，事件通过 `turn` 关联。
//! 这样事件保持精简，审计信息也完整保留。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::event::IfInjection;
use crate::id::{
    CandidateId, EventId, JudgmentId, PropositionId, SubjectId, TurnId, WorldLineId,
};
use crate::narrative::ScenePlan;
use crate::value::WorldTime;

/// 回合类型（docs/04 §1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnKind {
    If,
    Continue,
    Observe,
    Background,
    Creation,
}

// ---------------------------------------------------------------- 视图

/// 视图种类（docs/08 §1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Parse,
    God,
    Director,
    Pov,
    Narration,
    Check,
    Player,
    Creation,
}

/// 判定所依据的视图。`hash` 让判定可追溯、可复现（docs/07 §5）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewRef {
    pub kind: ViewKind,
    /// 视图持有者：角色视图是角色 ID，玩家视图是 `user`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<SubjectId>,
    pub hash: String,
}

// ---------------------------------------------------------------- 候选

/// 候选的形态（docs/07 §4.2）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum CandidateShape {
    /// 发生类：会不会发生。用带种子的抽样裁决。
    Occurs,
    /// 互斥组：几种互斥的结果中哪一个发生。整组只抽一次。
    Exclusive { options: Vec<String> },
}

/// 一个候选（docs/05 §5.2 `propose_candidate`）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: CandidateId,
    /// **稳定决策键**：跨世界线不变的语义标识，命运骰子按它取值（docs/03 §8）。
    ///
    /// 形如 `c_gu.notices.c_lin_abnormal`（状态候选，用被影响命题的键）或
    /// `c_lin.reaction_to_feeling`（互斥选择点）。规则见 [`Candidate::decision_key`]。
    ///
    /// 这个字段是「骰子 0.55，旧线 p = 0.41 → 未发生；新线 p = 0.72 → 发生」
    /// 能成立的前提：候选 ID 是回合内分配的，两条世界线上并不相同，不能拿它当键。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// 候选作用于哪个主体。世界事件可以没有主体。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SubjectId>,
    pub content: String,
    /// 内在或外在。对账规则按它决定能否只靠预演提交（docs/02 §11）。
    pub internal: bool,
    pub shape: CandidateShape,
    /// 依赖的其他候选。分层裁决按它做拓扑排序。
    #[serde(default)]
    pub depends_on: Vec<CandidateId>,
    /// 所依据的认知——判断时只能给判定者看这些。
    #[serde(default)]
    pub based_on: Vec<PropositionId>,
    /// 影响的命题。
    #[serde(default)]
    pub affects: Vec<PropositionId>,
}

impl Candidate {
    /// 稳定决策键（docs/03 §8）。优先用 LLM 给出的 `key`，没有则退到**首个被影响命题**
    /// 的规范键——状态候选的键就是它所影响的命题键（`<命题键>@<世界时间段>`）。
    ///
    /// 返回 `None` 表示这个候选没有跨世界线的稳定标识。引擎不该拿候选 ID 去顶替：
    /// 候选 ID 是回合内分配的，两条世界线上并不相同，用它当键会让「同一个决策
    /// 在两条线上用同一颗骰子」这条性质失效。
    pub fn decision_key<F>(&self, proposition_key: F) -> Option<String>
    where
        F: Fn(&PropositionId) -> Option<String>,
    {
        if let Some(key) = &self.key {
            return Some(key.clone());
        }
        let first = self.affects.first()?;
        proposition_key(first)
    }

    /// 互斥组携带的选项；不是互斥组时返回 `None`。
    pub fn options(&self) -> Option<&[String]> {
        match &self.shape {
            CandidateShape::Exclusive { options } => Some(options),
            CandidateShape::Occurs => None,
        }
    }
}

// ---------------------------------------------------------------- 判定

/// Jev 的原语（docs/07 §1）。实测枚举就是这三个。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Primitive {
    Noul,
    Choice,
    Score,
}

/// 判定的原始输出。字段名与实测的 Jev 响应一一对应（docs/14 §3）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JudgmentOutput {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        /// 用 `BTreeMap` 保证序列化键序固定（docs/12 §7）。
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        /// 0 基连续分数。等级数为 n 时区间是 `[0, n−1]`，
        /// 归一化要除以 `n − 1`（docs/06 §7）。
        score: f64,
        /// 索引 → 标签。
        legend: BTreeMap<String, String>,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
}

impl JudgmentOutput {
    pub const fn primitive(&self) -> Primitive {
        match self {
            JudgmentOutput::Noul { .. } => Primitive::Noul,
            JudgmentOutput::Choice { .. } => Primitive::Choice,
            JudgmentOutput::Score { .. } => Primitive::Score,
        }
    }

    /// 归一化到 `[0, 1]` 的数值。Noul 直接就是概率；Score 按等级数归一化。
    ///
    /// 返回 `None` 表示这个原语没有单一数值（Choice 要看分布）。
    pub fn as_normalized(&self) -> Option<f64> {
        match self {
            JudgmentOutput::Noul { noul } => Some(*noul),
            JudgmentOutput::Score { score, legend, .. } => {
                let levels = legend.len();
                if levels < 2 {
                    return None;
                }
                Some((score / (levels as f64 - 1.0)).clamp(0.0, 1.0))
            }
            JudgmentOutput::Choice { .. } => None,
        }
    }
}

/// 判定用量。`cost` 直接取自 Jev 的 `usage.cost`（docs/14 §5.4）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JudgmentUsage {
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// 一条判定记录（docs/07 §5）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Judgment {
    pub id: JudgmentId,
    pub turn: TurnId,
    /// 模板 ID@版本，如 `q.perception.notices@1`。
    pub template: String,
    /// 候选或问题的 ID。
    pub target: String,
    pub view: ViewRef,
    /// **响应回显的固定快照版本**，如 `typesafe/jev-1.13-20260917`。
    /// 不要记请求时的别名（docs/14 §7）。
    pub model: String,
    pub output: JudgmentOutput,
    pub usage: JudgmentUsage,
    pub latency_ms: u64,
}

// ---------------------------------------------------------------- 裁决

/// 裁决策略（docs/06 §1）。由问题的语义类别决定，不由它属于哪个领域决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionPolicy {
    /// 阈值：约束类与合规检查，不掷骰。
    Threshold,
    /// 带种子的抽样：发生类。
    SeededSample,
    /// 带种子的**分类**抽样：互斥类。整组只抽一次，选项按 ID 排序后累加分布
    /// （docs/06 §3）。与 [`ResolutionPolicy::SeededSample`] 的区别在于结果是
    /// 「选中了哪一个」而不是「是否发生」。
    SeededCategorical,
    /// 加权选择：导演调度。
    WeightedSelect,
    /// 数值输入：强度类，直接参与计算。
    Numeric,
}

/// 裁决结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolutionOutcome {
    /// 发生类 / 约束类：成立与否。
    Accepted { accepted: bool },
    /// 互斥类：选中了哪个选项。
    Selected { option: String },
    /// 数值类：算出来的值。
    Value { value: f64 },
}

/// 一次裁决（docs/03 §3）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resolution {
    /// 候选或问题的 ID。
    pub target: String,
    pub policy: ResolutionPolicy,
    /// 命运骰子的决策键（docs/03 §8）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_key: Option<String>,
    /// 骰子值 `u ∈ [0,1)`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub die: Option<f64>,
    pub outcome: ResolutionOutcome,
}

impl Resolution {
    /// 阈值裁决：不掷骰（docs/06 §1 硬规则：约束类的概率永远不拿去掷骰）。
    ///
    /// `passed` 是**放行与否**，由调用方按模板的方向算出来——阈值在不同模板上方向相反：
    /// `q.beat.violates_fact` 概率越高越可疑，`q.cand.in_character` 概率越高越可靠。
    /// 所以这里不接受概率去做比较：那等于把方向偷偷钉死成「越高越通过」，
    /// 而在合规检查上方向写反只会静默放过违规正文。
    /// 方向表与比较都在 `if-policy` 的 `threshold` 模块里。
    pub fn by_threshold(target: impl Into<String>, passed: bool) -> Self {
        Self {
            target: target.into(),
            policy: ResolutionPolicy::Threshold,
            decision_key: None,
            die: None,
            outcome: ResolutionOutcome::Accepted { accepted: passed },
        }
    }

    /// 带种子的抽样裁决：发生类。`u < p` 即发生（docs/06 §3）。
    pub fn by_die(
        target: impl Into<String>,
        probability: f64,
        decision_key: impl Into<String>,
        die: f64,
    ) -> Self {
        Self {
            target: target.into(),
            policy: ResolutionPolicy::SeededSample,
            decision_key: Some(decision_key.into()),
            die: Some(die),
            outcome: ResolutionOutcome::Accepted {
                accepted: die < probability,
            },
        }
    }

    /// 带种子的分类抽样：互斥类。整组共用一颗骰子，选中 `option`（docs/06 §3）。
    pub fn by_categorical(
        target: impl Into<String>,
        option: impl Into<String>,
        decision_key: impl Into<String>,
        die: f64,
    ) -> Self {
        Self {
            target: target.into(),
            policy: ResolutionPolicy::SeededCategorical,
            decision_key: Some(decision_key.into()),
            die: Some(die),
            outcome: ResolutionOutcome::Selected {
                option: option.into(),
            },
        }
    }

    /// 数值裁决：强度类与机制步。保留骰子值，便于复现（docs/06 §8）。
    pub fn by_value(
        target: impl Into<String>,
        value: f64,
        decision_key: impl Into<String>,
        die: f64,
    ) -> Self {
        Self {
            target: target.into(),
            policy: ResolutionPolicy::Numeric,
            decision_key: Some(decision_key.into()),
            die: Some(die),
            outcome: ResolutionOutcome::Value { value },
        }
    }

    /// 这次裁决是否放行。
    ///
    /// 只有 `Accepted` 有「放行 / 否决」这一说；`Selected` 与 `Value` 总是放行，
    /// 内容本身在别的字段里。调用方不该拿 `outcome == Accepted{false}` 以外的方式
    /// 判断否决——否则互斥组会被误判成「什么都没发生」。
    pub fn accepted(&self) -> bool {
        match &self.outcome {
            ResolutionOutcome::Accepted { accepted } => *accepted,
            ResolutionOutcome::Selected { .. } | ResolutionOutcome::Value { .. } => true,
        }
    }
}

// ---------------------------------------------------------------- 任务与回合

/// 一个 agent 任务的执行记录（docs/05 §4）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    /// 任务代号，如 `T-parse`。
    pub task: String,
    pub rounds: u32,
    #[serde(default)]
    pub calls: Vec<ToolCallRecord>,
    pub finish: TaskFinish,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool: String,
    /// 通过 / 被否决的理由。涉及秘密的否决不能说出秘密本身（docs/05 §6）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskFinish {
    Completed,
    /// 同类失败达到上限，交给回合流程兜底（docs/05 §6）。
    Fallback { reason: String },
    Cancelled,
}

/// 回合指标。Jev 的 token 与成本单独记，因为它便宜但不免费（docs/14 §5.4）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub latency_ms: u64,
    pub llm_tokens: u64,
    pub jev_tokens: u64,
    pub jev_cost_usd: f64,
}

/// 一份完整的回合记录（docs/03 §3）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    pub id: TurnId,
    pub line: WorldLineId,
    pub kind: TurnKind,
    /// 用户的原始输入（IF / 观测目标）。继续回合没有输入。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// IF 裁定卡。卡片先持久化为 pending，确认后才会产生 if_injected 事件。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ruling_card: Option<IfRulingCard>,
    /// 回合开始时所在的世界时间，供回溯判定「写入过去」。
    pub started_at: WorldTime,
    #[serde(default)]
    pub tasks: Vec<TaskRecord>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub judgments: Vec<Judgment>,
    #[serde(default)]
    pub resolutions: Vec<Resolution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_plan: Option<ScenePlan>,
    /// 本回合提交的事件 ID。
    #[serde(default)]
    pub committed: Vec<EventId>,
    #[serde(default)]
    pub metrics: TurnMetrics,
}

impl TurnRecord {
    pub fn new(id: impl Into<TurnId>, line: impl Into<WorldLineId>, kind: TurnKind, started_at: WorldTime) -> Self {
        Self {
            id: id.into(),
            line: line.into(),
            kind,
            input: None,
            ruling_card: None,
            started_at,
            tasks: Vec::new(),
            candidates: Vec::new(),
            judgments: Vec::new(),
            resolutions: Vec::new(),
            scene_plan: None,
            committed: Vec::new(),
            metrics: TurnMetrics::default(),
        }
    }

    /// 本回合的 Jev 总成本。
    pub fn jev_cost(&self) -> f64 {
        self.judgments.iter().map(|j| j.usage.cost_usd).sum()
    }

    pub fn judgment(&self, id: &JudgmentId) -> Option<&Judgment> {
        self.judgments.iter().find(|j| &j.id == id)
    }
}

/// 裁定卡生命周期。默认不自动确认（docs/01 §9.2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IfCardStatus {
    Pending,
    Confirmed,
    Cancelled,
}

/// IF 回合的最小可持久化裁定卡。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IfRulingCard {
    pub turn: TurnId,
    pub status: IfCardStatus,
    pub injection: IfInjection,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// 本地确定性预检发现的潜在冲突。语义模型尚未介入时只做提示，不自动覆盖事实。
    #[serde(default)]
    pub conflicts: Vec<IfConflict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_event: Option<EventId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IfConflict {
    pub event: EventId,
    pub existing_core: String,
    pub lock: crate::value::Lock,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IfConflictResolution {
    Reinterpret,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_normalization_uses_level_count() {
        let five = JudgmentOutput::Score {
            score: 3.73,
            legend: (0..5).map(|i| (i.to_string(), format!("L{i}"))).collect(),
            probabilities: BTreeMap::new(),
            confidence: 0.78,
        };
        let v = five.as_normalized().unwrap();
        assert!((v - 3.73 / 4.0).abs() < 1e-9);

        // 等级数变化时不能硬编码 5
        let ten = JudgmentOutput::Score {
            score: 4.5,
            legend: (0..10).map(|i| (i.to_string(), format!("L{i}"))).collect(),
            probabilities: BTreeMap::new(),
            confidence: 0.5,
        };
        assert!((ten.as_normalized().unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn noul_and_choice_are_not_interchangeable() {
        let n = JudgmentOutput::Noul { noul: 0.58 };
        assert_eq!(n.as_normalized(), Some(0.58));
        assert_eq!(n.primitive(), Primitive::Noul);

        let c = JudgmentOutput::Choice {
            choice: "go".into(),
            probabilities: BTreeMap::new(),
            confidence: 0.9,
        };
        assert_eq!(c.primitive(), Primitive::Choice);
        assert_eq!(c.as_normalized(), None);
    }

    #[test]
    fn seeded_sampling_uses_same_die_across_world_lines() {
        // 同一决策键、同一颗骰子：只有 p 越过 u 结果才翻转
        let old = Resolution::by_die("cand_1", 0.41, "c_gu.notices.c_lin_abnormal@D7", 0.55);
        let new = Resolution::by_die("cand_1", 0.72, "c_gu.notices.c_lin_abnormal@D7", 0.55);
        assert_eq!(old.die, new.die);
        assert_eq!(
            old.outcome,
            ResolutionOutcome::Accepted { accepted: false }
        );
        assert_eq!(new.outcome, ResolutionOutcome::Accepted { accepted: true });
    }

    #[test]
    fn threshold_resolution_has_no_die() {
        // p = 0.88 ≥ τ = 0.3 → 判为「违反锁定事实」→ 不放行
        let r = Resolution::by_threshold("q.beat.violates_fact", false);
        assert_eq!(r.policy, ResolutionPolicy::Threshold);
        assert!(r.die.is_none());
        assert!(r.decision_key.is_none());
        assert!(!r.accepted());
        assert_eq!(r.outcome, ResolutionOutcome::Accepted { accepted: false });
    }

    #[test]
    fn turn_record_sums_jev_cost() {
        let mut turn = TurnRecord::new("turn_1", "wl_main", TurnKind::If, WorldTime::EPOCH);
        for i in 0..3 {
            turn.judgments.push(Judgment {
                id: JudgmentId::new(format!("jdg_{i}")),
                turn: turn.id.clone(),
                template: "q.x@1".into(),
                target: "cand_1".into(),
                view: ViewRef {
                    kind: ViewKind::God,
                    holder: None,
                    hash: "b3:abc".into(),
                },
                model: "typesafe/jev-1.13-20260917".into(),
                output: JudgmentOutput::Noul { noul: 0.5 },
                usage: JudgmentUsage {
                    input_tokens: 300,
                    output_tokens: 20,
                    cost_usd: 0.0000126,
                },
                latency_ms: 600,
            });
        }
        assert!((turn.jev_cost() - 0.0000378).abs() < 1e-12);
    }

    #[test]
    fn candidate_decision_key_prefers_explicit_then_affected_proposition() {
        let mut candidate = Candidate {
            id: CandidateId::new("cand_1"),
            key: None,
            subject: Some(SubjectId::new("c_gu")),
            content: "顾言注意到林夏的异常".into(),
            internal: false,
            shape: CandidateShape::Occurs,
            depends_on: vec![],
            based_on: vec![],
            affects: vec![PropositionId::new("p_notices")],
        };
        // 没有显式键时，退到首个被影响命题的规范键
        assert_eq!(
            candidate.decision_key(|p| {
                (p.as_str() == "p_notices").then(|| "c_gu.notices.c_lin_abnormal".to_string())
            }),
            Some("c_gu.notices.c_lin_abnormal".to_string())
        );
        // 显式键优先
        candidate.key = Some("c_gu.notices@override".into());
        assert_eq!(
            candidate.decision_key(|_| Some("ignored".into())),
            Some("c_gu.notices@override".to_string())
        );
        // 既没有显式键、也没有可解析的被影响命题 → 没有稳定键，不能拿 ID 顶替
        candidate.key = None;
        candidate.affects.clear();
        assert_eq!(candidate.decision_key(|_| Some("x".into())), None);
    }

    #[test]
    fn categorical_resolution_selects_an_option() {
        let r = Resolution::by_categorical(
            "cand_007",
            "delay",
            "c_lin.reaction_to_feeling@D7",
            0.51,
        );
        assert_eq!(r.policy, ResolutionPolicy::SeededCategorical);
        assert_eq!(r.die, Some(0.51));
        assert_eq!(
            r.outcome,
            ResolutionOutcome::Selected {
                option: "delay".into()
            }
        );
        // 选中总是放行：否决只由 Accepted{false} 表示
        assert!(r.accepted());
        assert!(!Resolution::by_die("cand_1", 0.41, "k@D7", 0.55).accepted());
    }
}
