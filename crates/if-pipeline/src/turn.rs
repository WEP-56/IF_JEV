//! 回合驱动：把六个阶段串成 docs/04 §2 的那条链。
//!
//! ## 为什么是两段而不是一个大函数
//!
//! 真实流程里 **T-plan（第 10 步）夹在中间**：场景要先被选出来，才谈得上给它写计划。
//! 所以这里不假装能一步跑完，而是切成两段，中间那段交给调用方（未来的 agent 任务）：
//!
//! ```text
//! open()     ← 第 6–9 步：影响候选 → 分层裁决 → 场景候选 → 导演选择
//!   ↓ 调用方在这里跑 T-plan，把选中的场景变成 ScenePlan
//! resolve()  ← 第 10–13 步：引擎注入硬约束 → 逐节拍放行 → 对账 → 产出草稿
//! ```
//!
//! 一个 `run_if_turn` 式的大函数会逼调用方在选场景之前就把计划交上来——那不是编排，
//! 那是把顺序拧反。
//!
//! ## 引擎在什么时候说话
//!
//! 第 10 步的 [`inject_constraints`] 是**引擎直接写入、不由 LLM 决定**的那一份
//! （docs/04 §2.1）：受保护故事线的禁止项、在场集合的存在性、计划的合法形状。
//! 它们不参与评分，也不问 Jev——它们就是硬约束。

use if_domain::id::{CandidateId, EventId, SceneId, TurnId, WorldLineId};
use if_domain::narrative::{ScenePlan, Timestamp, Tendency};
use if_domain::projection::Projection;
use if_domain::rule::WorldSettings;
use if_domain::subject::Proposition;
use if_domain::turn::{TurnKind, TurnRecord};
use if_domain::value::WorldTime;
use if_judge::Judge;

use crate::audit::Audit;
use crate::beats::{self, BeatProposal, BeatRound};
use crate::candidates::{self, ImpactOutcome, ImpactRequest};
use crate::commit::{self, DraftCursor, ObservedChange, ProposedChange, Reconciliation, SceneCommit, TurnCommit};
use crate::context::TurnContext;
use crate::scenes::{self, SceneChoice, SceneProposal, SceneRequest};
use crate::PipelineError;

/// 第一段（第 6–9 步）的输入。
#[derive(Debug)]
pub struct OpenRequest<'a> {
    pub projection: &'a Projection,
    pub settings: &'a WorldSettings,
    pub ctx: &'a TurnContext,
    pub candidates: Vec<if_domain::turn::Candidate>,
    pub scenes: Vec<SceneProposal>,
    pub source_event: Option<EventId>,
    pub triggered: Vec<CandidateId>,
    /// 导演评分用的温度。`None` 取初始值。
    pub temperature: Option<f64>,
    pub salt: String,
}

/// 第一段的产出。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Opening {
    pub impact: ImpactOutcome,
    pub scenes: SceneChoice,
    /// 第一段用掉的判定序号游标。**第二段接着它往下发号**——
    /// 不这么做的话 `Beat::judgments` 里的 `jdg_0001` 会同时指向影响裁决的第一条记录。
    /// 调用方不用管它：[`resolve`] 会自己取。
    pub audit: Audit,
    pub warnings: Vec<String>,
}

impl Opening {
    /// 抽中的场景提议。
    pub fn scene(&self) -> Option<&SceneProposal> {
        self.scenes.proposal()
    }

    /// 放行候选对应的预演变化。**这一步不改世界**，只是把「骰子说会发生什么」摊开，
    /// 供 T-plan 与正文写作参考。
    pub fn proposed(&self, projection: &Projection) -> (Vec<ProposedChange>, Vec<String>) {
        let mut changes: Vec<ProposedChange> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for candidate in self.impact.accepted() {
            let selected = self.impact.selected_option(&candidate.id);
            let (mut produced, mut notes) = commit::proposed_from(candidate, projection, selected);
            changes.append(&mut produced);
            warnings.append(&mut notes);
        }
        changes.sort_by(|a, b| (&a.prop, &a.candidate).cmp(&(&b.prop, &b.candidate)));
        changes.dedup_by(|a, b| a.prop == b.prop && a.candidate == b.candidate);
        (changes, warnings)
    }
}

/// 跑第 6–9 步。
pub fn open(judge: &dyn Judge, request: OpenRequest<'_>) -> Result<Opening, PipelineError> {
    let mut audit = Audit::new();
    let impact = candidates::adjudicate(
        judge,
        ImpactRequest::new(
            request.projection,
            request.settings,
            request.ctx,
            request.candidates,
        )
        .from_event_opt(request.source_event.clone())
        .triggering(request.triggered.iter().cloned()),
        &mut audit,
    )?;

    let mut scene_request = SceneRequest::new(
        request.projection,
        request.settings,
        request.ctx,
        request.scenes,
    );
    if let Some(temperature) = request.temperature {
        scene_request = scene_request.temperature(temperature);
    }
    if !request.salt.is_empty() {
        scene_request = scene_request.salt(request.salt.clone());
    }
    let scenes = scenes::choose(judge, scene_request, &mut audit)?;

    let mut warnings = impact.warnings.clone();
    warnings.extend(scenes.warnings.iter().cloned());
    Ok(Opening {
        impact,
        scenes,
        audit,
        warnings,
    })
}

/// 第二段（第 10–13 步）的输入。
#[derive(Debug)]
pub struct ResolveRequest<'a> {
    pub projection: &'a Projection,
    pub settings: &'a WorldSettings,
    pub ctx: &'a TurnContext,
    pub opening: &'a Opening,
    /// 选中的场景 ID。必须与 `opening.scene()` 一致——不一致说明调用方在中间换了场景。
    pub scene: SceneId,
    /// T-plan 的产物。引擎会用 [`inject_constraints`] 补上硬约束再校验。
    pub plan: ScenePlan,
    /// T-render 切开来的节拍。同一 `index` 出现多次 = 同一次节拍的重写尝试。
    pub beats: Vec<BeatProposal>,
    /// T-extract 抽出来的、已展示正文里的变化（docs/02 §11）。
    pub observed: Vec<ObservedChange>,
    /// 需要新建的命题。会排在任何引用它们的 `FactSet` 之前写出去。
    pub new_propositions: Vec<Proposition>,
    /// 本回合另外产生的趋势（机制、后台结算带来的）。
    pub tendencies: Vec<Tendency>,
    /// `Store::next_seq()`。事件 ID 从它起顺号发（见 [`crate::commit`] 的分配契约）。
    pub first_seq: u64,
    pub displayed_at: Option<Timestamp>,
    /// 世界时间推进到的时刻。`None` = 不推进。
    pub time: Option<WorldTime>,
}

/// 第二段的产出。
#[derive(Clone, Debug, PartialEq)]
pub struct SceneOutcome {
    /// 注入硬约束之后的计划——**这一份才是真正生效的**。
    pub plan: ScenePlan,
    /// 引擎注入的硬约束说明，供审计与界面展示。
    pub injections: Vec<String>,
    pub beats: BeatRound,
    pub reconciliation: Reconciliation,
    pub drafts: Vec<if_domain::event::EventDraft>,
    pub committed: Vec<EventId>,
    /// 本回合新建的趋势。
    pub tendencies: Vec<Tendency>,
    pub warnings: Vec<String>,
}

impl SceneOutcome {
    /// 场景是否走到了收束（停止条件达成）。
    pub fn completed(&self) -> bool {
        matches!(self.beats.stopped, Some(crate::beats::BeatStop::Condition))
    }
}

/// 跑第 10–13 步。
pub fn resolve(judge: &dyn Judge, request: ResolveRequest<'_>) -> Result<SceneOutcome, PipelineError> {
    let mut warnings: Vec<String> = Vec::new();

    // 场景必须与第一段选出来的一致，否则「掷骰选了 A，正文写的是 B」会静默发生。
    if let Some(chosen) = request.opening.scenes.scene_id() {
        if chosen != &request.scene {
            return Err(PipelineError::invalid(format!(
                "场景 {chosen} 是第一段抽中的那条，resolve 收到的却是 {}",
                request.scene
            )));
        }
    }

    let mut plan = request.plan;
    let injections = inject_constraints(request.projection, &mut plan);
    // 计划的两条硬校验由领域层给（present 非空、视角人物在场）。
    plan.validate()?;
    for subject in &plan.present {
        if !request.projection.subjects.contains_key(subject) {
            warnings.push(format!("场景计划里的在场主体 {subject} 不在世界里"));
        }
    }

    // 接着第一段的游标发号——判定记录在一个回合里必须唯一（见 crate::audit）。
    let mut audit = request.opening.audit.clone();
    let mut round = beats::run(
        judge,
        beats::BeatRoundRequest::new(
            request.projection,
            request.settings,
            request.ctx,
            &plan,
            request.scene.clone(),
            request.beats,
        ),
        &mut audit,
    )?;
    // 上屏时刻盖在放行时就盖好，而不是只在写草稿时盖：`SceneOutcome` 里的节拍
    // 要与它写出去的那条 `beat_displayed` 事件一致，调用方拿到的产物才对得上日志。
    // 它不进投影（docs/02 §10），所以读时钟仍然只发生在调用方那一侧。
    for beat in &mut round.admitted {
        beat.displayed_at = request.displayed_at;
    }
    warnings.extend(round.warnings.iter().cloned());

    // ---- 对账（docs/02 §11）
    let (proposed, mut notes) = request.opening.proposed(request.projection);
    warnings.append(&mut notes);
    let reconciliation = commit::reconcile(&proposed, &request.observed);
    warnings.extend(reconciliation.warnings.iter().cloned());

    // ---- 组装并写出
    let mut tendencies = reconciliation_tendencies(request.opening);
    tendencies.extend(request.tendencies.iter().cloned());
    tendencies.sort_by(|a, b| a.id.cmp(&b.id));
    tendencies.dedup_by(|a, b| a.id == b.id);

    let commit = TurnCommit {
        propositions: request.new_propositions.clone(),
        changes: reconciliation.committed.clone(),
        tendencies: tendencies.clone(),
        tendency_updates: Vec::new(),
        threads: Vec::new(),
        scene: Some(SceneCommit {
            id: request.scene.clone(),
            index: request.ctx.scene_index,
            plan: plan.clone(),
            started_at: request.ctx.at,
            // 场景在上一节拍处结束，收束时刻就是当前世界时间（没推进时间时）。
            completed_at: Some(request.time.unwrap_or(request.ctx.at)),
            beats: round.admitted.clone(),
        }),
        time: request.time,
        displayed_at: request.displayed_at,
    };

    let mut cursor = DraftCursor::new(
        request.ctx.line.clone(),
        request.ctx.turn.clone(),
        request.first_seq,
        request.ctx.narrative_order,
    );
    let (drafts, committed) = commit::drafts(&commit, &mut cursor);

    Ok(SceneOutcome {
        plan,
        injections,
        beats: round,
        reconciliation,
        drafts,
        committed,
        tendencies,
        warnings,
    })
}

/// 把引擎硬约束写进场景计划（docs/04 §2.1）。
///
/// 返回注入说明。目前只有一项：**受保护故事线的禁止项**。其余三项
/// （认知边界、规则语义约束、未批准揭示的秘密）不是计划字段能表达的，
/// 它们落在检查视图里——那是第 11 步的事（见 [`crate::beats`]）。
pub fn inject_constraints(projection: &Projection, plan: &mut ScenePlan) -> Vec<String> {
    let mut notes = Vec::new();
    let mut forbidden: Vec<(String, String)> = projection
        .threads
        .values()
        .filter_map(|thread| thread.forbidden_resolution().map(|text| (thread.id.to_string(), text)))
        .collect();
    forbidden.sort();
    for (id, text) in forbidden {
        if plan.forbidden_resolutions.iter().any(|existing| existing == &text) {
            continue;
        }
        plan.forbidden_resolutions.push(text.clone());
        notes.push(format!("引擎注入禁止项（来自受保护故事线 {id}）：{text}"));
    }
    notes
}

/// 把裁决产生的新趋势整理成可提交的那一份。
fn reconciliation_tendencies(opening: &Opening) -> Vec<Tendency> {
    let mut tendencies = opening.impact.adjudication.tendencies.clone();
    tendencies.sort_by(|a, b| a.id.cmp(&b.id));
    tendencies.dedup_by(|a, b| a.id == b.id);
    tendencies
}

/// 把两段的产出汇成一份回合记录（docs/03 §3）。
///
/// 判定与裁决不进事件日志，进这里——事件因此保持精简，审计信息也不丢。
pub fn record(
    turn: impl Into<TurnId>,
    line: impl Into<WorldLineId>,
    kind: TurnKind,
    started_at: WorldTime,
    input: Option<String>,
    opening: Option<&Opening>,
    scene: Option<&SceneOutcome>,
) -> TurnRecord {
    let mut record = TurnRecord::new(turn, line, kind, started_at);
    record.input = input;
    if let Some(opening) = opening {
        record.candidates = opening
            .impact
            .candidates
            .values()
            .cloned()
            .collect();
        record.judgments = opening.impact.judgments.clone();
        record.judgments.extend(opening.scenes.judgments.iter().cloned());
        record.resolutions = opening.impact.adjudication.resolutions.clone();
        record.scene_plan = scene.map(|scene| scene.plan.clone());
    }
    if let Some(scene) = scene {
        record.judgments.extend(scene.beats.judgments.iter().cloned());
        record.committed = scene.committed.clone();
    }
    // 判定记录按 ID 排序，保证同一输入产出同一份记录。
    record.judgments.sort_by(|a, b| a.id.cmp(&b.id));
    record
}

#[cfg(test)]
mod tests;
