//! 回合执行：把 `if-pipeline` 的任务层接到一个真实的世界文件上（docs/04 §1、docs/05 §4）。
//!
//! 这一层是「司机」。`if-pipeline` 会**编**一个回合但不会**跑**它——它不认存储、不认设置，
//! 只收提议、问 Jev、出补丁。把「从哪个世界读、用什么模型、写到哪去」凑起来的是这里。
//!
//! 顺序（与 [`if_pipeline::turn`] 的两段驱动一一对应）：
//!
//! ```text
//! 读投影 → 组装 TurnContext → run_if_turn（五个任务） → append_batch 事件 → 存 TurnRecord
//! ```
//!
//! 三条刻意的取舍：
//!
//! 1. **事件 ID 先算号再写**（`Store::next_seq()` → `DraftCursor` → `append_batch`）：
//!    草稿里的号必须与实际写出来的号一致，这条契约由 if-store 与 if-app 两侧的测试钉住。
//! 2. **失败就不留痕**：任务拿不到提议时整段返回 `Err`，不写事件也不存回合记录。
//!    留一半的回合记录会让「这个回合到底推没推进」变成一件要读代码才知道的事。
//! 3. **不推进世界时间**：v1 的时间推进由机制结算负责，回合本身只写这一场戏。
//!    `TurnRequest.time` 因此是 `None`——这不是忘了填，是还没到那一步。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use if_agent::{build_provider, AgentEvent, ProviderSettings};
use if_domain::id::{SubjectId, TurnId};
use if_domain::narrative::ScenePlan;
use if_domain::projection::Projection;
use if_domain::turn::{IfCardStatus, TurnKind};
use if_judge::{JevJudge, Judge, LlmJudge};
use if_pipeline::context::{activate_for_turn, TurnContext, subject_name};
use if_pipeline::tasks::{run_if_turn, TurnFailure, TurnProviders, TurnRequest, TurnOutcome};
use if_policy::Strictness;
use if_store::Store;
use serde::Serialize;

use crate::settings::JevSettings;
use crate::world_worker::{snapshot, WorldSnapshot};

/// 跑一个回合需要的全部外部配置。
///
/// 都是**可发送的纯数据**：provider 与 judge 在工作线程上现造，
/// 因为 `Box<dyn Provider>` 既不是 `Send` 也不该跨线程复用（连接池属于线程）。
pub struct TurnConfig {
    /// 结构模型：候选 / 场景 / 计划 / 回收（docs/11 §6）。
    pub structure: ProviderSettings,
    /// 叙事模型：正文。模型名为空时退回结构模型。
    pub narrative: ProviderSettings,
    pub jev: JevSettings,
    /// 空 = 没有 Jev 密钥 → 用结构模型充当裁判（D11）。
    pub jev_key: String,
    /// 一致性严格度 0–100。
    pub strictness: u8,
}

/// 一次回合的结果，直接序列化给前端。
#[derive(Debug, Clone, Serialize)]
pub struct TurnReport {
    /// 世界是否真的推进了。`false` = 没有选出场景（候选全被否决，或候选场景全被硬否决）。
    pub advanced: bool,
    /// 场景是否走到收束（停止条件达成）。
    pub completed: bool,
    pub scene: Option<SceneView>,
    /// 已放行的节拍，按放行顺序。正文就是它们拼起来的。
    pub beats: Vec<BeatView>,
    /// 被拦下来的节拍。**要说出来**——被静默丢掉的正文是「我以为我写的还在」的源头。
    pub blocked: Vec<BlockedView>,
    /// 任务名 → 模型轮数（docs/03 §3）。
    pub tasks: Vec<TaskView>,
    pub warnings: Vec<String>,
    /// 本回合写进事件日志的事件 ID。
    pub committed: Vec<String>,
    pub snapshot: WorldSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct SceneView {
    pub id: String,
    pub goal: String,
    pub time_span: String,
    /// 视角人物。存名字：给用户看的是名字，ID 只在日志里有意义。
    pub pov: String,
    pub present: Vec<String>,
    pub required_beats: Vec<String>,
    pub stop_condition: String,
    /// 引擎注入的硬约束说明（docs/04 §2.1）。
    pub injections: Vec<String>,
    pub forbidden_resolutions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BeatView {
    pub index: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockedView {
    pub index: u32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub task: String,
    pub rounds: u32,
}

/// 跑一个 IF 回合并把它落盘。
///
/// `input` 为 `None` 时从**最近一张已确认、尚未演绎的裁定卡**取用户原话——
/// 「确认之后推进一步」走的就是这条。取不到就按「继续回合」跑（没有用户输入）。
pub fn run(
    store: &mut Store,
    path: &Path,
    config: &TurnConfig,
    cancel: &AtomicBool,
) -> Result<TurnReport, String> {
    if config.structure.model.trim().is_empty() {
        return Err("结构模型尚未配置，无法推演回合".into());
    }
    let structure = build_provider(config.structure.clone());
    // 叙事槽位没填模型时退回结构模型：设置里两个槽位指向同一个模型是允许的，
    // 而「叙事模型空着」应当降级成同一句话，不该是一个只有开发者看得懂的 provider 报错。
    let narrative_settings = if config.narrative.model.trim().is_empty() {
        config.structure.clone()
    } else {
        config.narrative.clone()
    };
    let narrative = build_provider(narrative_settings);
    let judge = build_judge(config);
    run_with(
        store,
        path,
        structure.as_ref(),
        narrative.as_ref(),
        judge.as_ref(),
        config.strictness,
        cancel,
    )
}

/// 真正干活的那一层：只认 `Provider` 与 `Judge`，与「它们从哪来」解耦。
///
/// 拆出来是为了能测**整条路径**（读投影 → 建上下文 → 五个任务 → 写事件 → 存回合记录）：
/// 只测 `run_if_turn` 覆盖不到「怎么把投影变成一个回合」。测试用
/// `if_agent::provider::scripted::ScriptedProvider` + `if_judge::StubJudge`，不联网、不花额度。
pub(crate) fn run_with(
    store: &mut Store,
    path: &Path,
    structure: &dyn if_agent::Provider,
    narrative: &dyn if_agent::Provider,
    judge: &dyn Judge,
    strictness: u8,
    cancel: &AtomicBool,
) -> Result<TurnReport, String> {
    let line = store
        .active_line()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "当前没有活跃世界线".to_owned())?;
    if store
        .turns_of_line(&line)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|turn| {
            turn.ruling_card
                .as_ref()
                .is_some_and(|card| card.status == IfCardStatus::Pending)
        })
    {
        return Err("已有待确认的 IF 裁定卡，请先确认或取消".into());
    }

    let projection = store.load_projection(&line).map_err(|e| e.to_string())?;
    let settings = projection
        .settings
        .clone()
        .ok_or_else(|| "文件不是有效的 IF 世界：缺少 world_created 事件".to_owned())?;

    let (input, source_event) = pending_if(store, &line)?;
    let ctx = context(&projection, &line, &settings, input.clone(), store, strictness)?;
    let first_seq = store.next_seq().map_err(|e| e.to_string())?;

    let mut emit = |_event: AgentEvent| {};
    let outcome = run_if_turn(
        TurnProviders::new(structure, narrative),
        judge,
        TurnRequest {
            projection: &projection,
            settings: &settings,
            ctx: &ctx,
            input,
            source_event,
            first_seq,
            displayed_at: Some(now_secs()),
            // v1 不推进世界时间：机制结算还没接，凭空往前拨一天是编故事不是模拟。
            time: None,
        },
        cancel,
        &mut emit,
    )
    .map_err(describe)?;

    if cancel.load(Ordering::Relaxed) {
        return Err("回合已取消".into());
    }

    persist(store, &projection, &ctx, &outcome, path)
}

/// 落盘：先写事件，再存回合记录。**这个顺序不能反**——
/// 回合记录里的 `committed` 是事件 ID，先存记录的话，写事件失败就留下一条指向空气的记录。
fn persist(
    store: &mut Store,
    projection: &Projection,
    ctx: &TurnContext,
    outcome: &TurnOutcome,
    path: &Path,
) -> Result<TurnReport, String> {
    let committed = match &outcome.scene {
        Some(scene) => store
            .append_batch(scene.drafts.clone())
            .map_err(|e| format!("写入回合事件失败：{e}"))?
            .into_iter()
            .map(|event| event.id)
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };

    let mut record = if_pipeline::turn::record(
        ctx.turn.clone(),
        ctx.line.clone(),
        TurnKind::If,
        ctx.at,
        ctx.user_input.clone(),
        Some(&outcome.opening),
        outcome.scene.as_ref(),
    );
    record.committed = committed.clone();
    record.tasks = outcome
        .tasks
        .iter()
        .map(|(task, rounds)| if_domain::turn::TaskRecord {
            task: task.clone(),
            rounds: *rounds,
            calls: Vec::new(),
            finish: if_domain::turn::TaskFinish::Completed,
        })
        .collect();
    record.metrics.jev_tokens = 0;
    record.metrics.jev_cost_usd = record.jev_cost();
    store.save_turn(&record).map_err(|e| format!("保存回合记录失败：{e}"))?;

    let snapshot = snapshot(store, path)?;
    Ok(report(outcome, projection, committed, snapshot))
}

/// 最近一张已确认、尚未演绎的 IF 裁定卡 → `(用户原话, 触发事件)`。
///
/// 判据是「最近的回合记录」而不是「最近一张卡」：`run_turn` 自己也会存一条回合记录，
/// 所以演绎过一次之后，最近的那条记录就不再是那张卡了——这正是我们要的幂等性。
fn pending_if(
    store: &Store,
    line: &if_domain::id::WorldLineId,
) -> Result<(Option<String>, Option<if_domain::id::EventId>), String> {
    let turns = store.turns_of_line(line).map_err(|e| e.to_string())?;
    let Some(latest) = turns.last() else {
        return Ok((None, None));
    };
    let Some(card) = latest
        .ruling_card
        .as_ref()
        .filter(|card| card.status == IfCardStatus::Confirmed)
    else {
        return Ok((None, None));
    };
    Ok((
        Some(card.injection.input.clone()),
        card.confirmed_event.clone(),
    ))
}

/// 组装本回合的共享上下文。
///
/// `focus` / `present` 的取法是 v1 的**临时口径**（文档还没裁定）：优先取当前镜头与活跃主体，
/// 一个都没有时退回投影里的全部角色。退回是必要的——计划里的视角人物必须在在场名单里，
/// 空名单会让每一次 T-plan 都被同一条领域校验挡下来，而那看起来像「模型不会写计划」。
fn context(
    projection: &Projection,
    line: &if_domain::id::WorldLineId,
    settings: &if_domain::rule::WorldSettings,
    input: Option<String>,
    store: &Store,
    strictness: u8,
) -> Result<TurnContext, String> {
    let scene_index = projection.scenes.len() as u64;
    let turn = TurnId::numbered(store.event_count().map_err(|e| e.to_string())? + 1);
    let subjects = focus_and_present(projection);
    let mut ctx = TurnContext::new(turn, line.clone(), TurnKind::If, projection.world_time, scene_index)
        .focus(subjects.clone())
        .present(subjects)
        .narrative_order(projection.narrative_order)
        .strictness(strictness_from_percent(strictness));
    ctx.user_input = input.clone();
    // 世界书激活（docs/08 §5）的扫描范围：用户这一轮写下的原话。
    // 场景计划要等 T-plan 才存在，所以首轮扫描只有输入——这不是遗漏，是顺序。
    let scan = input.clone().unwrap_or_default();
    ctx.lore = activate_for_turn(projection, settings, &ctx, &scan);
    Ok(ctx)
}

fn focus_and_present(projection: &Projection) -> Vec<SubjectId> {
    let mut focused: Vec<SubjectId> = projection
        .subjects
        .values()
        .filter(|subject| {
            matches!(
                subject.tier,
                if_domain::subject::Tier::Foreground | if_domain::subject::Tier::Active
            )
        })
        .map(|subject| subject.id.clone())
        .collect();
    if focused.is_empty() {
        focused = projection.subjects.keys().cloned().collect();
    }
    focused
}

fn build_judge(config: &TurnConfig) -> Box<dyn Judge> {
    if config.jev_key.trim().is_empty() {
        // D11：没有 Jev 密钥时用结构模型充当裁判。它不是 Jev，但比「判定缺失、
        // 于是所有约束类一律不通过」要好——后者会让每一次回合都原地失败。
        Box::new(LlmJudge::new(build_provider(config.structure.clone())))
    } else {
        Box::new(JevJudge::new(config.jev.config(config.jev_key.clone())))
    }
}

fn report(
    outcome: &TurnOutcome,
    projection: &Projection,
    committed: Vec<if_domain::id::EventId>,
    snapshot: WorldSnapshot,
) -> TurnReport {
    let scene = outcome.scene.as_ref();
    let scene_view = outcome.scene_proposal().map(|proposal| SceneView {
        id: proposal.id.to_string(),
        goal: scene.map(|s| s.plan.goal.clone()).unwrap_or_default(),
        time_span: scene.map(|s| s.plan.time_span.clone()).unwrap_or_default(),
        pov: scene
            .map(|s| subject_name(projection, &s.plan.pov))
            .unwrap_or_default(),
        present: scene
            .map(|s| present_names(projection, &s.plan))
            .unwrap_or_default(),
        required_beats: scene
            .map(|s| s.plan.required_beats.clone())
            .unwrap_or_default(),
        stop_condition: scene.map(|s| s.plan.stop_condition.clone()).unwrap_or_default(),
        injections: scene.map(|s| s.injections.clone()).unwrap_or_default(),
        forbidden_resolutions: scene
            .map(|s| s.plan.forbidden_resolutions.clone())
            .unwrap_or_default(),
    });

    let (beats, blocked) = match scene {
        Some(scene) => (
            scene
                .beats
                .admitted
                .iter()
                .map(|beat| BeatView {
                    index: beat.index,
                    text: beat.text.clone(),
                })
                .collect(),
            scene
                .beats
                .blocked
                .iter()
                .map(|(index, blocks)| BlockedView {
                    index: *index,
                    reasons: blocks
                        .iter()
                        .map(|block| format!("{}：{}", block.template, block.reason))
                        .collect(),
                })
                .collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };

    TurnReport {
        advanced: scene.is_some_and(|scene| !scene.beats.admitted.is_empty()),
        completed: scene.is_some_and(|scene| scene.completed()),
        scene: scene_view,
        beats,
        blocked,
        tasks: outcome
            .tasks
            .iter()
            .map(|(task, rounds)| TaskView {
                task: task.clone(),
                rounds: *rounds,
            })
            .collect(),
        warnings: outcome.warnings.clone(),
        committed: committed.iter().map(|id| id.to_string()).collect(),
        snapshot,
    }
}

fn present_names(projection: &Projection, plan: &ScenePlan) -> Vec<String> {
    plan.present
        .iter()
        .map(|subject| subject_name(projection, subject))
        .collect()
}

fn describe(failure: TurnFailure) -> String {
    match failure {
        TurnFailure::Task(error) => format!("回合失败（{}）：{error}", error.task()),
        TurnFailure::Pipeline(error) => format!("回合失败：{error}"),
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// 严格度 0–100 → 策略用的倍率（[`Strictness`] 的区间是 `[0.5, 2.0]`）。
///
/// 文档（D10）只说「线性缩放」，没说端点，所以这里必须自己定一个，并且让它**只有一处**：
/// **设置里的默认值就是中性点**（倍率 1.0，即文档里的初始阈值原样生效）。
/// 低于它按比例放宽到 `MIN`，高于它按比例收紧到 `MAX`。
///
/// 为什么不直接把 0–100 铺满 `[0.5, 2.0]`：那样默认的 70% 会落在 1.55，
/// 于是「默认设置」本身就悄悄把 `q.key.equivalent`（0.8）顶到 1.12 —— 比概率的上限还高，
/// 那一条判定从第一次回合起就永远不可能通过。默认值不该是一个隐藏的行为改变。
pub fn strictness_from_percent(percent: u8) -> Strictness {
    let percent = f64::from(percent.min(100)) / 100.0;
    let neutral = f64::from(crate::settings::DEFAULT_STRICTNESS) / 100.0;
    let factor = if percent <= neutral {
        Strictness::MIN + (percent / neutral) * (1.0 - Strictness::MIN)
    } else {
        1.0 + ((percent - neutral) / (1.0 - neutral)) * (Strictness::MAX - 1.0)
    };
    Strictness::new(factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strictness_is_neutral_at_the_default_and_spans_the_range() {
        // 默认值 = 中性：文档里的初始阈值原样生效。
        assert!(strictness_from_percent(crate::settings::DEFAULT_STRICTNESS).is_default());
        assert!(strictness_from_percent(0).factor() < 1.0, "调低应当放宽");
        assert!((strictness_from_percent(0).factor() - Strictness::MIN).abs() < 1e-12);
        assert!(strictness_from_percent(100).factor() > 1.0, "调高应当收紧");
        assert!((strictness_from_percent(100).factor() - Strictness::MAX).abs() < 1e-12);
        // 越界的输入被夹住，不会跑出 Strictness 自己的区间。
        assert!((strictness_from_percent(200).factor() - Strictness::MAX).abs() < 1e-12);
        // 单调：调得更严不会反而更松。
        let mut previous = 0.0;
        for percent in (0..=100).step_by(5) {
            let factor = strictness_from_percent(percent).factor();
            assert!(factor >= previous - 1e-12, "{percent}% 比前一点更松了");
            previous = factor;
        }
    }

    /// 没有活跃等级的主体时退回全部角色——空名单会让每一次 T-plan 都被
    /// 「视角人物不在在场主体列表里」挡下来。
    #[test]
    fn an_empty_tier_still_yields_the_subjects_it_has() {
        let mut projection = Projection::genesis("wl_main");
        for (id, name) in [("c_a", "甲"), ("c_b", "乙")] {
            projection.subjects.insert(
                SubjectId::new(id),
                if_domain::subject::Subject {
                    id: SubjectId::new(id),
                    kind: if_domain::subject::SubjectKind::Character,
                    name: name.into(),
                    aliases: Vec::new(),
                    profile: String::new(),
                    voice: None,
                    tier: if_domain::subject::Tier::Dormant,
                    shaped: false,
                    created_by: if_domain::id::EventId::new("evt_0001"),
                },
            );
        }
        let names = focus_and_present(&projection);
        assert_eq!(names.len(), 2, "全都在休眠时也要有人可以当视角人物");
    }

    // ---- 整条路径：一张确认过的裁定卡 → 世界真的推进一步 ----
    //
    // 与 `diagnostics::parse_if_with` 拆出来是同一个理由（也是同一次真机教训）：
    // 只测 `run_if_turn` 覆盖不到「怎么把投影变成一个回合」。所以这里用一个**真实的
    // 临时世界文件**，走真机同一条代码路径（读投影 → 建上下文 → 五个任务 → 写事件 →
    // 存回合记录），只有 provider 与 judge 是脚本化的，不联网、不花额度。

    use if_agent::provider::scripted::{ScriptedProvider, ScriptedTurn};
    use if_domain::event::Patch;
    use if_domain::value::WorldTime;
    use if_judge::StubJudge;
    use if_store::EventDraft;
    use serde_json::json;

    fn temp_world_path() -> std::path::PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("if-turn-test-{id}.ifworld"))
    }

    /// 两个角色的最小世界资产——播种只需要角色名，主体 ID 由 `slug` 定（`c_林夏`）。
    fn world_asset() -> crate::importer::ImportedWorld {
        crate::importer::ImportedWorld {
            name: "雨夜长安".into(),
            characters: ["林夏", "顾言"]
                .into_iter()
                .map(|name| crate::importer::ImportedCharacter {
                    name: name.to_owned(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    const BEAT_1: &str = "第一拍：雨落下来。";
    const BEAT_2: &str = "第二拍：她推开门。";
    const BEAT_3: &str = "第三拍：他转身走了。";

    /// 五个任务的脚本，顺序与 `run_if_turn` 一致。
    fn script() -> Vec<ScriptedTurn> {
        vec![
            ScriptedTurn::tool(
                "propose_candidate",
                json!({
                    "subject": "林夏",
                    "content": "林夏当晚去宫门找顾言",
                    "internal": false,
                    "shape": "occurs",
                    "options": [],
                    "depends_on": [],
                    "based_on": [],
                    "affects": []
                }),
            ),
            ScriptedTurn::text("已提交候选。"),
            ScriptedTurn::tool(
                "propose_scene",
                json!({
                    "summary": "雨夜，林夏去找顾言",
                    "threads": [],
                    "resolves": [],
                    "erupts": [],
                    "focus": ["林夏"],
                    "present": ["林夏"]
                }),
            ),
            ScriptedTurn::text("已提交场景候选。"),
            ScriptedTurn::tool(
                "submit_scene_plan",
                json!({
                    "goal": "林夏把话问出口",
                    "pov": "林夏",
                    "focus": ["林夏"],
                    "present": ["林夏"],
                    "time_span": "当晚",
                    "required_beats": [],
                    "stop_condition": "林夏把话说完",
                    "reveal_facts": [],
                    "reveal_lore": [],
                    "proposed_changes": []
                }),
            ),
            ScriptedTurn::text("已提交场景计划。"),
            ScriptedTurn::text(format!("{BEAT_1}<<<BEAT>>>{BEAT_2}<<<BEAT>>>{BEAT_3}")),
            ScriptedTurn::tool(
                "record_observation",
                json!({
                    "prop": "c_lin.mood",
                    "value": false,
                    "internal": false,
                    "text": "她推开门，没有说出来。"
                }),
            ),
            ScriptedTurn::text("已回收。"),
        ]
    }

    /// 与 `if-pipeline` 的测试夹具同口径：约束全过、节拍全干净。
    /// 默认桩给 0.5，而 `q.beat.*` 的方向是**越高越可疑**（`HigherFlags`）——
    /// 不显式压下去，三拍正文一条都放行不了。
    fn clean_judge() -> StubJudge {
        let mut judge = StubJudge::new();
        for template in [
            "q.cand.knowledge_gap",
            "q.scene.resolves_thread",
            "q.beat.violates_fact",
            "q.beat.violates_rule",
            "q.beat.knowledge_leak",
            "q.beat.forbidden_resolution",
            "q.beat.reveals_secret",
            "q.beat.stop_reached",
            "q.observe.leaks_secret",
        ] {
            judge = judge.noul_for_template(template, 0.0);
        }
        for template in ["q.cand.in_character", "q.behavior.occurs"] {
            judge = judge.noul_for_template(template, 1.0);
        }
        judge
    }

    /// 命题要能被 T-extract 认得，所以先落一条进世界——播种只建主体与设定条目，
    /// 命题是 T-parse 的事（`seed` 的文档说明了为什么不在这里猜）。
    fn sow_proposition(store: &mut Store) {
        let line = store.active_line().unwrap().unwrap();
        store
            .append(EventDraft::new(
                line,
                TurnId::numbered(0),
                WorldTime::EPOCH,
                Patch::PropositionCreated(Box::new(if_domain::subject::Proposition {
                    id: if_domain::id::PropositionId::new("p_lin_mood"),
                    key: "c_lin.mood".into(),
                    text: "林夏的心情".into(),
                    subjects: vec![SubjectId::new("c_林夏")],
                    kind: if_domain::subject::PropositionKind::State,
                    value_type: if_domain::subject::ValueType::Bool,
                    internal: true,
                })),
            ))
            .unwrap();
    }

    #[test]
    fn a_confirmed_card_becomes_a_played_turn() {
        let path = temp_world_path();
        let settings = if_domain::rule::WorldSettings {
            seed: 20260927,
            ..Default::default()
        };
        {
            let handle = crate::world_worker::WorldWorker::create(
                path.clone(),
                crate::world_worker::CreateRequest::new("雨夜长安", settings, Some(world_asset())),
            )
            .unwrap();
            // 「确认并锁定」——这是回合的起点，用户的原话就存在这张卡里。
            handle.world.submit_if("IF 林夏当晚去宫门".into()).unwrap();
            handle.world.confirm_if(None).unwrap();
        }

        let mut store = Store::open(&path).unwrap();
        sow_proposition(&mut store);
        let before = store.event_count().unwrap();

        let provider = ScriptedProvider::new(script());
        let judge = clean_judge();
        let cancel = AtomicBool::new(false);
        let report = run_with(
            &mut store,
            &path,
            &provider,
            &provider,
            &judge,
            crate::settings::DEFAULT_STRICTNESS,
            &cancel,
        )
        .expect("干净回合应当跑通");

        // ---- 世界真的动了
        assert!(report.advanced, "三拍都放行了才算推进");
        assert_eq!(report.beats.len(), 3);
        assert_eq!(report.beats[0].text, BEAT_1);
        assert!(!report.committed.is_empty(), "推进必须写事件");
        assert!(report.snapshot.event_count > before);

        // ---- 五个任务按顺序跑过
        let names: Vec<&str> = report.tasks.iter().map(|task| task.task.as_str()).collect();
        assert_eq!(
            names,
            vec!["T-impact", "T-scenes", "T-plan", "T-render", "T-extract"]
        );

        // ---- 场景计划是给人看的：视角人物是名字，不是 `c_林夏`
        let scene = report.scene.as_ref().expect("有场景");
        assert_eq!(scene.pov, "林夏");
        assert_eq!(scene.present, vec!["林夏".to_owned()]);

        // ---- 回合记录存下来了，用户的原话从裁定卡一路带到这里
        let line = store.active_line().unwrap().unwrap();
        let turns = store.turns_of_line(&line).unwrap();
        let last = turns.last().expect("至少有一条回合记录");
        assert_eq!(last.input.as_deref(), Some("IF 林夏当晚去宫门"));
        assert_eq!(last.tasks.len(), 5, "五个任务都要记进回合记录");
        assert_eq!(last.committed.len(), report.committed.len());

        // ---- 投影跟着长：这一场真的成了世界里的一场戏
        let projection = store.load_projection(&line).unwrap();
        assert_eq!(projection.scenes.len(), 1, "这一场必须落进投影");
        assert_eq!(projection.beats.len(), 3, "放行的三拍都要成为已展示的节拍");

        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    /// 裁定卡还挂着的时候不能推演：否则用户看到的是「卡还没确认，世界先动了」。
    #[test]
    fn a_pending_card_blocks_the_turn() {
        let path = temp_world_path();
        let worker = crate::world_worker::WorldWorker::create(
            path.clone(),
            crate::world_worker::CreateRequest::new(
                "雨夜长安",
                if_domain::rule::WorldSettings::default(),
                Some(world_asset()),
            ),
        )
        .unwrap()
        .world;
        worker.submit_if("IF 林夏当晚去宫门".into()).unwrap();
        drop(worker);

        let mut store = Store::open(&path).unwrap();
        let provider = ScriptedProvider::new(script());
        let error = run_with(
            &mut store,
            &path,
            &provider,
            &provider,
            &clean_judge(),
            crate::settings::DEFAULT_STRICTNESS,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.contains("待确认"), "{error}");
        assert_eq!(provider.remaining(), script().len(), "被拦下就不该发问");

        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    /// 回合跑过之后，同一张卡不该被第二次演绎——否则一次确认会推出两场戏。
    #[test]
    fn a_card_is_only_played_once() {
        let path = temp_world_path();
        let settings = if_domain::rule::WorldSettings {
            seed: 20260927,
            ..Default::default()
        };
        {
            let handle = crate::world_worker::WorldWorker::create(
                path.clone(),
                crate::world_worker::CreateRequest::new("雨夜长安", settings, Some(world_asset())),
            )
            .unwrap();
            handle.world.submit_if("IF 林夏当晚去宫门".into()).unwrap();
            handle.world.confirm_if(None).unwrap();
        }
        let mut store = Store::open(&path).unwrap();
        sow_proposition(&mut store);
        let cancel = AtomicBool::new(false);
        let first = ScriptedProvider::new(script());
        run_with(
            &mut store,
            &path,
            &first,
            &first,
            &clean_judge(),
            crate::settings::DEFAULT_STRICTNESS,
            &cancel,
        )
        .unwrap();

        // 第二遍：最近一条回合记录已经是「演绎过的那条」，不再提供 IF 输入，
        // 于是它退化成一次没有输入的继续回合——而不是把同一张卡再演一遍。
        let second = ScriptedProvider::new(script());
        let report = run_with(
            &mut store,
            &path,
            &second,
            &second,
            &clean_judge(),
            crate::settings::DEFAULT_STRICTNESS,
            &cancel,
        )
        .unwrap();
        let line = store.active_line().unwrap().unwrap();
        let turns = store.turns_of_line(&line).unwrap();
        assert_eq!(turns.last().unwrap().input, None, "第二遍不该再拿到那条 IF");

        // 但世界确实又推进了一场（这是「继续回合」，不是重复演绎）
        assert!(report.advanced);
        assert_eq!(store.load_projection(&line).unwrap().scenes.len(), 2);

        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}
