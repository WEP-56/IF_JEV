/**
 * 世界库（`library.db`）的前端入口（docs/10 §2）。
 *
 * 字段与 `crates/if-app/src/library.rs` 的 serde 输出一一对应（snake_case）。
 *
 * **列表与详情是两个形状**：列表只要摘要（名字、题材、条目数、来源数、会话数），
 * 详情才带 payload 与全部条目——世界库里放几百条设定时，列表页不该把它们全读出来。
 * 详情里的 `payload` 就是当初导入出来的 `ImportedWorld`，所以世界视图能原样重建。
 */
import { tauriInvoke } from './ipc';
import { toWorldAsset, type ImportedWorld } from './import';
import type { LibraryAsset, WorldAsset } from './types';

/** Rust `if_store::library::SourceRecord`。 */
export interface LibrarySource {
  source_key: string;
  kind: string;
  file?: string | null;
  format: string;
  spec_version?: string | null;
  version?: string | null;
  content_hash: string;
  imported_at: number;
  raw?: string | null;
}

/** Rust `if_store::library::LoreRecord`。`payload` 是 `ImportedLore` 的 JSON。 */
export interface LibraryLore {
  source_key: string;
  uid?: string | null;
  ordinal: number;
  title: string;
  section: string;
  payload: unknown;
}

/** Rust `if_store::library::SessionRef`。 */
export interface LibrarySession {
  id: string;
  asset_id: string;
  label: string;
  world_file: string;
  created_at: number;
}

/** Rust `if_store::library::WorldAsset`。 */
export interface StoredAsset {
  id: string;
  name: string;
  genre: string;
  summary: string;
  origin: 'imported' | 'written';
  revision: number;
  created_at: number;
  updated_at: number;
  payload: ImportedWorld;
}

/** Rust `if_app_lib::library::AssetDetail`。 */
export interface LibraryDetail {
  asset: StoredAsset;
  sources: LibrarySource[];
  lore: LibraryLore[];
  sessions: LibrarySession[];
}

/** Rust `if_app_lib::library::AssetChange`：写入后返回详情与刷新过的列表。 */
export interface LibraryChange {
  detail: LibraryDetail;
  assets: AssetSummaryDto[];
}

/** Rust `if_store::library::AssetSummary`（序列化后是 snake_case）。 */
export interface AssetSummaryDto {
  id: string;
  name: string;
  genre: string;
  summary: string;
  origin: 'imported' | 'written';
  revision: number;
  created_at: number;
  updated_at: number;
  lore_count: number;
  source_count: number;
  session_count: number;
  source_files: string[];
}

const ORIGIN_LABEL: Record<LibraryAsset['origin'], string> = {
  imported: '已导入',
  written: '手动撰写',
  demo: '示例资产',
};

/** 资产来源的可读标签。 */
export function originLabel(origin: LibraryAsset['origin']): string {
  return ORIGIN_LABEL[origin];
}

/** 毫秒时间戳 → 列表页用的短标签。 */
export function formatStamp(ms: number): string {
  if (!ms) return '';
  const at = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

export function toLibraryAsset(dto: AssetSummaryDto): LibraryAsset {
  return {
    id: dto.id,
    name: dto.name,
    genre: dto.genre,
    summary: dto.summary,
    origin: dto.origin,
    revision: dto.revision,
    loreCount: dto.lore_count,
    sourceCount: dto.source_count,
    sessionCount: dto.session_count,
    sourceFiles: dto.source_files,
    updated: formatStamp(dto.updated_at),
  };
}

export function toLibraryAssets(dtos: AssetSummaryDto[]): LibraryAsset[] {
  return dtos.map(toLibraryAsset);
}

/** 详情 → 前端的世界视图（新建会话要用的那个形状）。 */
export function toWorldAssetFromDetail(detail: LibraryDetail): WorldAsset {
  return toWorldAsset(detail.asset.payload, detail.asset.id, formatStamp(detail.asset.updated_at));
}

/** 浏览器预览用：把演示资产折成摘要形状，好让同一个列表组件两种模式都能渲染。 */
export function demoLibraryAssets(demos: WorldAsset[]): LibraryAsset[] {
  return demos.map((world) => ({
    id: world.id,
    name: world.name,
    genre: world.genre,
    summary: world.summary,
    origin: 'demo' as const,
    revision: 1,
    loreCount: 0,
    sourceCount: 0,
    sessionCount: 0,
    sourceFiles: [],
    updated: world.updated,
  }));
}

/** 演示资产按 id 反查，供浏览器预览下「选择世界」用。 */
export function demoWorldById(demos: WorldAsset[], id: string): WorldAsset | undefined {
  return demos.find((world) => world.id === id);
}

/* ---------------------------------------------------------------- IPC */

export async function listWorlds(): Promise<LibraryAsset[]> {
  return toLibraryAssets(await tauriInvoke<AssetSummaryDto[]>('list_worlds'));
}

export async function loadWorldAsset(id: string): Promise<LibraryDetail> {
  return tauriInvoke<LibraryDetail>('get_world_asset', { id });
}

/** 删除资产，返回刷新过的列表。被会话引用时这里会抛出带原因的错。 */
export async function deleteWorldAsset(id: string): Promise<LibraryAsset[]> {
  return toLibraryAssets(await tauriInvoke<AssetSummaryDto[]>('delete_world_asset', { id }));
}

/** 从文件导入并落库。同一个来源再次导入会替换该来源的条目，而不是追加。 */
export async function importFileToLibrary(
  fileName: string,
  dataBase64: string,
): Promise<LibraryChange> {
  return tauriInvoke<LibraryChange>('import_world_file_to_library', { fileName, dataBase64 });
}

export async function createWrittenWorld(
  name: string,
  genre?: string,
  summary?: string,
): Promise<LibraryChange> {
  return tauriInvoke<LibraryChange>('create_written_world', { name, genre, summary });
}
