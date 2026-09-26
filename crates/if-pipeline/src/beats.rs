//! 第 11 步：逐节拍检查与放行（docs/04 §4）。
//!
//! 这是「以 Jev 约束 LLM」最直接的那一半：正文一旦写出，就在**进历史之前**过一遍
//! 合规检查。没过就不展示、不入历史，host 把有针对性的修改说明追加进对话让模型重写。
//!
//! 三件事必须分清：
//!
//! 1. **检查用检查视图**——它含未批准揭示的秘密，否则查不出泄露（docs/04 §2.1）。
//! 2. **否决理由不能说出秘密**。检查视图里可以写「是否透露了 X」，但回给模型的说明
//!    必须是「涉及尚未批准揭示的内容」——否则第一次否决就把秘密交代了（docs/05 §2.5）。
//!    所以 [`BeatBlock`] 分两个字段：`reason` 是能说给模型听的那一份，
//!    `template` 与 `probability` 只进判定记录与日志。
//! 3. **约束类不掷骰**（docs/06 §1）。这里没有任何骰子，只有阈值；缺判定按
//!    「不通过」处理（docs/06 §9）——宁可让模型重写一次，也不要让违规正文上屏。
//!
//! 重试的编排在这里但**重写不在**：调用方把同一 `index` 的第 2、3 次尝试接着传进来
//! （[`BeatProposal`] 是有序的），这一层负责计数、跳过与止损。真正的重写请求
//! （降低温度、附完整约束清单）是 host 的事（docs/04 §4.5 第 2 条瀑布）。
//!
//! 背压（docs/04 §4.7）也不在这里：那是流式读取侧的事——本模块只看已经切好的片段。

use std::collections::BTreeMap;

use if_domain::id::{BeatId, JudgmentId, SceneId, SubjectId};
use if_domain::narrative::{Beat, RevealTarget, ScenePlan, MAX_BEATS_PER_SCENE, MAX_BEAT_RETRIES};
use if_domain::projection::Projection;
use if_domain::rule::WorldSettings;
use if_domain::turn::{Judgment, JudgmentOutput, ViewKind};
use if_domain::value::Visibility;
use if_judge::{Judge, JudgeRequest, JudgeResponse};
use if_policy::Policy;

use crate::audit::Audit;
use crate::context::{subject_name, TurnContext};
use crate::question::{
    self, Q_BEAT_FORBIDDEN_RESOLUTION, Q_BEAT_GOAL_DONE, Q_BEAT_KNOWLEDGE_LEAK,
    Q_BEAT_REVEALS_SECRET, Q_BEAT_STOP_REACHED, Q_BEAT_VIOLATES_FACT, Q_BEAT_VIOLATES_RULE,
};
use crate::PipelineError;

/// 单次请求里最多检查几条事实【初始值 12】。
///
/// 真实卡的设定量在 10 万字量级（docs/13 §6.4），把全部事实塞进一次节拍检查既超预算
/// 也没有必要——只有锁得最紧的那几条才可能被正文违背。截断按锁定等级**降序**（越紧越先）、
/// 命题键升序，所以同一个世界永远截到同一批。
pub const MAX_FACT_CHECKS: usize = 12;

/// 单次请求里最多检查几条规则【初始值 8】。
pub const MAX_RULE_CHECKS: usize = 8;

/// 一个节拍提议（T-render 的输出，按分隔标记切开的一个片段）。
#[derive(Clone, Debug, PartialEq)]
pub struct BeatProposal {
    pub index: u32,
    pub text: String,
    /// 必需的节拍被否决时不能跳过，只能在最后一个已放行的节拍处结束场景
    /// （docs/04 §4.5 第 1 条瀑布）。
    pub required: bool,
    /// 对应场景计划 `required_beats` 里的哪一条。
    pub plan_beat: Option<usize>,
}

impl BeatProposal {
    pub fn new(index: u32, text: impl Into<String>) -> Self {
        Self {
            index,
            text: text.into(),
            required: false,
            plan_beat: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn plan_beat(mut self, index: usize) -> Self {
        self.plan_beat = Some(index);
        self
    }
}

/// 一条未通过的检查。
///
/// `reason` 是**能说给模型听**的那一份。秘密只写成「涉及尚未批准揭示的内容」——
/// 问题里可以点出秘密，理由里不行（docs/05 §2.5）。
#[derive(Clone, Debug, PartialEq)]
pub struct BeatBlock {
    pub template: String,
    pub target: String,
    /// 触发方向上的概率。缺失表示这一问没拿到判定，按不通过处理（docs/06 §9）。
    pub probability: Option<f64>,
    pub reason: String,
}

/// 一次节拍检查的结论。
#[derive(Clone, Debug, PartialEq)]
pub struct BeatVerdict {
    pub index: u32,
    pub admitted: bool,
    pub blocks: Vec<BeatBlock>,
    /// 停止条件是否已经达成（docs/04 §4.2）。达成即结束场景。
    pub stop_reached: bool,
    /// 节拍目标完成度。**没有阈值表项**，所以只报原始概率，不据此裁决——
    /// 给它硬编一个阈值会让「完成了吗」变成一个悄悄生效的规则。
    pub goal_done: Option<f64>,
    pub judgments: Vec<Judgment>,
    pub warnings: Vec<String>,
}

impl BeatVerdict {
    pub fn judgment_ids(&self) -> Vec<JudgmentId> {
        self.judgments
            .iter()
            .map(|judgment| judgment.id.clone())
            .collect()
    }
}

/// 一次节拍检查的输入。
#[derive(Debug)]
pub struct BeatRequest<'a> {
    pub projection: &'a Projection,
    pub settings: &'a WorldSettings,
    pub ctx: &'a TurnContext,
    pub plan: &'a ScenePlan,
    pub beat: BeatProposal,
    /// 此前已放行的节拍原文。`q.beat.stop_reached` 的「累计到哪一步」靠它。
    pub prior: Vec<String>,
}

/// 检查一个节拍，决定是否放行。
///
/// `audit` 是本回合共用的判定序号游标（见 [`crate::audit`]）——
/// [`Beat::judgments`] 里的 ID 要能在 `TurnRecord::judgments` 里唯一指回这一条。
pub fn check(
    judge: &dyn Judge,
    request: BeatRequest<'_>,
    audit: &mut Audit,
) -> Result<BeatVerdict, PipelineError> {
    let policy = request.ctx.policy(request.settings);
    let index = request.beat.index;

    // 检查视图：含未批准揭示的秘密（否则查不出泄露），并且带节拍原文。
    let view = request
        .ctx
        .view(ViewKind::Check)
        .scene_plan(request.plan.clone())
        .beat_text(request.beat.text.clone())
        .task("T-render");
    let compiled = if_views::compile(request.projection, &view);

    let mut ask = JudgeRequest::new(compiled);
    let mut slots: Vec<Slot> = Vec::new();

    // ---- 事实（越紧的越先问）
    for fact in checkable_facts(request.projection, request.ctx) {
        let question = question::beat_violates_fact(index, &fact.id, &fact.text);
        slots.push(Slot::new(
            Q_BEAT_VIOLATES_FACT,
            safe_detail(fact.visibility, &fact.text),
        ));
        ask.push(question);
    }

    // ---- 规则
    for (id, text, boundaries) in checkable_rules(request.projection, request.ctx) {
        let question = question::beat_violates_rule(index, &id, &text, &boundaries);
        slots.push(Slot::new(Q_BEAT_VIOLATES_RULE, Some(text.clone())));
        ask.push(question);
    }

    // ---- 认知边界：每个在场角色一问
    let relevant: Vec<String> = checkable_facts(request.projection, request.ctx)
        .into_iter()
        .map(|fact| fact.key)
        .collect();
    for subject in &request.plan.present {
        let name = subject_name(request.projection, subject);
        let known = known_keys(request.projection, subject, &relevant, request.ctx.scene_index);
        let question = question::beat_knowledge_leak(index, &name, &known);
        slots.push(Slot::new(Q_BEAT_KNOWLEDGE_LEAK, Some(name)));
        ask.push(question);
    }

    // ---- 本场景被禁止的结果
    for (ordinal, forbidden) in request.plan.forbidden_resolutions.iter().enumerate() {
        let question = question::beat_forbidden_resolution(index, ordinal, forbidden);
        slots.push(Slot::new(
            Q_BEAT_FORBIDDEN_RESOLUTION,
            Some(forbidden.clone()),
        ));
        ask.push(question);
    }

    // ---- 未批准揭示的秘密。**理由里绝不出现秘密本身**，所以 detail 是 None。
    for (key, text) in unrevealed_secrets(request.projection, request.plan, request.ctx.scene_index) {
        let question = question::beat_reveals_secret(index, &key, &text);
        slots.push(Slot::new(Q_BEAT_REVEALS_SECRET, None));
        ask.push(question);
    }

    // ---- 停止条件与节拍目标
    let stop = question::beat_stop_reached(index, &request.plan.stop_condition, &request.prior);
    slots.push(Slot::new(Q_BEAT_STOP_REACHED, None));
    ask.push(stop);
    let goal = question::beat_goal_done(index, &request.plan.goal);
    slots.push(Slot::new(Q_BEAT_GOAL_DONE, None));
    ask.push(goal);

    // 问题键必须唯一（docs/07 §2 R1）。重复等于这一问的答案会覆盖另一问。
    ask.validate()?;

    let cancel = std::sync::atomic::AtomicBool::new(false);
    let response = judge.judge(&ask, &cancel)?;
    let turn = request.ctx.turn.clone();
    let judgments = response.to_judgments(&ask, &turn, || audit.take());

    Ok(verdict(&policy, &ask, &response, &slots, judgments, index))
}

// ---------------------------------------------------------------- 节拍循环

/// 同一节拍已被否决几次。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetryLedger {
    attempts: BTreeMap<u32, u32>,
}

impl RetryLedger {
    pub fn attempts(&self, index: u32) -> u32 {
        self.attempts.get(&index).copied().unwrap_or(0)
    }

    fn record(&mut self, index: u32) {
        *self.attempts.entry(index).or_insert(0) += 1;
    }

    /// 还能不能再试一次。上限是 `1 + MAX_BEAT_RETRIES`（首次 + 2 次重写，docs/04 §4.5）。
    pub fn can_retry(&self, index: u32) -> bool {
        self.attempts(index) < 1 + MAX_BEAT_RETRIES
    }
}

/// 场景为什么停下。
#[derive(Clone, Debug, PartialEq)]
pub enum BeatStop {
    /// 停止条件达成，正常收束。
    Condition,
    /// 必需节拍用尽重试。调用方可以降温度重写一次再回来（docs/04 §4.5 第 2 条瀑布）。
    Rejected {
        index: u32,
        blocks: Vec<BeatBlock>,
        exhausted: bool,
    },
    /// 达到每场景节拍上限（docs/04 §4.6）。
    Limit,
    /// 提议用完了。
    Exhausted,
}

/// 一个场景的节拍放行结果。
#[derive(Clone, Debug, PartialEq)]
pub struct BeatRound {
    pub scene: SceneId,
    /// 已放行的节拍，按放行顺序。
    pub admitted: Vec<Beat>,
    /// 被否决的节拍：`(节拍序号, 未通过的检查)`。同一序号可能有多条（重写后仍不过）。
    pub blocked: Vec<(u32, Vec<BeatBlock>)>,
    pub stopped: Option<BeatStop>,
    pub ledger: RetryLedger,
    /// 本场景全部放行检查的判定记录，按节拍顺序。审计要用——节拍上的 `judgments`
    /// 只存 ID，内容在这里。
    pub judgments: Vec<Judgment>,
    pub warnings: Vec<String>,
}

impl BeatRound {
    /// 一个还没有任何节拍的回合。
    pub fn new(scene: impl Into<SceneId>) -> Self {
        Self {
            scene: scene.into(),
            admitted: Vec::new(),
            blocked: Vec::new(),
            stopped: None,
            ledger: RetryLedger::default(),
            judgments: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// 最后一个放行节拍的下标；一个都没放行时返回 `None`。
    pub fn last_admitted(&self) -> Option<u32> {
        self.admitted.last().map(|beat| beat.index)
    }

    /// 最后一个放行节拍之后的节拍都没有上屏（docs/04 §4.5 第 3 条瀑布）。
    pub fn displayed_text(&self) -> String {
        self.admitted
            .iter()
            .map(|beat| beat.text.as_str())
            .collect::<Vec<_>>()
            .join("
")
    }
}

/// 一次节拍循环的输入。
#[derive(Debug)]
pub struct BeatRoundRequest<'a> {
    pub projection: &'a Projection,
    pub settings: &'a WorldSettings,
    pub ctx: &'a TurnContext,
    pub plan: &'a ScenePlan,
    pub scene: SceneId,
    /// 按渲染顺序排列的节拍提议。同一 `index` 出现多次 = 同一次节拍的重写尝试。
    pub beats: Vec<BeatProposal>,
    /// 每场景节拍上限【初始值 6】。
    pub max_beats: u32,
}

impl<'a> BeatRoundRequest<'a> {
    pub fn new(
        projection: &'a Projection,
        settings: &'a WorldSettings,
        ctx: &'a TurnContext,
        plan: &'a ScenePlan,
        scene: impl Into<SceneId>,
        beats: Vec<BeatProposal>,
    ) -> Self {
        Self {
            projection,
            settings,
            ctx,
            plan,
            scene: scene.into(),
            beats,
            max_beats: MAX_BEATS_PER_SCENE,
        }
    }
}

/// 走完一次节拍序列：检查、放行、重试、止损。
///
/// 停止条件（`q.beat.stop_reached`）达成后**不再读后面的节拍**——正文多写一段
/// 就多一段要检查、多一段可能违背事实的内容。
pub fn run(
    judge: &dyn Judge,
    request: BeatRoundRequest<'_>,
    audit: &mut Audit,
) -> Result<BeatRound, PipelineError> {
    let mut round = BeatRound::new(request.scene.clone());
    let mut prior: Vec<String> = Vec::new();

    for proposal in &request.beats {
        if round.admitted.len() as u32 >= request.max_beats {
            round.stopped = Some(BeatStop::Limit);
            break;
        }
        // 同一节拍的重写尝试之间不重复累计 `prior`。
        let verdict = check(
            judge,
            BeatRequest {
                projection: request.projection,
                settings: request.settings,
                ctx: request.ctx,
                plan: request.plan,
                beat: proposal.clone(),
                prior: prior.clone(),
            },
            audit,
        )?;
        round.warnings.extend(verdict.warnings.clone());
        round.judgments.extend(verdict.judgments.clone());
        round.ledger.record(proposal.index);

        if verdict.admitted {
            let beat = make_beat(&round.scene, proposal, verdict.judgment_ids());
            prior.push(proposal.text.clone());
            round.admitted.push(beat);
            if verdict.stop_reached {
                round.stopped = Some(BeatStop::Condition);
                break;
            }
            continue;
        }

        round.blocked.push((proposal.index, verdict.blocks.clone()));
        if round.ledger.can_retry(proposal.index) {
            // 还有一次重写机会：接着看序列里的下一个同号提议。
            continue;
        }
        if !proposal.required {
            // 非必需节拍：跳过它，继续往下走（docs/04 §4.5 第 1 条瀑布）。
            round.warnings.push(format!(
                "第 {} 个节拍重试 {} 次仍未通过，作为非必需节拍跳过",
                proposal.index, MAX_BEAT_RETRIES
            ));
            continue;
        }
        round.stopped = Some(BeatStop::Rejected {
            index: proposal.index,
            blocks: verdict.blocks.clone(),
            exhausted: true,
        });
        break;
    }

    if round.stopped.is_none() {
        round.stopped = Some(BeatStop::Exhausted);
    }
    Ok(round)
}

/// 节拍 ID 由场景 + 序号推出来（`scene_0001_b03`）。
///
/// 不额外发号：节拍不是事件，它的 ID 只需要在场景内唯一且可读，
/// 而项目里的 ID 约定本来就是「带前缀的序号」。
fn make_beat(scene: &SceneId, proposal: &BeatProposal, judgments: Vec<JudgmentId>) -> Beat {
    Beat {
        id: BeatId::new(format!("{}_b{:02}", scene.as_str(), proposal.index)),
        scene: scene.clone(),
        index: proposal.index,
        text: proposal.text.clone(),
        plan_beat: proposal.plan_beat,
        judgments,
        displayed_at: None,
    }
}

// ---------------------------------------------------------------- 内部

/// 一次提问在裁决里的位置：它的模板，以及**能说给模型听**的补充说明。
#[derive(Clone, Debug, PartialEq)]
struct Slot {
    template: &'static str,
    /// `None` = 这条检查涉及秘密，理由里不许出现它的内容。
    detail: Option<String>,
}

impl Slot {
    fn new(template: &'static str, detail: Option<String>) -> Self {
        Self { template, detail }
    }
}

/// 秘密的补充说明必须为空；其余按可见性决定能不能点名。
fn safe_detail(visibility: Visibility, text: &str) -> Option<String> {
    match visibility {
        Visibility::Secret => None,
        _ => Some(text.to_owned()),
    }
}

/// 把判定读成结论。
fn verdict(
    policy: &Policy,
    ask: &JudgeRequest,
    response: &JudgeResponse,
    slots: &[Slot],
    judgments: Vec<Judgment>,
    index: u32,
) -> BeatVerdict {
    let mut blocks: Vec<BeatBlock> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut stop_reached = false;
    let mut goal_done = None;

    for (question, slot) in ask.questions.iter().zip(slots) {
        let template = question.template.as_str();
        let probability = response
            .answers
            .get(&question.key)
            .and_then(JudgmentOutput::as_normalized);

        if slot.template == Q_BEAT_STOP_REACHED {
            // 缺判定时**不结束场景**：多写一段最多少写的事，误判结束会把场景砍在半途。
            stop_reached = probability
                .and_then(|p| policy.passes(template, p))
                .unwrap_or(false);
            continue;
        }
        if slot.template == Q_BEAT_GOAL_DONE {
            goal_done = probability;
            continue;
        }

        let passed = match policy.threshold(template) {
            Some(spec) => probability.is_some_and(|p| spec.passes(p)),
            // 没有阈值表项的模板不该出现在放行判断里。挡住比放过安全。
            None => false,
        };
        if passed {
            continue;
        }
        let mut reason = reason_for(slot.template).to_owned();
        if let Some(detail) = &slot.detail {
            reason = format!("{reason}：{detail}");
        }
        blocks.push(BeatBlock {
            template: template.to_owned(),
            target: format!("beat_{index}"),
            probability,
            reason,
        });
    }

    blocks.sort_by(|a, b| a.template.cmp(&b.template));
    if !response.missing.is_empty() {
        warnings.push(format!(
            "节拍 {index} 有 {} 个检查没拿到判定（{}），按不通过处理",
            response.missing.len(),
            response.missing.join("、")
        ));
    }

    BeatVerdict {
        index,
        admitted: blocks.is_empty(),
        blocks,
        stop_reached,
        goal_done,
        judgments,
        warnings,
    }
}

/// 给一条模板配一句**能说给模型听**的否决理由。
///
/// 键是模板 ID（带版本），比较的是**去掉版本后的名字**——两边都要去掉。
/// 只去掉输入那一侧是错的：`Q_BEAT_REVEALS_SECRET` 的值本身是
/// `q.beat.reveals_secret@1`，而 `base` 已经被削成 `q.beat.reveals_secret`，
/// 于是每一条分支都匹配不上，所有否决理由静默退化成「未通过检查」——
/// 模型于是拿不到「改哪儿」的提示，只能瞎改。
fn reason_for(template: &str) -> &'static str {
    let base = question::template_name(template);
    for (id, reason) in BLOCK_REASONS {
        if base == question::template_name(id) {
            return reason;
        }
    }
    "未通过检查"
}

/// 否决理由表。加一条检查项就必须在这里加一行——`tests::every_blocked_template_has_a_reason`
/// 会盯着这一点。
const BLOCK_REASONS: &[(&str, &str)] = &[
    (Q_BEAT_VIOLATES_FACT, "与已锁定的事实相矛盾"),
    (Q_BEAT_VIOLATES_RULE, "违反了当前生效的世界规则"),
    (Q_BEAT_KNOWLEDGE_LEAK, "让在场角色表现出了他不可能知道的信息"),
    (Q_BEAT_FORBIDDEN_RESOLUTION, "出现了本场景被禁止的结果"),
    (Q_BEAT_REVEALS_SECRET, "涉及尚未批准揭示的内容"),
];

/// 一条要过检查的事实。
///
/// `id` 与 `key` 都给是因为用途不同：问题键的后缀必须是**唯一**的（用 `id`），
/// 而「这个角色知道哪些」要按命题键说人话（用 `key`）。ID 来自投影的 `BTreeMap` 键，
/// 唯一性由结构保证；命题键是人写的，撞了也不该让整个节拍检查跑不起来。
#[derive(Clone, Debug, PartialEq)]
struct FactUnderCheck {
    id: String,
    key: String,
    text: String,
    visibility: Visibility,
}

/// 本场景要检查的事实：锁得最紧的那几条，按锁定等级降序、命题键升序截断。
fn checkable_facts(projection: &Projection, ctx: &TurnContext) -> Vec<FactUnderCheck> {
    let mut facts: Vec<(u8, String, FactUnderCheck)> = projection
        .facts
        .iter()
        .filter_map(|(id, fact)| {
            let proposition = projection.propositions.get(id)?;
            if !fact.is_valid_at(ctx.at) {
                return None;
            }
            if !if_views::fact_is_visible(
                ViewKind::Check,
                None,
                projection,
                proposition,
                fact,
                ctx.scene_index,
            ) {
                return None;
            }
            Some((
                u8::MAX - fact.lock.rank(),
                proposition.key.clone(),
                FactUnderCheck {
                    id: id.as_str().to_owned(),
                    key: proposition.key.clone(),
                    text: proposition.text.clone(),
                    visibility: fact.visibility,
                },
            ))
        })
        .collect();
    facts.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    facts.truncate(MAX_FACT_CHECKS);
    facts.into_iter().map(|(_, _, fact)| fact).collect()
}

fn checkable_rules(projection: &Projection, ctx: &TurnContext) -> Vec<(String, String, String)> {
    let mut rules: Vec<(String, String, String)> = projection
        .rules
        .values()
        .filter(|rule| rule.is_active_at(ctx.at))
        .map(|rule| {
            (
                rule.id.as_str().to_owned(),
                rule.text.clone(),
                rule.boundaries.join("；"),
            )
        })
        .collect();
    rules.sort();
    rules.truncate(MAX_RULE_CHECKS);
    rules
}

/// 某主体对本场景相关命题知道哪些。
fn known_keys(
    projection: &Projection,
    subject: &SubjectId,
    relevant: &[String],
    scene_index: u64,
) -> String {
    let known = if_views::knowledge_of(projection, subject, scene_index);
    let mut keys: Vec<&str> = projection
        .propositions
        .iter()
        .filter(|(id, _)| known.contains(*id))
        .map(|(_, proposition)| proposition.key.as_str())
        .filter(|key| relevant.iter().any(|wanted| wanted == key))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    if keys.is_empty() {
        "（本场景的相关事实里，他一条都不知道）".to_owned()
    } else {
        keys.join("、")
    }
}

/// 本场景尚未批准揭示的秘密。返回 `(命题 ID, 命题文本)`——ID 做问题键的后缀，
/// 文本只进问题本身，绝不进否决理由。
fn unrevealed_secrets(
    projection: &Projection,
    plan: &ScenePlan,
    scene_index: u64,
) -> Vec<(String, String)> {
    if_views::secret_facts(projection, scene_index)
        .into_iter()
        .filter(|id| {
            !plan.reveal_allowed.iter().any(|target| {
                matches!(target, RevealTarget::Fact { prop } if prop == *id)
            })
        })
        .map(|id| (id.as_str().to_owned(), projection.propositions.get(id)))
        .filter_map(|(id, proposition)| proposition.map(|p| (id, p.text.clone())))
        .collect()
}

#[cfg(test)]
mod tests;
