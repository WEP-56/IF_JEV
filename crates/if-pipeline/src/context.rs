//! 回合上下文：一次回合里所有阶段共用的那几样东西。
//!
//! 它解决的是一个很小但到处冒头的问题：**同一个回合里的每个阶段都要重新拼一遍
//! 「现在是第几个场景、谁是焦点、带哪些设定条目、用哪张阈值表」**。散着拼的结果是
//! 两处视图的 `scene_index` 差一、或者判定用的阈值表和裁决用的不是同一张——
//! 这类错不会报错，只会让 L1 保护期和严格度的行为对不上文档。
//!
//! 所以这里只做三件事：**装住输入**、**按它构造视图与策略**、**按它激活设定条目**。
//! 它不做任何判定，也不碰存储。
//!
//! [`TurnContext::lore_signals`] 与 [`activate_for_turn`] 是 [`crate::lore`] 的第一个
//! 真实调用方——世界书激活（docs/08 §5）到此才接进回合流程。

use if_domain::id::{SubjectId, TurnId, WorldLineId};
use if_domain::narrative::LoreEntry;
use if_domain::projection::Projection;
use if_domain::rule::WorldSettings;
use if_domain::turn::{TurnKind, ViewKind};
use if_domain::value::WorldTime;
use if_policy::{Dice, Policy, Strictness, Thresholds};
use if_views::{ViewRequest, DEFAULT_VIEW_BUDGET};

use crate::lore::{activate_lore, LoreSignals};

/// 默认补进视图的最近节拍数（docs/08 §3.4）【初始值 6】。
pub const DEFAULT_RECENT_BEATS: usize = 6;

/// 一次回合的共享上下文。
///
/// 字段都是 `pub`：调用方（`if-app` 的世界工作线程）本来就要从投影与设置里把它们凑出来，
/// 而这里有 builder 可链式设，也可以直接改。**回归正题的最小集**：
/// 场景序号、世界时间、焦点与在场、已激活的设定条目、阈值与严格度。
#[derive(Clone, Debug)]
pub struct TurnContext {
    pub turn: TurnId,
    pub line: WorldLineId,
    pub kind: TurnKind,
    /// 当前场景序号。L1 保护期、概率激活的决策键、导演的逾期度都按它算。
    pub scene_index: u64,
    /// 回合开始时的世界时间。回溯判定与事实的 `valid_from` 都用它。
    pub at: WorldTime,
    /// 叙述顺序游标：本节拍上屏时从它开始发号。
    pub narrative_order: u64,
    pub user_input: Option<String>,
    /// 焦点主体（docs/02 §10.1）。导演评分与视图扩展的起点。
    pub focus: Vec<SubjectId>,
    /// 在场主体。认知传播、视角隔离与 `q.beat.knowledge_leak` 的检查范围。
    pub present: Vec<SubjectId>,
    /// 本回合激活的设定条目（见 [`activate_for_turn`]）。
    pub lore: Vec<LoreEntry>,
    pub strictness: Strictness,
    pub thresholds: Thresholds,
    pub view_budget: usize,
    pub recent_beats: usize,
}

impl TurnContext {
    pub fn new(
        turn: impl Into<TurnId>,
        line: impl Into<WorldLineId>,
        kind: TurnKind,
        at: WorldTime,
        scene_index: u64,
    ) -> Self {
        Self {
            turn: turn.into(),
            line: line.into(),
            kind,
            scene_index,
            at,
            narrative_order: 0,
            user_input: None,
            focus: Vec::new(),
            present: Vec::new(),
            lore: Vec::new(),
            strictness: Strictness::default(),
            thresholds: Thresholds::default(),
            view_budget: DEFAULT_VIEW_BUDGET,
            recent_beats: DEFAULT_RECENT_BEATS,
        }
    }

    pub fn input(mut self, input: impl Into<String>) -> Self {
        self.user_input = Some(input.into());
        self
    }

    pub fn focus(mut self, subjects: impl IntoIterator<Item = SubjectId>) -> Self {
        self.focus = subjects.into_iter().collect();
        self
    }

    pub fn present(mut self, subjects: impl IntoIterator<Item = SubjectId>) -> Self {
        self.present = subjects.into_iter().collect();
        self
    }

    pub fn lore(mut self, lore: Vec<LoreEntry>) -> Self {
        self.lore = lore;
        self
    }

    pub fn strictness(mut self, strictness: impl Into<Strictness>) -> Self {
        self.strictness = strictness.into();
        self
    }

    pub fn thresholds(mut self, thresholds: Thresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    pub fn narrative_order(mut self, order: u64) -> Self {
        self.narrative_order = order;
        self
    }

    pub fn budget(mut self, budget: usize) -> Self {
        self.view_budget = budget;
        self
    }

    /// 按视图种类造一个已填好公共字段的请求。
    ///
    /// 各阶段只需要再补自己那份（`beat_text`、`scene_plan`、`holder`），
    /// 于是「焦点与在场是不是同一个集合」这类问题不会再出现两次答案。
    pub fn view(&self, kind: ViewKind) -> ViewRequest {
        let mut request = ViewRequest::new(kind);
        request.scene_index = self.scene_index;
        request.budget = self.view_budget;
        request.present = self.present.clone();
        request.focus = self.focus.clone();
        request.lore = self.lore.clone();
        request.recent_beats = self.recent_beats;
        request
    }

    /// 本回合的裁决策略。种子属于世界，不属于回合（docs/03 §8）。
    pub fn policy(&self, settings: &WorldSettings) -> Policy {
        Policy::new(settings, self.at, self.scene_index)
            .with_thresholds(self.thresholds.clone())
            .with_strictness(self.strictness)
    }

    /// 世界书激活所需的信号。`scan` 是关键词扫描范围（场景计划、最近节拍、用户输入）。
    pub fn lore_signals(&self, scan: impl Into<String>) -> LoreSignals {
        LoreSignals::new(self.scene_index)
            .focus(self.focus.clone())
            .scan(scan)
    }
}

/// 按回合上下文激活设定条目（docs/08 §5）。
///
/// 骰子从世界种子来，`salt` 留空——同一个场景重放必须得到同一批条目，
/// 这里换盐值等于让「重放」这件事不成立。
pub fn activate_for_turn(
    projection: &Projection,
    settings: &WorldSettings,
    context: &TurnContext,
    scan: &str,
) -> Vec<LoreEntry> {
    activate_lore(
        projection,
        &context.lore_signals(scan),
        &Dice::new(settings.seed),
        if_policy::NO_SALT,
    )
}

/// 主体的显示名。判定问题的措辞要用它——问题里写 `c_lin` 对判定者毫无意义。
pub fn subject_name(projection: &Projection, subject: &SubjectId) -> String {
    projection
        .subjects
        .get(subject)
        .map(|s| s.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| subject.as_str().to_owned())
}
