//! 故事线、趋势、设定条目、场景与节拍（docs/02 §7–§10）。

use serde::{Deserialize, Serialize};

use crate::id::{
    BeatId, CandidateId, EventId, JudgmentId, LoreId, PropositionId, SceneId, SubjectId, ThreadId,
    TendencyId,
};
use crate::rule::Condition;
use crate::value::WorldTime;

/// 墙上时钟时间戳（Unix 毫秒）。与 [`WorldTime`] 不同，它记录的是**现实世界**的时刻，
/// 只用于审计（例如节拍是什么时候展示给玩家的）。
pub type Timestamp = i64;

// ---------------------------------------------------------------- 故事线

/// 故事线阶段（docs/02 §7）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStage {
    /// 埋下
    Seeded,
    /// 发展
    Developing,
    /// 升级
    Escalating,
    /// 高潮
    Climax,
    /// 已解决
    Resolved,
    /// 已搁置
    Abandoned,
}

impl ThreadStage {
    /// 线性主干上的位置；`Abandoned` 不在主干上，返回 `None`。
    pub const fn chain_index(self) -> Option<u8> {
        match self {
            ThreadStage::Seeded => Some(0),
            ThreadStage::Developing => Some(1),
            ThreadStage::Escalating => Some(2),
            ThreadStage::Climax => Some(3),
            ThreadStage::Resolved => Some(4),
            ThreadStage::Abandoned => None,
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, ThreadStage::Resolved | ThreadStage::Abandoned)
    }

    /// 只允许转到相邻阶段；跨阶段必须由用户 IF 触发（已确认，docs/02 §7）。
    pub fn can_step_to(self, next: ThreadStage) -> bool {
        if next == ThreadStage::Abandoned {
            return !self.is_terminal();
        }
        if self == ThreadStage::Abandoned {
            return false;
        }
        match (self.chain_index(), next.chain_index()) {
            (Some(a), Some(b)) => b.abs_diff(a) <= 1,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub title: String,
    /// 这条线要回答的问题，如「顾言会不会离开」。
    pub question: String,
    /// 代价与利害。
    #[serde(default)]
    pub stakes: String,
    #[serde(default)]
    pub subjects: Vec<SubjectId>,
    pub stage: ThreadStage,
    /// 到达该阶段之前不得解决。引擎据此注入 `forbidden_resolutions`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_until: Option<ThreadStage>,
    /// 0–1。
    pub pressure: f64,
    /// 上次推进时的场景序号。
    pub last_advanced: u64,
    /// 期望每隔多少个场景推进一次。
    pub cadence: f64,
}

impl Thread {
    /// 受保护的故事线要注入的禁止项（docs/02 §7、docs/04 §2.1）。
    /// 返回 `None` 表示这条线当前不受保护。
    pub fn forbidden_resolution(&self) -> Option<String> {
        let protected = self.protected_until.as_ref()?;
        if self.stage >= *protected {
            return None;
        }
        Some(format!(
            "故事线「{}」的核心问题（{}）不得在本场景得到最终回答",
            self.title, self.question
        ))
    }

    /// 逾期程度，用于导演评分的 `overdue_bonus`（docs/06 §6）：
    /// `clamp(距上次推进的场景数 / cadence − 1, 0, 1)`
    pub fn overdue_bonus(&self, current_scene: u64) -> f64 {
        if self.cadence <= 0.0 {
            return 0.0;
        }
        let gap = current_scene.saturating_sub(self.last_advanced) as f64;
        ((gap / self.cadence) - 1.0).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------- 趋势

/// 观察带与趋势的参数（docs/06 §5）【初始值】。
pub const TENDENCY_WATCH_THRESHOLD: f64 = 0.3;
pub const TENDENCY_ERUPT_THRESHOLD: f64 = 0.8;
pub const TENDENCY_DISSOLVE_THRESHOLD: f64 = 0.05;
pub const TENDENCY_DECAY_PER_SCENE: f64 = 0.95;
pub const TENDENCY_INITIAL_PRESSURE_FACTOR: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TendencyStatus {
    Latent,
    Erupted,
    Dissolved,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tendency {
    pub id: TendencyId,
    /// 如「顾言开始怀疑林夏」。
    pub text: String,
    /// 爆发后对应的命题或事件。
    #[serde(default)]
    pub target: String,
    /// 0–1。
    pub pressure: f64,
    pub threshold: f64,
    #[serde(default)]
    pub contributors: Vec<EventId>,
    pub status: TendencyStatus,
}

impl Tendency {
    /// 被掷骰否决、但概率落在观察带内时新建一条趋势（docs/06 §5）。
    pub fn from_failed_candidate(
        id: impl Into<TendencyId>,
        text: impl Into<String>,
        probability: f64,
        source: impl Into<EventId>,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            target: String::new(),
            pressure: probability * TENDENCY_INITIAL_PRESSURE_FACTOR,
            threshold: TENDENCY_ERUPT_THRESHOLD,
            contributors: vec![source.into()],
            status: TendencyStatus::Latent,
        }
    }

    /// 一个发生类候选是否够格转成趋势。
    pub fn qualifies_for_watch(probability: f64) -> bool {
        probability >= TENDENCY_WATCH_THRESHOLD
    }

    /// 按推动量调整压力。低于消散阈值即消散，达到阈值即爆发。
    pub fn push(&mut self, delta: f64) {
        if self.status != TendencyStatus::Latent {
            return;
        }
        self.pressure = (self.pressure + delta).clamp(0.0, 1.0);
        if self.pressure >= self.threshold {
            self.status = TendencyStatus::Erupted;
        } else if self.pressure < TENDENCY_DISSOLVE_THRESHOLD {
            self.status = TendencyStatus::Dissolved;
        }
    }

    /// 每个场景一次的衰减。
    pub fn decay(&mut self) {
        if self.status != TendencyStatus::Latent {
            return;
        }
        self.pressure *= TENDENCY_DECAY_PER_SCENE;
        if self.pressure < TENDENCY_DISSOLVE_THRESHOLD {
            self.status = TendencyStatus::Dissolved;
        }
    }

    /// 压力的模糊描述，供玩家视图与观测使用（docs/01 §13）——
    /// 不给出具体内容，只给一个量级。
    pub fn fuzzy_label(&self) -> &'static str {
        match self.pressure {
            p if p >= TENDENCY_ERUPT_THRESHOLD => "一触即发",
            p if p >= 0.55 => "暗流涌动",
            p if p >= TENDENCY_WATCH_THRESHOLD => "略有起伏",
            _ => "几不可察",
        }
    }
}

/// `q.tendency.push` 的等级 → 推动量（docs/06 §5、§7）。
/// 等级顺序：削弱 / 无影响 / 轻微推动 / 明显推动 / 强烈推动。
pub const TENDENCY_PUSH_DELTAS: [f64; 5] = [-0.1, 0.0, 0.05, 0.15, 0.3];

/// 把 Score 原语的等级索引（0 基）换算成推动量。
///
/// 注意 Score 返回的是**连续分数**，调用方应先按 `score / (等级数 − 1)` 归一化并取整到等级，
/// 或者直接用 `legend` 对齐标签后取索引（见 docs/06 §7）。
pub fn tendency_push_delta(level_index: usize) -> f64 {
    TENDENCY_PUSH_DELTAS
        .get(level_index)
        .copied()
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------- 设定条目

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecondaryLogic {
    AndAny,
    AndAll,
    NotAny,
    NotAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoreSection {
    World,
    Character,
    Scene,
    Style,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoreVisibility {
    Public,
    Secret,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoreStatus {
    Active,
    Superseded,
}

/// 设定条目（世界书，docs/02 §9）。静态背景文本，不是可变状态。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoreEntry {
    pub id: LoreId,
    pub title: String,
    pub content: String,
    /// 触发关键词。
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub secondary_keys: Vec<String>,
    pub logic: SecondaryLogic,
    /// 关联的主体：主体处于焦点时直接激活（IF 新增，比关键词可靠）。
    #[serde(default)]
    pub subjects: Vec<SubjectId>,
    /// 条件激活，引用事实。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    #[serde(default)]
    pub constant: bool,
    #[serde(default)]
    pub order: i32,
    pub section: LoreSection,
    pub visibility: LoreVisibility,
    /// secret 条目：哪些主体知道。
    #[serde(default)]
    pub known_by: Vec<SubjectId>,
    /// 概率激活用命运骰子实现（docs/08 §5），对应酒馆的 `probability`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<f64>,
    pub status: LoreStatus,
    pub source: EventId,
}

impl LoreEntry {
    /// 是否属于某个消费方可见的范围。叙事视图只放 public 条目
    /// （secret 条目需要 `reveal_allowed` 覆盖，见 docs/08 §5）。
    pub fn visible_in_narrative(&self) -> bool {
        self.visibility == LoreVisibility::Public && self.status == LoreStatus::Active
    }

    /// secret 条目对某个角色是否可见。
    pub fn visible_to(&self, subject: &SubjectId) -> bool {
        match self.visibility {
            LoreVisibility::Public => true,
            LoreVisibility::Secret => self.known_by.iter().any(|s| s == subject),
        }
    }
}

// ---------------------------------------------------------------- 场景与节拍

/// 场景计划（docs/02 §10）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenePlan {
    pub goal: String,
    /// 视角人物。
    pub pov: SubjectId,
    /// 焦点主体：本场景要处理谁。
    #[serde(default)]
    pub focus: Vec<SubjectId>,
    /// 在场主体（已确认）：认知传播、视角隔离、节拍泄密检查的依据。
    /// 与 `focus` 不是一回事——门外偷看的人是 present 但未必是 focus。
    #[serde(default)]
    pub present: Vec<SubjectId>,
    /// 世界时间跨度。
    #[serde(default)]
    pub time_span: String,
    #[serde(default)]
    pub required_beats: Vec<String>,
    pub stop_condition: String,
    /// 引擎按受保护的故事线注入，加上计划自带的（docs/04 §2.1）。
    #[serde(default)]
    pub forbidden_resolutions: Vec<String>,
    /// 本场景允许向读者揭示的内容。
    #[serde(default)]
    pub reveal_allowed: Vec<RevealTarget>,
    /// 本场景要体现的预演变化。
    #[serde(default)]
    pub proposed_changes: Vec<CandidateId>,
}

/// 允许揭示的对象。事实与设定条目都可能需要被揭示。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RevealTarget {
    Fact { prop: PropositionId },
    Lore { lore: LoreId },
}

impl ScenePlan {
    /// 计划的两条硬校验（引擎侧，不依赖 Jev）：
    /// 视角人物必须在场；`present` 不能为空——没有在场者就没有认知传播的对象。
    pub fn validate(&self) -> Result<(), ScenePlanError> {
        if self.present.is_empty() {
            return Err(ScenePlanError::EmptyPresent);
        }
        if !self.present.contains(&self.pov) {
            return Err(ScenePlanError::PovNotPresent);
        }
        Ok(())
    }

    pub fn allows_reveal(&self, target: &RevealTarget) -> bool {
        self.reveal_allowed.contains(target)
    }
}

/// 场景计划的引擎侧校验失败。手写 `Display`，让 `if-domain` 只依赖 serde。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScenePlanError {
    EmptyPresent,
    PovNotPresent,
}

impl std::fmt::Display for ScenePlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ScenePlanError::EmptyPresent => "场景计划没有列出任何在场主体",
            ScenePlanError::PovNotPresent => "视角人物不在在场主体列表里",
        })
    }
}

impl std::error::Error for ScenePlanError {}

/// 场景。`index` 是场景序号，L1 保护期按它计算（docs/01 §7）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: SceneId,
    pub index: u64,
    pub plan: ScenePlan,
    pub started_at: WorldTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<WorldTime>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Beat {
    pub id: BeatId,
    pub scene: SceneId,
    pub index: u32,
    pub text: String,
    /// 对应 `required_beats` 中的哪一条。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_beat: Option<usize>,
    /// 放行检查的判定记录。
    #[serde(default)]
    pub judgments: Vec<JudgmentId>,
    /// 展示给玩家的现实时刻。写入 `beat_displayed` 时填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displayed_at: Option<Timestamp>,
}

/// 每个场景最多多少个节拍（docs/04 §4）【初始值 6】。
pub const MAX_BEATS_PER_SCENE: u32 = 6;

/// 同一节拍最多重试几次（docs/04 §4）【初始值 2】。
pub const MAX_BEAT_RETRIES: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(stage: ThreadStage, protected: Option<ThreadStage>) -> Thread {
        Thread {
            id: ThreadId::new("thr_1"),
            title: "顾言会不会离开".into(),
            question: "顾言最终会离开吗".into(),
            stakes: "离开意味着永别".into(),
            subjects: vec![],
            stage,
            protected_until: protected,
            pressure: 0.5,
            last_advanced: 0,
            cadence: 3.0,
        }
    }

    #[test]
    fn thread_stage_only_steps_to_adjacent() {
        assert!(ThreadStage::Seeded.can_step_to(ThreadStage::Developing));
        assert!(!ThreadStage::Seeded.can_step_to(ThreadStage::Escalating));
        assert!(ThreadStage::Developing.can_step_to(ThreadStage::Seeded));
        assert!(!ThreadStage::Seeded.can_step_to(ThreadStage::Resolved));
        assert!(ThreadStage::Developing.can_step_to(ThreadStage::Abandoned));
        assert!(!ThreadStage::Resolved.can_step_to(ThreadStage::Abandoned));
    }

    #[test]
    fn protected_thread_emits_forbidden_resolution() {
        let t = thread(ThreadStage::Developing, Some(ThreadStage::Climax));
        assert!(t.forbidden_resolution().is_some());
        // 已经到达保护阶段之后就不再禁止
        let done = thread(ThreadStage::Climax, Some(ThreadStage::Climax));
        assert!(done.forbidden_resolution().is_none());
    }

    #[test]
    fn overdue_bonus_is_clamped() {
        let mut t = thread(ThreadStage::Developing, None);
        t.last_advanced = 0;
        t.cadence = 3.0;
        assert_eq!(t.overdue_bonus(0), 0.0); // 刚到 1 倍，未逾期
        assert!((t.overdue_bonus(3) - 0.0).abs() < 1e-9); // 恰好 1 倍
        assert!((t.overdue_bonus(6) - 1.0).abs() < 1e-9); // 2 倍 → 满额
        assert_eq!(t.overdue_bonus(999), 1.0); // 封顶
    }

    #[test]
    fn tendency_erupts_and_dissolves() {
        let mut t = Tendency::from_failed_candidate("tnd_1", "顾言开始怀疑林夏", 0.5, "evt_1");
        assert!((t.pressure - 0.25).abs() < 1e-9); // p × 0.5
        t.push(0.3);
        t.push(0.3);
        assert!(t.pressure >= TENDENCY_ERUPT_THRESHOLD);
        assert_eq!(t.status, TendencyStatus::Erupted);
        // 爆发后不再接受推动
        let before = t.pressure;
        t.push(0.3);
        assert_eq!(t.pressure, before);

        let mut dying = Tendency::from_failed_candidate("tnd_2", "x", 0.2, "evt_1");
        dying.push(-0.5);
        assert_eq!(dying.status, TendencyStatus::Dissolved);
    }

    #[test]
    fn watch_band_matches_documented_threshold() {
        assert!(!Tendency::qualifies_for_watch(0.29));
        assert!(Tendency::qualifies_for_watch(0.3));
    }

    #[test]
    fn push_deltas_match_documented_table() {
        assert_eq!(tendency_push_delta(0), -0.1);
        assert_eq!(tendency_push_delta(1), 0.0);
        assert_eq!(tendency_push_delta(2), 0.05);
        assert_eq!(tendency_push_delta(3), 0.15);
        assert_eq!(tendency_push_delta(4), 0.3);
        // 越界不 panic
        assert_eq!(tendency_push_delta(9), 0.0);
    }

    #[test]
    fn fuzzy_label_never_leaks_content() {
        let mut t = Tendency::from_failed_candidate("tnd_1", "顾言开始怀疑林夏", 0.6, "evt_1");
        let label = t.fuzzy_label();
        assert!(!label.contains("顾言"));
        t.pressure = 0.9;
        assert_eq!(t.fuzzy_label(), "一触即发");
    }

    #[test]
    fn scene_plan_requires_pov_present() {
        let mut plan = ScenePlan {
            goal: "表现林夏的犹豫".into(),
            pov: SubjectId::new("c_lin"),
            focus: vec![SubjectId::new("c_gu")],
            present: vec![SubjectId::new("c_gu")],
            time_span: "当晚".into(),
            required_beats: vec![],
            stop_condition: "顾言开始怀疑".into(),
            forbidden_resolutions: vec![],
            reveal_allowed: vec![],
            proposed_changes: vec![],
        };
        assert_eq!(plan.validate(), Err(ScenePlanError::PovNotPresent));
        plan.present.push(SubjectId::new("c_lin"));
        assert!(plan.validate().is_ok());
        plan.present.clear();
        assert_eq!(plan.validate(), Err(ScenePlanError::EmptyPresent));
    }
}
