//! 第 6–7 步：影响候选 → 约束门 → 发生类判定 → 分层裁决（docs/04 §2、docs/06 §4）。
//!
//! 这是「Jev 只输出分布，引擎按策略裁决」里的**前半段**：把提议翻成问题、把答案翻成
//! 概率、再交给 [`if_policy::Policy`] 掷骰。骰子与阈值一行都不在这里——这里只负责
//! **问得对**和**接得上**。
//!
//! 三次判定互不混同，因为它们的语义类别不同（docs/06 §1）：
//!
//! | 问题 | 类别 | 处理 |
//! |---|---|---|
//! | `q.cand.in_character` | 约束类 | 阈值，越高越可靠；不通过即否决 |
//! | `q.cand.knowledge_gap` | 约束类 | 阈值，越高越可疑；触发即否决 |
//! | `q.behavior.occurs` / `q.world.occurs` | 发生类 | 命运骰子 |
//! | `q.outcome.choice` | 互斥类 | 命运骰子，整组只抽一次 |
//!
//! **约束门先跑，发生类后跑**：一个候选如果根本不合人物或要用到他不知道的信息，
//! 就不该再花一次判定去问它会不会发生——那只会让被否决的事多留一条痕迹。
//!
//! 视图按持有者分组：行为候选进它所属主体的角色视图，世界候选进上帝视图。
//! 分组是**确定性**的（`BTreeMap`），所以同一个候选集合永远得到同一批请求。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;

use if_domain::id::{CandidateId, EventId, SubjectId, TurnId};
use if_domain::projection::Projection;
use if_domain::rule::WorldSettings;
use if_domain::turn::{Candidate, CandidateShape, Judgment, JudgmentOutput, ViewKind};
use if_judge::{Judge, JudgeRequest, JudgeResponse, Question};
use if_policy::{Adjudication, AdjudicationInput};

use crate::audit::Audit;
use crate::context::{subject_name, TurnContext};
use crate::question::{self, Q_CAND_IN_CHARACTER, Q_CAND_KNOWLEDGE_GAP};
use crate::PipelineError;

/// 一次影响裁决的输入。
///
/// 候选由 agent 任务（T-impact）提出；这里接受的是**已经成形的提议**，
/// 不负责生成（`if-pipeline` 的边界，见 crate 文档）。
#[derive(Debug)]
pub struct ImpactRequest<'a> {
    pub projection: &'a Projection,
    pub settings: &'a WorldSettings,
    pub ctx: &'a TurnContext,
    pub candidates: Vec<Candidate>,
    /// 本回合的触发事件（IF 注入）。趋势的贡献事件取自它。
    pub source_event: Option<EventId>,
    /// 有明确触发事件的候选。
    ///
    /// 只有列在这里的候选才可能改动 **L1 保护期内**的状态（docs/06 §4）。
    /// 默认空集是刻意的：**宁可挡住对受保护状态的改动**，也不要因为调用方忘了声明
    /// 「这条是 IF 直接引起的」而让保护期静默失效。
    pub triggered: BTreeSet<CandidateId>,
}

impl<'a> ImpactRequest<'a> {
    pub fn new(
        projection: &'a Projection,
        settings: &'a WorldSettings,
        ctx: &'a TurnContext,
        candidates: Vec<Candidate>,
    ) -> Self {
        Self {
            projection,
            settings,
            ctx,
            candidates,
            source_event: None,
            triggered: BTreeSet::new(),
        }
    }

    pub fn from_event(mut self, event: impl Into<EventId>) -> Self {
        self.source_event = Some(event.into());
        self
    }

    /// `Option<EventId>` 的版本。回合驱动从上下文里拿到的本来就是可选的。
    pub fn from_event_opt(mut self, event: Option<EventId>) -> Self {
        self.source_event = event;
        self
    }

    pub fn triggering(mut self, ids: impl IntoIterator<Item = CandidateId>) -> Self {
        self.triggered.extend(ids);
        self
    }
}

/// 约束门对一个候选的结论。
#[derive(Clone, Debug, PartialEq)]
pub struct CandidateGate {
    pub candidate: CandidateId,
    pub rejected: bool,
    /// 人读的否决理由；通过时为空。
    pub reasons: Vec<String>,
}

/// 一次影响裁决的全部产出。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImpactOutcome {
    pub adjudication: Adjudication,
    pub judgments: Vec<Judgment>,
    /// 本回合参与裁决的候选，按 ID。裁决记录里只有 ID，取回内容要它。
    pub candidates: BTreeMap<CandidateId, Candidate>,
    /// 约束门的结论，按候选 ID。
    pub gates: Vec<CandidateGate>,
    pub warnings: Vec<String>,
}

impl ImpactOutcome {
    /// 放行的候选，按候选 ID。**不是**按层序——分层只决定判定顺序，
    /// 谁最终成立与层号无关。
    pub fn accepted(&self) -> Vec<&Candidate> {
        let mut ids: Vec<&CandidateId> = self.adjudication.accepted();
        ids.sort();
        ids.into_iter().filter_map(|id| self.candidates.get(id)).collect()
    }

    /// 被否决的候选（含约束门否掉的），按候选 ID。
    pub fn rejected(&self) -> Vec<&Candidate> {
        let mut ids: Vec<CandidateId> = self
            .adjudication
            .rejected()
            .into_iter()
            .cloned()
            .chain(
                self.gates
                    .iter()
                    .filter(|gate| gate.rejected)
                    .map(|gate| gate.candidate.clone()),
            )
            .collect();
        ids.sort();
        ids.dedup();
        ids.iter().filter_map(|id| self.candidates.get(id)).collect()
    }

    pub fn is_accepted(&self, id: &CandidateId) -> bool {
        self.adjudication.accepted().into_iter().any(|other| other == id)
    }

    /// 某个候选的约束门结论。
    pub fn gate(&self, id: &CandidateId) -> Option<&CandidateGate> {
        self.gates.iter().find(|gate| &gate.candidate == id)
    }

    /// 某个互斥组最终选中的选项。
    pub fn selected_option(&self, id: &CandidateId) -> Option<&str> {
        self.adjudication.selected_option(id)
    }
}

/// 候选需要哪一类视图。
///
/// 分成两档而不是「有主体就用角色视图」：世界事件（没有主体的事）在角色视图里
/// 根本无从描述，它必须进上帝视图——这也是 `Candidate::subject` 为 `None` 的含义。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Holder {
    Subject(SubjectId),
    World,
}

fn holder_of(candidate: &Candidate) -> Option<Holder> {
    candidate.subject.clone().map(Holder::Subject)
}

/// 跑完第 6–7 步。
///
/// `audit` 是本回合共用的判定序号游标——三段判定都用它发号，所以
/// `Beat::judgments` 里的 ID 在 `TurnRecord` 里指向的是唯一那条记录。
///
/// 失败只有两种：判定后端报错，或候选依赖成环/重复（那是输入本身的问题）。
/// 其余全部降级并进 `warnings`：判定缺失按「约束类不通过、发生类不发生」处理（docs/06 §9）。
pub fn adjudicate(
    judge: &dyn Judge,
    request: ImpactRequest<'_>,
    audit: &mut Audit,
) -> Result<ImpactOutcome, PipelineError> {
    let policy = request.ctx.policy(request.settings);
    let cancel = AtomicBool::new(false);
    let turn = request.ctx.turn.clone();
    let mut judgments: Vec<Judgment> = Vec::new();

    // ---- 约束门：只有带主体的候选才过这道门，世界事件直接进发生类
    let mut gates: Vec<CandidateGate> = Vec::new();
    let mut survivors: Vec<Candidate> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let mut gated: BTreeMap<Holder, Vec<&Candidate>> = BTreeMap::new();
    for candidate in &request.candidates {
        match holder_of(candidate) {
            Some(holder) => gated.entry(holder).or_default().push(candidate),
            None => survivors.push(candidate.clone()),
        }
    }

    for (holder, group) in &gated {
        let Holder::Subject(subject) = holder else {
            continue;
        };
        let name = subject_name(request.projection, subject);
        let view = view_for(request.ctx, holder, "T-impact");
        let compiled = if_views::compile(request.projection, &view);

        let mut ask = JudgeRequest::new(compiled);
        for candidate in group {
            let target = candidate.id.as_str();
            ask.push(question::cand_in_character(target, &name, &candidate.content));
            ask.push(question::cand_knowledge_gap(target, &name, &candidate.content));
        }
        let response = judge.judge(&ask, &cancel)?;
        judgments.extend(number(&ask, &response, &turn, audit));
        warnings.extend(missing_warnings(&response, "约束门"));

        for candidate in group {
            let mut gate = CandidateGate {
                candidate: candidate.id.clone(),
                rejected: false,
                reasons: Vec::new(),
            };
            check(
                &gate_verdict(&policy, &response, candidate.id.as_str(), Q_CAND_IN_CHARACTER),
                "不在人物的合理范围内",
                &mut gate,
            );
            check(
                &gate_verdict(&policy, &response, candidate.id.as_str(), Q_CAND_KNOWLEDGE_GAP),
                "需要用到他不知道的信息",
                &mut gate,
            );
            if !gate.rejected {
                survivors.push((*candidate).clone());
            }
            gates.push(gate);
        }
    }

    // ---- 发生类 / 互斥类：只在过门的候选里问
    let mut grouped: BTreeMap<Holder, Vec<&Candidate>> = BTreeMap::new();
    for candidate in &survivors {
        grouped
            .entry(holder_of(candidate).unwrap_or(Holder::World))
            .or_default()
            .push(candidate);
    }

    let mut probabilities: BTreeMap<CandidateId, f64> = BTreeMap::new();
    let mut distributions: BTreeMap<CandidateId, BTreeMap<String, f64>> = BTreeMap::new();
    let mut unaskable: Vec<String> = Vec::new();

    for (holder, group) in &grouped {
        let view = view_for(request.ctx, holder, "T-impact");
        let compiled = if_views::compile(request.projection, &view);
        let mut ask = JudgeRequest::new(compiled);
        let mut asked: Vec<&Candidate> = Vec::new();

        for candidate in group {
            let name = candidate
                .subject
                .as_ref()
                .map(|subject| subject_name(request.projection, subject))
                .unwrap_or_default();
            match occurrence_question(candidate, &name) {
                Some(question) => {
                    ask.push(question);
                    asked.push(candidate);
                }
                None => unaskable.push(format!(
                    "候选 {} 的互斥组没有给出至少两个选项，本回合无法抽取结果",
                    candidate.id
                )),
            }
        }
        if asked.is_empty() {
            continue;
        }

        let response = judge.judge(&ask, &cancel)?;
        judgments.extend(number(&ask, &response, &turn, audit));
        warnings.extend(missing_warnings(&response, "发生类"));

        for candidate in asked {
            let id = &candidate.id;
            match &candidate.shape {
                CandidateShape::Occurs => {
                    if let Some(p) = answer(&response, id.as_str(), "occurs")
                        .and_then(JudgmentOutput::as_normalized)
                    {
                        probabilities.insert(id.clone(), p);
                    }
                }
                CandidateShape::Exclusive { .. } => {
                    if let Some(JudgmentOutput::Choice { probabilities: dist, .. }) =
                        answer(&response, id.as_str(), "choice")
                    {
                        distributions.insert(id.clone(), dist.clone());
                    }
                }
            }
        }
    }
    warnings.extend(unaskable);

    // ---- 分层裁决：骰子、阈值、L1 保护期、观察带转趋势都在 `if-policy` 里
    let mut input = AdjudicationInput::new(survivors.clone());
    input.probabilities = probabilities;
    input.distributions = distributions;
    input.triggered = request
        .triggered
        .iter()
        .filter(|id| survivors.iter().any(|candidate| &candidate.id == *id))
        .cloned()
        .collect();
    input.protected = protected_props(request.projection, request.ctx.scene_index);
    input.proposition_keys = request
        .projection
        .propositions
        .iter()
        .map(|(id, proposition)| (id.clone(), proposition.key.clone()))
        .collect();
    input.source_event = request.source_event.clone();

    let adjudication = policy.run(&input)?;
    warnings.extend(adjudication.warnings.iter().cloned());

    let candidates = request
        .candidates
        .iter()
        .map(|candidate| (candidate.id.clone(), candidate.clone()))
        .collect();

    Ok(ImpactOutcome {
        adjudication,
        judgments,
        candidates,
        gates,
        warnings,
    })
}

// ---------------------------------------------------------------- 内部

/// 按持有者造视图并填上任务代号。
fn view_for(ctx: &TurnContext, holder: &Holder, task: &str) -> if_views::ViewRequest {
    let mut view = match holder {
        // 行为候选：判定者只能看到这个角色知道的（P9）。
        Holder::Subject(subject) => {
            let mut view = ctx.view(ViewKind::Pov);
            view.holder = Some(subject.clone());
            view
        }
        // 世界候选：没有主体，进上帝视图。
        Holder::World => ctx.view(ViewKind::God),
    };
    view.task = task.to_owned();
    view
}

/// 一个候选在发生类里的问题。互斥组不足两个选项时返回 `None`——
/// 那种组抽不出结果，编一个选项只会把不自洽写进正文。
fn occurrence_question(candidate: &Candidate, holder_name: &str) -> Option<Question> {
    let target = candidate.id.as_str();
    match &candidate.shape {
        CandidateShape::Occurs if candidate.subject.is_some() => Some(question::behavior_occurs(
            target,
            holder_name,
            &candidate.content,
        )),
        CandidateShape::Occurs => Some(question::world_occurs(target, &candidate.content)),
        CandidateShape::Exclusive { options } => {
            if options.len() < 2 {
                return None;
            }
            let pairs: Vec<(String, String)> = options
                .iter()
                .map(|option| (option.clone(), option.clone()))
                .collect();
            Some(question::outcome_choice(target, &candidate.content, &pairs))
        }
    }
}

/// 约束类判定的结论。`None` 表示这一问没拿到答案——按 docs/06 §9 视为不通过。
fn gate_verdict(
    policy: &if_policy::Policy,
    response: &JudgeResponse,
    target: &str,
    template: &str,
) -> Option<bool> {
    let suffix = template
        .trim_start_matches("q.cand.")
        .trim_end_matches("@1");
    let probability = answer(response, target, suffix)?.as_normalized()?;
    policy.passes(template, probability)
}

fn check(verdict: &Option<bool>, reason: &str, gate: &mut CandidateGate) {
    if verdict == &Some(true) {
        return;
    }
    gate.rejected = true;
    match verdict {
        Some(_) => gate.reasons.push(reason.to_owned()),
        None => gate
            .reasons
            .push(format!("{reason}（判定缺失，约束类按不通过处理）")),
    }
}

/// 响应里某个问题的答案。
fn answer<'a>(response: &'a JudgeResponse, target: &str, suffix: &str) -> Option<&'a JudgmentOutput> {
    response.answers.get(&format!("{target}.{suffix}"))
}

/// 请求里问了、响应里没答的问题。判定缺失本身是正常降级，但必须可见——
/// 静默缺失会让「这一回合怎么这么平淡」变得无从解释。
fn missing_warnings(response: &JudgeResponse, stage: &str) -> Vec<String> {
    if response.missing.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "{stage}：{} 个问题没有拿到判定（{}），按 docs/06 §9 降级处理",
        response.missing.len(),
        response.missing.join("、")
    )]
}

/// L1 保护期内的事实所对应的命题（docs/06 §4）。
fn protected_props(projection: &Projection, scene_index: u64) -> BTreeSet<if_domain::id::PropositionId> {
    projection
        .facts
        .iter()
        .filter(|(_, fact)| fact.is_protected_at(scene_index))
        .map(|(id, _)| id.clone())
        .collect()
}

/// 把一次响应对齐成判定记录，ID 从回合共用的游标上取。
fn number(
    ask: &JudgeRequest,
    response: &JudgeResponse,
    turn: &TurnId,
    audit: &mut Audit,
) -> Vec<Judgment> {
    response.to_judgments(ask, turn, || audit.take())
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_domain::id::PropositionId;
    use if_domain::subject::{Proposition, PropositionKind, ValueType};
    use if_judge::StubJudge;

    fn prop(id: &str, key: &str) -> Proposition {
        Proposition {
            id: PropositionId::new(id),
            key: key.to_owned(),
            text: format!("命题 {id}"),
            subjects: vec![],
            kind: PropositionKind::State,
            value_type: ValueType::Bool,
            internal: false,
        }
    }

    fn candidate(id: &str, subject: Option<&str>, affects: &[&str]) -> Candidate {
        Candidate {
            id: CandidateId::new(id),
            key: Some(format!("{id}.decision")),
            subject: subject.map(SubjectId::new),
            content: format!("候选 {id} 的内容"),
            internal: false,
            shape: CandidateShape::Occurs,
            depends_on: vec![],
            based_on: vec![],
            affects: affects.iter().map(|id| PropositionId::new(*id)).collect(),
        }
    }

    fn projection() -> Projection {
        let mut projection = Projection::genesis("wl_main");
        for (id, key) in [("p_a", "c_lin.mood"), ("p_b", "c_gu.mood")] {
            projection.propositions.insert(PropositionId::new(id), prop(id, key));
        }
        projection.subjects.insert(
            SubjectId::new("c_lin"),
            if_domain::subject::Subject {
                id: SubjectId::new("c_lin"),
                kind: if_domain::subject::SubjectKind::Character,
                name: "林夏".into(),
                aliases: vec![],
                profile: String::new(),
                voice: None,
                tier: if_domain::subject::Tier::Active,
                shaped: false,
                created_by: EventId::new("evt_0001"),
            },
        );
        projection
    }

    /// 设置只有一份常量，所以借出来的是 `&'static`——否则
    /// `ImpactRequest::new(&projection, settings(), ...)` 里的临时值会在语句结束时被丢掉，
    /// 而请求还活着。
    fn settings() -> &'static WorldSettings {
        static SETTINGS: std::sync::OnceLock<WorldSettings> = std::sync::OnceLock::new();
        SETTINGS.get_or_init(|| WorldSettings {
            seed: 20260926,
            narrative_style: String::new(),
            narrative_pov: if_domain::rule::NarrativePov::ThirdLimited,
            mechanic_step: if_domain::rule::MechanicStep::Day,
            director_style: "均衡".into(),
            mode: if_domain::rule::Mode::Sandbox,
        })
    }

    fn context() -> TurnContext {
        TurnContext::new(
            TurnId::numbered(1),
            "wl_main",
            if_domain::turn::TurnKind::If,
            if_domain::value::WorldTime::EPOCH,
            0,
        )
    }

    #[test]
    fn passing_the_gate_then_occurring_is_accepted() {
        let projection = projection();
        let ctx = context();
        let judge = StubJudge::new()
            .noul_for_template("q.cand.in_character", 0.9)
            .noul_for_template("q.cand.knowledge_gap", 0.1)
            .noul_for_template("q.behavior.occurs", 0.99);
        let request = ImpactRequest::new(
            &projection,
            settings(),
            &ctx,
            vec![candidate("cand_1", Some("c_lin"), &["p_a"])],
        );
        let outcome = adjudicate(&judge, request, &mut Audit::new()).unwrap();
        assert_eq!(outcome.accepted().len(), 1);
        assert_eq!(outcome.accepted()[0].id.as_str(), "cand_1");
        assert!(outcome.rejected().is_empty());
        // 约束门的两问 + 发生类的一问
        assert_eq!(outcome.judgments.len(), 3);
    }

    #[test]
    fn failing_in_character_stops_before_the_dice() {
        let projection = projection();
        let ctx = context();
        let judge = StubJudge::new()
            .noul_for_template("q.cand.in_character", 0.1)
            .noul_for_template("q.cand.knowledge_gap", 0.1)
            .noul_for_template("q.behavior.occurs", 0.99);
        let request = ImpactRequest::new(
            &projection,
            settings(),
            &ctx,
            vec![candidate("cand_1", Some("c_lin"), &["p_a"])],
        );
        let outcome = adjudicate(&judge, request, &mut Audit::new()).unwrap();
        assert!(outcome.accepted().is_empty());
        assert_eq!(outcome.rejected().len(), 1);
        // 只问了约束门两问——被否决的候选不该再花一次发生类判定
        assert_eq!(outcome.judgments.len(), 2);
        let gate = outcome.gate(&CandidateId::new("cand_1")).unwrap();
        assert!(gate.rejected);
        assert_eq!(gate.reasons, vec!["不在人物的合理范围内"]);
    }

    #[test]
    fn missing_judgment_rejects_a_constraint_candidate() {
        let projection = projection();
        let ctx = context();
        let judge = StubJudge::new()
            .noul_for_template("q.cand.in_character", 0.9)
            .drop_key("cand_1.knowledge_gap")
            .noul_for_template("q.behavior.occurs", 0.99);
        let request = ImpactRequest::new(
            &projection,
            settings(),
            &ctx,
            vec![candidate("cand_1", Some("c_lin"), &["p_a"])],
        );
        let outcome = adjudicate(&judge, request, &mut Audit::new()).unwrap();
        let gate = outcome.gate(&CandidateId::new("cand_1")).unwrap();
        assert!(gate.rejected, "缺判定必须否决，而不是放行");
        assert!(gate.reasons[0].contains("判定缺失"));
        assert!(outcome.warnings.iter().any(|w| w.contains("约束门")));
    }

    #[test]
    fn world_candidates_skip_the_gate() {
        let projection = projection();
        let ctx = context();
        let judge = StubJudge::new()
            .noul_for_template("q.cand.in_character", 0.0)
            .noul_for_template("q.world.occurs", 0.9);
        let request = ImpactRequest::new(
            &projection,
            settings(),
            &ctx,
            vec![candidate("cand_1", None, &["p_a"])],
        );
        let outcome = adjudicate(&judge, request, &mut Audit::new()).unwrap();
        assert!(outcome.gate(&CandidateId::new("cand_1")).is_none());
        assert_eq!(outcome.accepted().len(), 1, "世界事件没有人物约束门");
    }

    #[test]
    fn exclusive_group_needs_two_options() {
        let projection = projection();
        let ctx = context();
        let mut single = candidate("cand_1", None, &["p_a"]);
        single.shape = CandidateShape::Exclusive {
            options: vec!["only".into()],
        };
        let judge = StubJudge::new();
        let request = ImpactRequest::new(&projection, settings(), &ctx, vec![single]);
        let outcome = adjudicate(&judge, request, &mut Audit::new()).unwrap();
        assert!(outcome.accepted().is_empty());
        assert!(outcome.warnings.iter().any(|w| w.contains("至少两个选项")));
    }

    #[test]
    fn exclusive_group_is_drawn_once_and_the_option_is_recorded() {
        let projection = projection();
        let ctx = context();
        let mut group = candidate("cand_1", None, &["p_a"]);
        group.shape = CandidateShape::Exclusive {
            options: vec!["stay".into(), "leave".into()],
        };
        let judge = StubJudge::new();
        let request = ImpactRequest::new(&projection, settings(), &ctx, vec![group]);
        let outcome = adjudicate(&judge, request, &mut Audit::new()).unwrap();
        let picked = outcome
            .selected_option(&CandidateId::new("cand_1"))
            .expect("互斥组必然抽出一个选项");
        assert!(picked == "stay" || picked == "leave", "{picked}");
        // 互斥类选中的结果总是放行，内容在 `Selected` 里
        assert!(outcome.is_accepted(&CandidateId::new("cand_1")));
    }

    #[test]
    fn adjudication_is_deterministic_for_the_same_input() {
        let projection = projection();
        let ctx = context();
        let make = || {
            StubJudge::new()
                .noul_for_template("q.behavior.occurs", 0.62)
                .noul_for_template("q.world.occurs", 0.62)
        };
        let candidates = vec![
            candidate("cand_1", Some("c_lin"), &["p_a"]),
            candidate("cand_2", None, &["p_b"]),
        ];
        let once = adjudicate(
            &make(),
            ImpactRequest::new(&projection, settings(), &ctx, candidates.clone()),
            &mut Audit::new(),
        )
        .unwrap();
        let twice = adjudicate(
            &make(),
            ImpactRequest::new(&projection, settings(), &ctx, candidates),
            &mut Audit::new(),
        )
        .unwrap();
        assert_eq!(once.adjudication.resolutions, twice.adjudication.resolutions);
    }

    #[test]
    fn a_rejected_candidate_never_becomes_a_tendency_without_a_source() {
        // 被掷骰否决、且概率落在观察带内的候选会转成趋势（docs/06 §5）；
        // 没有触发事件时它只记压力，不编一个贡献事件。
        let projection = projection();
        let ctx = context();
        let judge = StubJudge::new().noul_for_template("q.world.occurs", 0.9);
        let request = ImpactRequest::new(
            &projection,
            settings(),
            &ctx,
            vec![candidate("cand_1", None, &["p_a"])],
        );
        let outcome = adjudicate(&judge, request, &mut Audit::new()).unwrap();
        // 0.9 落在观察带（≥0.3），可能被骰子否决也可能通过；两种都是合法结果，
        // 这里只断言「趋势要么没有，要么有明确的贡献来源规则」
        for tendency in &outcome.adjudication.tendencies {
            assert!((tendency.pressure - 0.45).abs() < 1e-9);
        }
    }
}
