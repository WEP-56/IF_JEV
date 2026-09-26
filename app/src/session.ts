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

/** 恢复已有会话：重启之后回到上一次的进度。 */
export async function openSession(sessionId: string): Promise<SessionView> {
  return tauriInvoke<SessionView>('open_session', { sessionId });
}
