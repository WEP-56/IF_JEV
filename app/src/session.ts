/**
 * 会话的创建与恢复（docs/10 §3）。
 *
 * 字段与 `crates/if-app/src/session.rs`、`world_worker.rs`、`seed.rs` 的 serde 输出一一对应。
 *
 * **一个世界可以有多个会话**（世界资产与会话是两个对象，docs/10 §1）。所以「选世界」之后
 * 还要问一句「继续哪一个」——`listSessions` 就是为这一问存在的。
 */
import { tauriInvoke } from './ipc';
import type { Projection } from './projection';

/** Rust `if_app_lib::world_worker::WorldSnapshot`。 */
export interface WorldSnapshot {
  path: string;
  label: string;
  active_line_id: string;
  head_seq: number;
  event_count: number;
  projection: Projection;
  pending_if?: IfRulingCard | null;
}

/** Rust `if_domain::turn::IfRulingCard`。裁定卡的生命周期在后端已经跑通，UI 还没接。 */
export interface IfRulingCard {
  turn: string;
  status: 'pending' | 'confirmed' | 'cancelled';
  injection: {
    input: string;
    kind: string;
    core: string;
    time_anchor: string;
    scope: string;
    lock: string;
    digestion?: string | null;
    non_commitments: string[];
  };
  warnings: string[];
  conflicts?: { event: string; existing_core: string; lock: string; reason: string }[];
}

/** Rust `if_app_lib::seed::SeedReport`。新建会话时后端给的一份「这次播种都做了什么」。 */
export interface SeedReport {
  world_name: string;
  subjects: number;
  lore: number;
  lore_constant: number;
  lore_unreachable: number;
  lore_disabled: number;
  opening?: string | null;
  alternate_openings: number;
  notes: string[];
}

/** Rust `if_store::library::SessionRef`。 */
export interface SessionRef {
  id: string;
  asset_id: string;
  label: string;
  world_file: string;
  created_at: number;
}

/** Rust `if_app_lib::session::SessionView`。 */
export interface SessionView {
  snapshot: WorldSnapshot;
  /** 会话在世界库里的 ID。删除与重新打开都用它。 */
  session_id: string;
  asset_id: string;
  asset_name: string;
  genre: string;
  opening?: string | null;
  seed?: SeedReport | null;
}

/** 选一个世界资产新建会话。`label` 省略时用资产名。 */
export async function createSession(worldId: string, label?: string): Promise<SessionView> {
  return tauriInvoke<SessionView>('create_session', { worldId, label: label ?? null });
}

/** 某个世界已有的会话。 */
export async function listSessions(worldId: string): Promise<SessionRef[]> {
  return tauriInvoke<SessionRef[]>('list_sessions', { worldId });
}

/**
 * 世界库里**全部**会话，最近的在前。
 *
 * 启动时靠它把会话列表装回侧栏——只列「本次运行里建过的」等于重启后就找不到了。
 * 返回的是引用（不是投影）：投影要打开 `.ifworld` 才有，一次只开一个，所以点开才载入。
 */
export async function listAllSessions(): Promise<SessionRef[]> {
  return tauriInvoke<SessionRef[]>('list_all_sessions');
}

/** 删除一条会话：移出世界库并删掉它的世界文件。返回剩下的会话。 */
export async function deleteSession(sessionId: string): Promise<SessionRef[]> {
  return tauriInvoke<SessionRef[]>('delete_session', { sessionId });
}

/** 恢复已有会话：重启之后回到上一次的进度。 */
export async function openSession(sessionId: string): Promise<SessionView> {
  return tauriInvoke<SessionView>('open_session', { sessionId });
}

/* ---------- 回合推演（Rust `if-app-lib::turn_runner`） ---------- */

/** Rust `if_app_lib::turn_runner::SceneView`。视角与在场存的是**名字**，不是 ID。 */
export interface TurnScene {
  id: string;
  goal: string;
  time_span: string;
  pov: string;
  present: string[];
  required_beats: string[];
  stop_condition: string;
  /** 引擎注入的硬约束说明（比如受保护故事线的禁止项）。 */
  injections: string[];
  forbidden_resolutions: string[];
}

/** Rust `if_app_lib::turn_runner::TurnReport`：一次回合的产物。 */
export interface TurnReport {
  /**
   * 世界是否真的推进了。`false` = 没选出场景（候选全被否决，或候选场景全被硬否决）——
   * 这时 `beats` 是空的，界面要如实说「没有推进」，不能假装写了正文。
   */
  advanced: boolean;
  /** 场景是否走到收束（停止条件达成）。 */
  completed: boolean;
  scene?: TurnScene | null;
  /** 已放行的节拍，按放行顺序。正文就是它们拼起来的。 */
  beats: { index: number; text: string }[];
  /** 被拦下来的节拍。**要说出来**——被静默丢掉的正文是「我以为我写的还在」的源头。 */
  blocked: { index: number; reasons: string[] }[];
  tasks: { task: string; rounds: number }[];
  warnings: string[];
  /** 本回合写进事件日志的事件 ID。 */
  committed: string[];
  snapshot: WorldSnapshot;
}

/**
 * 推进一步：跑一个完整的 IF 回合（T-impact → T-scenes → T-plan → T-render → T-extract）。
 *
 * 用户输入取自**最近一张已确认、尚未演绎的裁定卡**，所以正常的用法是
 * 「确认并锁定」之后紧接着调它。它跑得很久（若干次真实模型调用），期间可以 `cancelTurn`。
 */
export async function runTurn(): Promise<TurnReport> {
  return tauriInvoke<TurnReport>('run_turn');
}

/** 中止正在跑的回合。它只置一个原子位，所以立刻生效（不需要等当前回合跑完）。 */
export async function cancelTurn(): Promise<void> {
  return tauriInvoke<void>('cancel_turn');
}
