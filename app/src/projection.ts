/**
 * 投影（`if-domain::Projection`）的前端切片与映射。
 *
 * 会话的**世界视图来自投影，不是来自资产**：资产是「用户准备好的那份材料」，
 * 投影是「这个世界现在是什么样」。播种之后两者就不再一致了——IF 改的是投影，不会回头改资产。
 *
 * 只声明前端真正会读的字段。投影很大（命题、事实、事实史、信念、声称、观测、注入……），
 * 把用不到的部分也声明出来，只会让这一层和后端实现无谓地绑死。需要哪块再加哪块。
 */
import type { Character, World } from './types';

export type Tier = 'dormant' | 'active' | 'foreground';
export type LoreSection = 'world' | 'character' | 'scene' | 'style';
export type LoreVisibility = 'public' | 'secret';
export type LoreStatus = 'active' | 'superseded';
export type SecondaryLogic = 'and_any' | 'and_all' | 'not_any' | 'not_all';
/** `Lock` 在 Rust 侧是 `rename_all = "UPPERCASE"`，所以就是 `L0`–`L3`。 */
export type Lock = 'L0' | 'L1' | 'L2' | 'L3';

export interface ProjectionSubject {
  id: string;
  kind: 'character' | 'group' | 'faction' | 'location' | 'item' | 'concept';
  name: string;
  aliases: string[];
  profile: string;
  voice?: string | null;
  tier: Tier;
  /** 是否已定型。定型后核心设定升为 L2，只能经 IF 修改（docs/10 §6）。 */
  shaped: boolean;
  created_by: string;
}

export interface ProjectionLore {
  id: string;
  title: string;
  content: string;
  keys: string[];
  secondary_keys: string[];
  logic: SecondaryLogic;
  constant: boolean;
  order: number;
  section: LoreSection;
  visibility: LoreVisibility;
  probability?: number | null;
  status: LoreStatus;
  source: string;
}

export interface ProjectionRule {
  id: string;
  text: string;
  lock: Lock;
  scope: string[];
}

export interface ProjectionSettings {
  seed: number;
  narrative_style: string;
  narrative_pov: 'third_limited' | 'first_person' | 'omniscient';
  mechanic_step: 'hour' | 'day' | 'week' | 'month';
  director_style: string;
  mode: 'sandbox' | 'challenge';
}

export interface Projection {
  anchor: { line: string; seq: number };
  subjects: Record<string, ProjectionSubject>;
  lore: Record<string, ProjectionLore>;
  rules: Record<string, ProjectionRule>;
  /** 自世界纪元起经过的分钟数（Rust `WorldTime` 是 transparent 的）。 */
  world_time: number;
  narrative_order: number;
  settings?: ProjectionSettings | null;
  world_seed?: number | null;
}

/* ---------------------------------------------------------------- 展示

   下面这些是**演示数据形状**（`types.ts` 的 `World` / `Character`）的填充。
   那个形状是早期前端稿留下的，字段比真实世界模型花哨（stats / factions / locations
   在 IF 里都还没有对应物）。这里只填有依据的部分，剩下的留空——渲染层负责省略空段。 */

const TIER_LABEL: Record<Tier, string> = {
  dormant: '暂不模拟',
  active: '正常模拟',
  foreground: '当前镜头',
};

const SECTION_LABEL: Record<LoreSection, string> = {
  world: '世界',
  character: '角色',
  scene: '场景',
  style: '文风',
};

const GRADIENTS = [
  'from-sky-500 to-indigo-600',
  'from-rose-500 to-pink-600',
  'from-emerald-500 to-teal-600',
  'from-amber-500 to-orange-600',
  'from-violet-500 to-purple-600',
];

export function tierLabel(tier: Tier): string {
  return TIER_LABEL[tier];
}

export function sectionLabel(section: LoreSection): string {
  return SECTION_LABEL[section];
}

/** `WorldTime` 的默认展示（与 Rust `Display` 一致）：第 N 天 HH:MM。 */
export function formatWorldTime(minutes: number): string {
  const day = Math.floor(minutes / 1440);
  const rest = minutes - day * 1440;
  const pad = (n: number) => String(n).padStart(2, '0');
  return `第 ${day} 天 ${pad(Math.floor(rest / 60))}:${pad(rest % 60)}`;
}

function initialsOf(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return '?';
  const chars = [...trimmed];
  return chars.length <= 2 ? trimmed : chars.slice(0, 2).join('');
}

/**
 * 主体 → 角色卡。**只有 `name` / `profile` / `aliases` / `tier` / `shaped` 是有依据的**，
 * `stats` 之类的数值面板留空——IF 里目前没有对应的量。
 */
export function toCharacters(projection: Projection): Character[] {
  return Object.keys(projection.subjects)
    .sort()
    .map((id, index) => {
      const subject = projection.subjects[id];
      return {
        id: subject.id,
        name: subject.name,
        title: subject.aliases[0] ?? TIER_LABEL[subject.tier],
        initials: initialsOf(subject.name),
        gradient: GRADIENTS[index % GRADIENTS.length],
        // 「草稿 / 已定型」是投影里真实存在的状态（docs/10 §6）
        tags: subject.shaped ? ['已定型'] : ['草稿'],
        bio: subject.profile,
        stats: [],
        state: [{ k: '模拟分辨率', v: TIER_LABEL[subject.tier] }],
        locked: subject.shaped,
        location: '未指定',
      };
    });
}

/** 世界视图。名字与题材来自资产（投影里没有），时间与规则来自投影。 */
export function toWorld(
  projection: Projection,
  asset: { name: string; genre: string; summary: string },
): World {
  return {
    name: asset.name,
    genre: asset.genre,
    era: formatWorldTime(projection.world_time),
    day: Math.floor(projection.world_time / 1440),
    summary: asset.summary,
    rules: Object.keys(projection.rules)
      .sort()
      .map((id) => projection.rules[id].text),
    vars: [],
    factions: [],
    locations: [],
  };
}

/** 设定条目，按 `order` 再按 ID 排——与视图编译的排序一致（docs/08 §5）。 */
export function orderedLore(projection: Projection): ProjectionLore[] {
  return Object.keys(projection.lore)
    .map((id) => projection.lore[id])
    .filter((entry) => entry.status === 'active')
    .sort((a, b) => a.order - b.order || (a.id < b.id ? -1 : 1));
}
