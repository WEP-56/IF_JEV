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
