//! 回合驱动：把五个任务的产物喂进 [`crate::turn`] 的两段驱动。
//!
//! ```text
//! T-impact    →  turn::open_with()          第 6 步：候选
//! T-scenes    →  ↑ 的钩子（裁决之后）        第 8 步：场景候选
//! （引擎）     →  ↑ 的后半段                 第 7 / 9 步：分层裁决 + 导演选择
//! T-plan      →  （夹在中间）                第 10 步：给选中的场景写计划
//! T-render    →  turn::resolve_with()       第 11 步：正文切节拍 → 逐节拍放行
//! T-extract   →  ↑ 的钩子（放行之后）        第 12 步：只回收**已放行**的节拍
//! （引擎）     →  ↑ 的后半段                 第 13 步：对账 → 事件草稿
//! ```
//!
//! 两段驱动的顺序是 [`crate::turn`] 的设计，这里只是按它把任务插进去。
//! 两处钩子（[`turn::open_with`] / [`turn::resolve_with`]）都不是「顺手加的开关」：
//!
//! - **T-scenes 必须看见裁决结果**——不知道哪些候选真的会发生，就提不出「能演什么」；
//! - **T-extract 必须看见放行结果**——从被拦下来的节拍里回收，
//!   等于把一件没人看见的事写进历史。
//!
//! 拿不到提议就如实失败，**不假装世界推了一步**。两处例外，都降级成警告（docs/06 §9）：
//!
//! - T-extract 空手而归（这一场确实什么都没改）——它由
//!   [`TaskKind::allows_empty`](super::TaskKind::allows_empty) 判定，不算失败；
//! - T-extract 因为上游报错没跑成——对账退回「只按预演提交」，但要留下一句能看见的话。

use std::sync::atomic::AtomicBool;

use if_agent::{AgentEvent, Provider};
use if_domain::id::EventId;
use if_domain::narrative::{Beat, ScenePlan, Timestamp};
use if_domain::projection::Projection;
use if_domain::rule::WorldSettings;
use if_domain::turn::Candidate;
use if_domain::value::WorldTime;
use if_judge::Judge;

use crate::commit::ObservedChange;
use crate::context::TurnContext;
use crate::scenes::SceneProposal;
use crate::turn::{self, Opening, OpenRequest, ResolveRequest, SceneOutcome};

use super::host::{TaskKind, TaskOutcome};
use super::render::Rendered;
use super::{extract as t_extract, impact as t_impact, plan as t_plan, render as t_render,
            scenes as t_scenes, TaskError};

/// 一次 IF 回合要的东西。
///
/// `first_seq` 由调用方给：事件 ID 是**预分配**的，发号游标住在存储层
/// （`Store::next_seq()`，见 [`crate::commit::DraftCursor`] 的分配契约）。
#[derive(Debug)]
pub struct TurnRequest<'a> {
    pub projection: &'a Projection,
    pub settings: &'a WorldSettings,
    pub ctx: &'a TurnContext,
    /// 用户这一轮写下的 IF 原话（继续回合为 `None`）。
    pub input: Option<String>,
    /// 触发本回合的事件（锁定的 IF），供 [`crate::candidates`] 记因果。
    pub source_event: Option<EventId>,
    /// `Store::next_seq()`。
    pub first_seq: u64,
    /// 节拍上屏的现实时刻。**不进投影**（docs/02 §10），所以只有调用方能给。
    pub displayed_at: Option<Timestamp>,
    /// 本回合推进到的世界时间。`None` = 不推进。
    pub time: Option<WorldTime>,
}

/// 一次回合的产出。
#[derive(Debug)]
pub struct TurnOutcome {
    pub opening: Opening,
    /// 没有场景可演时为 `None`（候选一个都没放行，或候选场景全被硬否决）。
    pub scene: Option<SceneOutcome>,
    /// 任务名 → 模型轮数，供 [`if_domain::turn::TurnRecord`] 记录（docs/03 §3）。
    pub tasks: Vec<(String, u32)>,
    pub warnings: Vec<String>,
}

impl TurnOutcome {
    /// 被抽中的那条场景提议。
    pub fn scene_proposal(&self) -> Option<&SceneProposal> {
        self.opening.scene()
    }

    /// 这一回合是否真的写出了正文。
    pub fn has_text(&self) -> bool {
        self.scene
            .as_ref()
            .is_some_and(|scene| !scene.beats.admitted.is_empty())
    }
}

/// 回合失败：要么是任务没交上提议，要么是编排编不下去。
#[derive(Debug, thiserror::Error)]
pub enum TurnFailure {
    #[error(transparent)]
    Task(#[from] TaskError),
    #[error(transparent)]
    Pipeline(#[from] crate::PipelineError),
}

/// 五个任务用的两个模型槽位（docs/11 §6）。
///
/// 分开是因为职责不同：**结构模型**做解析 / 候选 / 计划 / 回收（要求的是稳定与结构化），
/// **叙事模型**只写正文（要求的是文笔）。它们可以指向同一个模型，
/// 但「可以相同」不等于「必须相同」——把这件事塞进一个 `provider` 参数，
/// 就等于让调用方没法只说一个。
#[derive(Clone, Copy)]
pub struct TurnProviders<'a> {
    pub structure: &'a dyn Provider,
    pub narrative: &'a dyn Provider,
}

impl std::fmt::Debug for TurnProviders<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnProviders").finish_non_exhaustive()
    }
}

impl<'a> TurnProviders<'a> {
    pub fn new(structure: &'a dyn Provider, narrative: &'a dyn Provider) -> Self {
        Self {
            structure,
            narrative,
        }
    }

    /// 只有一个模型时让它兼任两个槽位（设置里两个槽位指向同一个模型的常见情形）。
    pub fn single(provider: &'a dyn Provider) -> Self {
        Self {
            structure: provider,
            narrative: provider,
        }
    }
}

/// 跑一个完整的 IF 回合。`cancel` 被置位时立刻中止，已写出的东西由调用方决定怎么处理。
pub fn run_if_turn(
    providers: TurnProviders<'_>,
    judge: &dyn Judge,
    request: TurnRequest<'_>,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<TurnOutcome, TurnFailure> {
    let provider = providers.structure;
    let projection = request.projection;
    let settings = request.settings;
    let ctx = request.ctx;

    let mut warnings: Vec<String> = Vec::new();
    let mut tasks: Vec<(String, u32)> = Vec::new();

    // ---- 第 6 步：T-impact
    let impact_task = t_impact::run(provider, projection, ctx, request.input.as_deref(), cancel, emit)?;
    warnings.extend(impact_task.workspace.warnings.iter().cloned());
    tasks.push((TaskKind::Impact.as_str().to_owned(), impact_task.rounds));

    // ---- 第 6–9 步：分层裁决 → T-scenes → 导演选择
    let mut scenes_task: Option<TaskOutcome> = None;
    let mut scenes_error: Option<TaskError> = None;
    let opening = turn::open_with(
        judge,
        OpenRequest {
            projection,
            settings,
            ctx,
            candidates: impact_task.workspace.candidates,
            scenes: Vec::new(),
            source_event: request.source_event.clone(),
            triggered: Vec::new(),
            temperature: None,
            salt: String::new(),
        },
        |impact| {
            let accepted: Vec<Candidate> = impact.accepted().into_iter().cloned().collect();
            // 场景 ID 从「世界里已经有几场」往后发——它进投影，重号就是覆盖上一场。
            match t_scenes::run(provider, projection, ctx, &accepted, ctx.scene_index, cancel, emit) {
                Ok(outcome) => {
                    let scenes = outcome.workspace.scenes.clone();
                    scenes_task = Some(outcome);
                    scenes
                }
                Err(error) => {
                    scenes_error = Some(error);
                    Vec::new()
                }
            }
        },
    )?;
    if let Some(error) = scenes_error {
        return Err(error.into());
    }
    if let Some(outcome) = scenes_task {
        warnings.extend(outcome.workspace.warnings.iter().cloned());
        tasks.push((TaskKind::Scenes.as_str().to_owned(), outcome.rounds));
    }
    warnings.extend(opening.warnings.iter().cloned());

    let Some(scene) = opening.scene().cloned() else {
        warnings.push(
            "本回合没有选出场景（候选全被否决，或候选场景全被硬否决），世界没有推进".to_owned(),
        );
        return Ok(TurnOutcome {
            opening,
            scene: None,
            tasks,
            warnings,
        });
    };

    let accepted: Vec<Candidate> = opening.impact.accepted().into_iter().cloned().collect();

    // ---- 第 10 步：T-plan（**夹在两段之间**）
    let plan = t_plan::run(provider, projection, ctx, &scene, &accepted, cancel, emit)?;
    tasks.push((TaskKind::Plan.as_str().to_owned(), 1));

    // ---- 第 11 步：T-render → 逐节拍放行（正文走**叙事**槽位）
    let rendered = t_render::run(providers.narrative, projection, settings, ctx, &plan, cancel, emit)?;
    warnings.extend(rendered.warnings.iter().cloned());
    tasks.push((t_render::TASK.to_owned(), 1));

    // ---- 第 11–13 步：放行 → 回收 → 对账 → 事件草稿
    let mut extract_error: Option<TaskError> = None;
    let mut extract_rounds = 0u32;
    let scene_id = scene.id.clone();
    let outcome = turn::resolve_with(
        judge,
        ResolveRequest {
            projection,
            settings,
            ctx,
            opening: &opening,
            scene: scene_id,
            plan,
            beats: rendered.beats,
            observed: Vec::new(),
            new_propositions: Vec::new(),
            tendencies: Vec::new(),
            first_seq: request.first_seq,
            displayed_at: request.displayed_at,
            time: request.time,
        },
        |round, _| match t_extract::run(provider, projection, ctx, &round.admitted, cancel, emit) {
            Ok((observed, rounds)) => {
                extract_rounds = rounds;
                Ok(observed)
            }
            Err(error) => {
                extract_error = Some(error);
                Ok(Vec::new())
            }
        },
    )?;
    tasks.push((TaskKind::Extract.as_str().to_owned(), extract_rounds));

    if let Some(error) = extract_error {
        // 取消是真的中止；其余降级成「没回收到东西」，但要看得见。
        if matches!(error, TaskError::Cancelled { .. }) {
            return Err(error.into());
        }
        warnings.push(format!("正文回收没有完成：{error}；本回合只按预演对账"));
    }
    warnings.extend(outcome.warnings.iter().cloned());

    Ok(TurnOutcome {
        opening,
        scene: Some(outcome),
        tasks,
        warnings,
    })
}

/// 只回收**已经放行**的节拍（[`Beat`]），返回 `(观察, 模型轮数)`。
pub fn extract_observations(
    provider: &dyn Provider,
    projection: &Projection,
    ctx: &TurnContext,
    displayed: &[Beat],
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<(Vec<ObservedChange>, u32), TaskError> {
    t_extract::run(provider, projection, ctx, displayed, cancel, emit)
}

/// T-render 的独立入口：调用方想自己管节拍放行时用得上（docs/04 §4.7 的背压侧）。
pub fn render_text(
    provider: &dyn Provider,
    projection: &Projection,
    settings: &WorldSettings,
    ctx: &TurnContext,
    plan: &ScenePlan,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(AgentEvent),
) -> Result<Rendered, TaskError> {
    t_render::run(provider, projection, settings, ctx, plan, cancel, emit)
}
