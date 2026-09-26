/**
 * 酒馆导入结果的前端类型与预览映射。
 *
 * 字段与 `crates/if-app/src/importer/model.rs` 的 serde 输出一一对应（snake_case）。
 * 这里只做「忠实呈现 + 预览映射」，不改变导入内容；语义抽取在 Rust 侧后续阶段完成。
 */
import type { Character, World, WorldAsset } from './types';

export type LoreSection = 'world' | 'character' | 'scene';
export type LoreLogic = 'and_any' | 'not_all' | 'not_any' | 'and_all';
export type LoreRole = 'system' | 'user' | 'assistant';

export interface ImportedLore {
  title: string;
  content: string;
  keys: string[];
  secondary_keys: string[];
  constant: boolean;
  enabled: boolean;
  order: number;
  priority?: number | null;
  selective: boolean;
  logic: LoreLogic;
  section: LoreSection;
  depth?: number | null;
  role?: LoreRole | null;
  probability?: number | null;
  use_probability?: boolean | null;
  group?: string | null;
  group_weight?: number | null;
  group_override?: boolean | null;
  sticky?: number | null;
  cooldown?: number | null;
  delay?: number | null;
  scan_depth?: number | null;
  case_sensitive?: boolean | null;
  match_whole_words?: boolean | null;
  use_regex?: boolean | null;
  exclude_recursion?: boolean | null;
  prevent_recursion?: boolean | null;
  delay_until_recursion?: boolean | null;
  character_filter?: unknown;
  vectorized: boolean;
  decorators: string[];
  extensions?: unknown;
  source_uid?: string | null;
  source_dialect: string;
}

export interface ImportedCharacter {
  name: string;
  nickname?: string | null;
  description: string;
  personality: string;
  scenario: string;
  first_message: string;
  alternate_greetings: string[];
  example_messages: string;
  aliases: string[];
  system_prompt: string;
  post_history_instructions: string;
  creator_notes: string;
  group_only_greetings: string[];
  assets: { asset_type: string; uri: string; name: string; ext: string }[];
  source: string[];
  tags: string[];
  creator?: string | null;
  character_version?: string | null;
  creation_date?: number | null;
  modification_date?: number | null;
  extensions?: unknown;
}

export interface ImportedWorld {
  name: string;
  genre: string;
  summary: string;
  source_format: string;
  source_kind: string;
  source_file?: string | null;
  spec_version?: string | null;
  characters: ImportedCharacter[];
  lore: ImportedLore[];
  lore_meta: {
    name: string;
    description: string;
    scan_depth?: number | null;
    token_budget?: number | null;
    recursive_scanning?: boolean | null;
  };
  macros: string[];
  source_fields: string[];
  warnings: string[];
}

/** 来源格式的可读标签。 */
export function formatLabel(format: string): string {
  const table: Record<string, string> = {
    chara_card_v3: '角色卡 V3',
    chara_card_v2: '角色卡 V2',
    chara_card_v1: '角色卡 V1（无 spec）',
    lorebook_v3: '世界书 V3',
    world_info_json: '世界书（酒馆导出）',
  };
  return table[format] ?? format;
}

/** 归段的可读标签。 */
export function sectionLabel(section: LoreSection): string {
  return { world: '世界段', character: '角色段', scene: '场景段' }[section];
}

/** 逻辑的可读标签。 */
export function logicLabel(logic: LoreLogic): string {
  return { and_any: 'AND ANY', not_all: 'NOT ALL', not_any: 'NOT ANY', and_all: 'AND ALL' }[logic];
}

const GRADIENTS = [
  'from-sky-500 to-indigo-600',
  'from-rose-500 to-pink-600',
  'from-emerald-500 to-teal-600',
  'from-amber-500 to-orange-600',
  'from-violet-500 to-purple-600',
];

function initialsOf(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return '?';
  const chars = [...trimmed];
  return chars.length <= 2 ? trimmed : chars.slice(0, 2).join('');
}

/** 把导入出的角色映射为前端展示用的角色卡。字段缺失时留空，不编造内容。 */
export function toCharacter(imported: ImportedCharacter, index: number): Character {
  const profile = [imported.description, imported.personality].filter(Boolean).join('\n\n');
  return {
    id: `imported-c-${index}`,
    name: imported.name,
    title: imported.nickname?.trim() || imported.tags[0] || imported.character_version || '导入角色',
    initials: initialsOf(imported.name),
    gradient: GRADIENTS[index % GRADIENTS.length],
    tags: imported.tags.slice(0, 6),
    bio: profile,
    stats: [],
    state: [],
    locked: false,
    location: '未指定',
  };
}

/** 把导入结果映射为世界库资产。世界层设置留待用户在世界库中编辑（docs/10 §5）。 */
export function toWorld(imported: ImportedWorld): World {
  return {
    name: imported.name,
    genre: imported.genre || '待整理',
    era: '—',
    day: 0,
    summary: imported.summary,
    rules: [],
    vars: [],
    factions: [],
    locations: [],
  };
}

export function toWorldAsset(imported: ImportedWorld, id: string, updated: string): WorldAsset {
  return {
    id,
    name: imported.name,
    genre: imported.genre || '待整理',
    summary: imported.summary || `来源：${imported.source_file ?? imported.source_kind}`,
    source: 'imported',
    characters: imported.characters.map(toCharacter),
    world: toWorld(imported),
    updated,
  };
}

/** 文件 → base64，供 Tauri IPC 传输任意类型（JSON / PNG）。 */
export async function fileToBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = '';
  const step = 0x8000;
  for (let i = 0; i < bytes.length; i += step) {
    binary += String.fromCharCode(...bytes.subarray(i, i + step));
  }
  return btoa(binary);
}
