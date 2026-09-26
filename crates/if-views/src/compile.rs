//! 视图编译器：投影 → 某个消费方有资格看到的状态。
//!
//! 编译步骤按 docs/08 §3：选焦点 → 扩展相关子图 → 激活设定条目（由调用方传入，
//! `if-lore` 负责）→ 补充历史 → 可见性过滤 → 预算裁剪 → 确定性序列化。
//!
//! 两条不可退让的性质：
//!
//! 1. **缺席即不可见。** 角色视图没给持有者时得到的是空视图，不是全知视图。
//! 2. **同一输入同一指纹。** 状态序列化走 `serde_json` 的排序 Map，键序固定；
//!    指纹由 [`crate::hash::view_hash`] 给出，判定记录靠它追溯（docs/07 §5）。

use if_domain::id::{PropositionId, SubjectId, USER_HOLDER};
use if_domain::narrative::{LoreEntry, LoreSection, RevealTarget, ScenePlan, TendencyStatus};
use if_domain::projection::Projection;
use if_domain::turn::{ViewKind, ViewRef};
use if_domain::value::WorldTime;
use if_judge::CompiledView;
use serde::Serialize;

use crate::state::{
    BeatLine, BeliefLine, DirectorLine, FactLine, KnowledgeBoundary, LoreLine, RuleLine,
    ScenePlanLine, SubjectLine, TendencyLine, ThreadLine, ViewState, DEFAULT_VIEW_BUDGET,
};
use crate::visibility;

/// 一次视图编译的请求。多数字段只对特定视图有意义，默认值即「不提供」。
#[derive(Debug, Clone)]
pub struct ViewRequest {
    pub kind: ViewKind,
    /// 角色视图与叙事视图的持有者。玩家视图用 [`USER_HOLDER`]。
    pub holder: Option<SubjectId>,
    pub scene_index: u64,
    /// 任务代号（`T-impact` 等），写进状态供判定者判断语境。
    pub task: String,
    pub budget: usize,
    pub user_input: Option<String>,
    pub scene_plan: Option<ScenePlan>,
    /// 在场主体。检查视图与叙事视图的认知边界据此收敛。
    pub present: Vec<SubjectId>,
    pub focus: Vec<SubjectId>,
    pub reveal_allowed: Vec<PropositionId>,
    /// 待检查的节拍原文。只有检查视图用。
    pub beat_text: Option<String>,
    /// 已激活的设定条目。激活由 `if-lore` 负责，编译器只负责过滤与呈现。
    pub lore: Vec<LoreEntry>,
    /// 补进视图的最近节拍数【docs/08 §3.4 初始值 6】。
    pub recent_beats: usize,
}

impl ViewRequest {
    pub fn new(kind: ViewKind) -> Self {
        Self {
            kind,
            holder: None,
            scene_index: 0,
            task: String::new(),
            budget: DEFAULT_VIEW_BUDGET,
            user_input: None,
            scene_plan: None,
            present: Vec::new(),
            focus: Vec::new(),
            reveal_allowed: Vec::new(),
            beat_text: None,
            lore: Vec::new(),
            recent_beats: 6,
        }
    }

    pub fn holder(mut self, holder: impl Into<SubjectId>) -> Self {
        self.holder = Some(holder.into());
        self
    }

    pub fn task(mut self, task: impl Into<String>) -> Self {
        self.task = task.into();
        self
    }

    pub fn scene_index(mut self, scene_index: u64) -> Self {
        self.scene_index = scene_index;
        self
    }

    pub fn budget(mut self, budget: usize) -> Self {
        self.budget = budget;
        self
    }

    pub fn user_input(mut self, input: impl Into<String>) -> Self {
        self.user_input = Some(input.into());
        self
    }

    pub fn scene_plan(mut self, plan: ScenePlan) -> Self {
        self.present = plan.present.clone();
        self.focus = plan.focus.clone();
        self.reveal_allowed = plan
            .reveal_allowed
            .iter()
            .filter_map(|target| match target {
                RevealTarget::Fact { prop } => Some(prop.clone()),
                RevealTarget::Lore { .. } => None,
            })
            .collect();
        self.scene_plan = Some(plan);
        self
    }

    pub fn beat_text(mut self, text: impl Into<String>) -> Self {
        self.beat_text = Some(text.into());
        self
    }

    pub fn lore(mut self, lore: Vec<LoreEntry>) -> Self {
        self.lore = lore;
        self
    }

    /// 只指定在场主体，不带完整场景计划。检查视图与解析前预检会这么用。
    pub fn present_hint(mut self, present: &[&str]) -> Self {
        self.present = present.iter().map(|s| SubjectId::new(*s)).collect();
        self
    }
}

/// 编译一个视图。
pub fn compile(projection: &Projection, request: &ViewRequest) -> CompiledView {
    let mut state = ViewState::new(kind_name(request.kind), projection.world_time.to_string(), request.scene_index)
        .holder(request.holder.as_ref())
        .task(request.task.clone())
        .narrative_order(projection.narrative_order);

    match request.kind {
        ViewKind::God | ViewKind::Director => {
            fill_omniscient(&mut state, projection, request);
            if request.kind == ViewKind::Director {
                state.director = Some(director_signals(projection));
            }
        }
        ViewKind::Pov => fill_pov(&mut state, projection, request),
        ViewKind::Narration => fill_narration(&mut state, projection, request),
        ViewKind::Check => fill_check(&mut state, projection, request),
        ViewKind::Parse => fill_parse(&mut state, projection, request),
        ViewKind::Player => fill_player(&mut state, projection, request),
        ViewKind::Creation => fill_omniscient(&mut state, projection, request),
    }

    state.trim_to_budget(request.budget);
    let value = serde_json::to_value(&state).unwrap_or(serde_json::Value::Null);
    let hash = crate::hash::view_hash(&value);
    CompiledView {
        meta: ViewRef {
            kind: request.kind,
            holder: request.holder.clone(),
            hash,
        },
        state: value,
    }
}

fn kind_name(kind: ViewKind) -> &'static str {
    match kind {
        ViewKind::Parse => "parse",
        ViewKind::God => "god",
        ViewKind::Director => "director",
        ViewKind::Pov => "pov",
        ViewKind::Narration => "narration",
        ViewKind::Check => "check",
        ViewKind::Player => "player",
        ViewKind::Creation => "creation",
    }
}

// ---------------------------------------------------------------- 各视图的填充

/// 上帝 / 导演 / 创作视图：全部事实，包含秘密。
fn fill_omniscient(state: &mut ViewState, projection: &Projection, request: &ViewRequest) {
    state.subjects = subject_lines(projection);
    state.facts = fact_lines(projection, request, None);
    state.beliefs = belief_lines(projection, None);
    state.rules = rule_lines(projection, &request.focus, None);
    state.threads = thread_lines(projection, None);
    state.tendencies = tendency_lines(projection, None);
    state.lore = lore_lines(&request.lore, None);
    state.recent_beats = beat_lines(projection, request.recent_beats);
}

/// 角色视图：该角色能感知的一切，且**只有**这些。
fn fill_pov(state: &mut ViewState, projection: &Projection, request: &ViewRequest) {
    let Some(holder) = request.holder.as_ref() else {
        // 没有持有者就没有角色视图。空视图是唯一安全的降级。
        return;
    };
    state.subjects = subject_lines(projection);
    state.facts = fact_lines(projection, request, Some(holder));
    state.beliefs = belief_lines(projection, Some(holder));
    let focus = vec![holder.clone()];
    state.rules = rule_lines(projection, &focus, Some(holder));
    state.threads = thread_lines(projection, Some(holder));
    state.tendencies = tendency_lines(projection, Some(holder));
    state.lore = lore_lines(&request.lore, Some(holder));
}

/// 叙事视图：场景计划 + 在场角色的认知边界摘要 + 玩家可见的事实。
///
/// **不带 `unapproved_secrets`**——叙述者知道谁不知道什么就够了，不需要知道秘密本身
/// （docs/04 §2.1）。
fn fill_narration(state: &mut ViewState, projection: &Projection, request: &ViewRequest) {
    state.facts = fact_lines(projection, request, None);
    state.rules = rule_lines(projection, &request.focus, None);
    state.scene_plan = request.scene_plan.as_ref().map(scene_plan_line);
    let relevant = relevant_props(&state.facts);
    state.knowledge_boundaries =
        knowledge_boundaries(projection, &request.present, &relevant, request.scene_index);
    state.reveal_allowed = reveal_keys(projection, request);
    // 叙事视图只带文风段条目：文风是给写作用的，设定条目走世界观渠道。
    let style_lore: Vec<LoreEntry> = request
        .lore
        .iter()
        .filter(|entry| entry.section == LoreSection::Style)
        .cloned()
        .collect();
    state.lore = lore_lines(&style_lore, None);
    state.recent_beats = beat_lines(projection, request.recent_beats);
}

/// 检查视图：节拍原文 + 约束 + 相关事实 + 认知边界 + 未批准揭示的秘密。
fn fill_check(state: &mut ViewState, projection: &Projection, request: &ViewRequest) {
    state.beat_text = request.beat_text.clone();
    state.facts = fact_lines(projection, request, None);
    state.rules = rule_lines(projection, &request.focus, None);
    state.scene_plan = request.scene_plan.as_ref().map(scene_plan_line);
    let relevant = relevant_props(&state.facts);
    state.knowledge_boundaries =
        knowledge_boundaries(projection, &request.present, &relevant, request.scene_index);
    state.reveal_allowed = reveal_keys(projection, request);
    // 秘密清单只进检查视图。
    state.unapproved_secrets = visibility::secret_facts(projection, request.scene_index)
        .into_iter()
        .filter(|prop| !request.reveal_allowed.contains(prop))
        .map(|prop| prop.as_str().to_owned())
        .collect();
}

/// 解析视图：用户输入 + 玩家可见的世界切片。
fn fill_parse(state: &mut ViewState, projection: &Projection, request: &ViewRequest) {
    state.user_input = request.user_input.clone();
    state.facts = fact_lines(projection, request, None);
    state.rules = rule_lines(projection, &request.focus, None);
    state.threads = thread_lines(projection, None);
    state.lore = lore_lines(&request.lore, None);
}

/// 玩家视图：玩家认知投影。
fn fill_player(state: &mut ViewState, projection: &Projection, request: &ViewRequest) {
    let holder = SubjectId::new(USER_HOLDER);
    state.facts = fact_lines(projection, request, Some(&holder));
    state.beliefs = belief_lines(projection, Some(&holder));
    state.threads = thread_lines(projection, None);
    state.lore = lore_lines(&request.lore, None);
    state.recent_beats = beat_lines(projection, request.recent_beats);
}

// ---------------------------------------------------------------- 分节构造

/// 主体摘要。
///
/// 不按持有者裁剪主体清单：一个角色认不认得另一个人，要靠信念层而不是主体表来判定，
/// v1 不做熟人图。真正的隔离发生在事实层——`facts` 已经按可见性过滤过了。
fn subject_lines(projection: &Projection) -> Vec<SubjectLine> {
    projection
        .subjects
        .values()
        .map(|subject| SubjectLine {
            id: subject.id.as_str().to_owned(),
            kind: tag(&subject.kind),
            name: subject.name.clone(),
            tier: tag(&subject.tier),
            profile: subject.profile.clone(),
            aliases: subject.aliases.clone(),
        })
        .collect()
}

fn fact_lines(
    projection: &Projection,
    request: &ViewRequest,
    holder: Option<&SubjectId>,
) -> Vec<FactLine> {
    let mut lines: Vec<FactLine> = projection
        .facts
        .iter()
        .filter_map(|(prop_id, fact)| {
            let proposition = projection.propositions.get(prop_id)?;
            if !fact.is_valid_at(projection.world_time) {
                return None;
            }
            if !visibility::fact_is_visible(
                request.kind,
                holder,
                projection,
                proposition,
                fact,
                request.scene_index,
            ) {
                return None;
            }
            Some(FactLine {
                prop: prop_id.as_str().to_owned(),
                key: proposition.key.clone(),
                text: proposition.text.clone(),
                value: fact.value.to_string(),
                lock: fact.lock.to_string(),
                lock_rank: fact.lock.rank(),
                internal: proposition.internal,
                subjects: proposition
                    .subjects
                    .iter()
                    .map(|s| s.as_str().to_owned())
                    .collect(),
            })
        })
        .collect();
    lines.sort_by(|a, b| a.key.cmp(&b.key));
    lines
}

fn belief_lines(
    projection: &Projection,
    holder: Option<&SubjectId>,
) -> Vec<BeliefLine> {
    let mut lines: Vec<BeliefLine> = projection
        .beliefs
        .values()
        .filter(|belief| holder.map_or(true, |h| &belief.holder == h))
        .map(|belief| BeliefLine {
            holder: belief.holder.as_str().to_owned(),
            prop: belief.prop.as_str().to_owned(),
            text: projection
                .propositions
                .get(&belief.prop)
                .map(|p| p.text.clone())
                .unwrap_or_default(),
            value: belief.value.to_string(),
            belief: belief.belief,
            certainty: belief.certainty,
        })
        .collect();
    lines.sort_by(|a, b| (&a.holder, &a.prop).cmp(&(&b.holder, &b.prop)));
    lines
}

fn rule_lines(
    projection: &Projection,
    focus: &[SubjectId],
    holder: Option<&SubjectId>,
) -> Vec<RuleLine> {
    let mut lines: Vec<RuleLine> = projection
        .rules
        .values()
        .filter(|rule| rule.is_active_at(projection.world_time))
        .filter(|rule| match holder {
            Some(subject) => rule.applies_to(subject),
            None => focus.is_empty() || focus.iter().any(|s| rule.applies_to(s)),
        })
        .map(|rule| RuleLine {
            id: rule.id.as_str().to_owned(),
            text: rule.text.clone(),
            lock: rule.lock.to_string(),
            constraints: rule.constraints.clone(),
            boundaries: rule.boundaries.clone(),
        })
        .collect();
    lines.sort_by(|a, b| a.id.cmp(&b.id));
    lines
}

fn thread_lines(
    projection: &Projection,
    holder: Option<&SubjectId>,
) -> Vec<ThreadLine> {
    let mut lines: Vec<ThreadLine> = projection
        .threads
        .values()
        .filter(|thread| match holder {
            Some(subject) => thread.subjects.iter().any(|s| s == subject),
            None => true,
        })
        .map(|thread| ThreadLine {
            id: thread.id.as_str().to_owned(),
            title: thread.title.clone(),
            question: thread.question.clone(),
            stage: tag(&thread.stage),
            pressure: thread.pressure,
            protected: thread.protected_until.is_some(),
        })
        .collect();
    lines.sort_by(|a, b| a.id.cmp(&b.id));
    lines
}

fn tendency_lines(
    projection: &Projection,
    holder: Option<&SubjectId>,
) -> Vec<TendencyLine> {
    let mut lines: Vec<TendencyLine> = projection
        .tendencies
        .values()
        .filter(|tendency| tendency.status != TendencyStatus::Dissolved)
        .filter(|tendency| match holder {
            // 趋势默认对玩家与角色隐藏（docs/02 §8），角色视图里只给指向自己的那部分线索。
            Some(subject) => tendency.target.contains(subject.as_str()),
            None => true,
        })
        .map(|tendency| TendencyLine {
            id: tendency.id.as_str().to_owned(),
            text: tendency.text.clone(),
            pressure: tendency.pressure,
            status: tag(&tendency.status),
        })
        .collect();
    lines.sort_by(|a, b| a.id.cmp(&b.id));
    lines
}

fn lore_lines(active: &[LoreEntry], holder: Option<&SubjectId>) -> Vec<LoreLine> {
    let mut lines: Vec<LoreLine> = active
        .iter()
        .filter(|entry| match holder {
            // secret 条目只给知道它的人（docs/02 §9）。
            Some(subject) => {
                entry.visibility != if_domain::narrative::LoreVisibility::Secret
                    || entry.known_by.iter().any(|s| s == subject)
            }
            None => true,
        })
        .map(|entry| LoreLine {
            id: entry.id.as_str().to_owned(),
            title: entry.title.clone(),
            section: tag(&entry.section),
            content: entry.content.clone(),
        })
        .collect();
    lines.sort_by(|a, b| a.id.cmp(&b.id));
    lines
}

fn beat_lines(projection: &Projection, limit: usize) -> Vec<BeatLine> {
    if limit == 0 {
        return Vec::new();
    }
    let start = projection.beats.len().saturating_sub(limit);
    projection.beats[start..]
        .iter()
        .map(|beat| BeatLine {
            narrative_order: u64::from(beat.index),
            scene: beat.scene.as_str().to_owned(),
            text: beat.text.clone(),
        })
        .collect()
}

/// 在场角色的认知边界。`q.beat.knowledge_leak` 的检查范围（docs/04 §4）。
///
/// `relevant` 是本场景相关的命题 ID（通常是检查视图里出现的事实）。
/// 角色不知道其中哪一条，就是泄露能着力的地方；反过来，他知道的哪些条目允许被说出来，
/// 叙事视图据此收敛措辞。
fn knowledge_boundaries(
    projection: &Projection,
    present: &[SubjectId],
    relevant: &[PropositionId],
    scene_index: u64,
) -> Vec<KnowledgeBoundary> {
    present
        .iter()
        .map(|holder| {
            let known = visibility::knowledge_of(projection, holder, scene_index);
            let mut knows: Vec<String> = relevant
                .iter()
                .filter(|prop| known.contains(*prop))
                .map(|prop| {
                    projection
                        .propositions
                        .get(prop)
                        .map(|p| p.key.clone())
                        .unwrap_or_else(|| prop.as_str().to_owned())
                })
                .collect();
            let mut unaware: Vec<String> = relevant
                .iter()
                .filter(|prop| !known.contains(*prop))
                .map(|prop| {
                    projection
                        .propositions
                        .get(prop)
                        .map(|p| p.key.clone())
                        .unwrap_or_else(|| prop.as_str().to_owned())
                })
                .collect();
            knows.sort();
            unaware.sort();
            KnowledgeBoundary {
                holder: holder.as_str().to_owned(),
                name: projection
                    .subjects
                    .get(holder)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| holder.as_str().to_owned()),
                knows,
                unaware_of: unaware,
            }
        })
        .collect()
}

fn reveal_keys(projection: &Projection, request: &ViewRequest) -> Vec<String> {
    let mut keys: Vec<String> = request
        .reveal_allowed
        .iter()
        .map(|prop| {
            projection
                .propositions
                .get(prop)
                .map(|p| p.key.clone())
                .unwrap_or_else(|| prop.as_str().to_owned())
        })
        .collect();
    keys.sort();
    keys
}

/// 视图里出现过的命题 ID。认知边界以它为「本场景相关」的范围。
fn relevant_props(facts: &[FactLine]) -> Vec<PropositionId> {
    facts
        .iter()
        .map(|fact| PropositionId::new(fact.prop.as_str()))
        .collect()
}

fn scene_plan_line(plan: &ScenePlan) -> ScenePlanLine {
    ScenePlanLine {
        goal: plan.goal.clone(),
        pov: plan.pov.as_str().to_owned(),
        focus: plan.focus.iter().map(|s| s.as_str().to_owned()).collect(),
        present: plan.present.iter().map(|s| s.as_str().to_owned()).collect(),
        time_span: plan.time_span.clone(),
        required_beats: plan.required_beats.clone(),
        stop_condition: plan.stop_condition.clone(),
        forbidden_resolutions: plan.forbidden_resolutions.clone(),
        reveal_allowed: plan
            .reveal_allowed
            .iter()
            .map(|target| match target {
                RevealTarget::Fact { prop } => prop.as_str().to_owned(),
                RevealTarget::Lore { lore } => lore.as_str().to_owned(),
            })
            .collect(),
    }
}

fn director_signals(projection: &Projection) -> DirectorLine {
    let scene_count = projection.scenes.len() as u64;
    let overdue_threads = projection
        .threads
        .values()
        .filter(|thread| thread.overdue_bonus(scene_count) > 0.0)
        .map(|thread| format!("{}: {}", thread.id.as_str(), thread.title))
        .collect();
    let erupting_tendencies = projection
        .tendencies
        .values()
        .filter(|tendency| tendency.pressure >= tendency.threshold)
        .map(|tendency| format!("{}: {}", tendency.id.as_str(), tendency.text))
        .collect();
    DirectorLine {
        overdue_threads,
        absent_subjects: Vec::new(),
        erupting_tendencies,
        scene_count,
    }
}

/// 取枚举的线上名字（serde 的 `rename_all` 结果），与持久化词汇保持一致。
fn tag<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// 世界时间的呈现形式，供调用方拼任务提示。
pub fn now(at: WorldTime) -> String {
    at.to_string()
}

#[cfg(test)]
mod tests;
