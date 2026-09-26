export type Role = 'event' | 'jev' | 'narrator' | 'system' | 'image';

export interface StateDiff {
  key: string;
  from: string;
  to: string;
  trend: 'up' | 'down' | 'neutral';
}

export interface JevResult {
  tick: string;
  steps: string[];
  done: number; // 已完成步骤
  diffs: StateDiff[];
  rules: string[];
  latency?: number;
  branchScore?: number;
}

export interface Message {
  id: string;
  role: Role;
  content?: string;
  time: string;
  eventTag?: string;
  jev?: JevResult;
  choices?: string[];
  streaming?: boolean;
  image?: { name: string; gradient: string; caption: string };
}

export interface Stat {
  label: string;
  value: number;
  color: string;
}

export interface Character {
  id: string;
  name: string;
  title: string;
  initials: string;
  gradient: string;
  tags: string[];
  bio: string;
  stats: Stat[];
  state: { k: string; v: string }[];
  locked: boolean;
  portrait?: boolean;
  location: string;
}

export interface WorldVar {
  label: string;
  value: string;
  pct?: number;
  trend: 'up' | 'down' | 'neutral';
}

export interface World {
  name: string;
  genre: string;
  era: string;
  day: number;
  summary: string;
  rules: string[];
  vars: WorldVar[];
  factions: { name: string; attitude: string; power: number }[];
  locations: { name: string; status: string; danger: number }[];
}

export interface MapNode {
  id: string;
  parent?: string;
  label: string;
  type: 'origin' | 'event' | 'state' | 'branch';
  day: string;
  main: boolean;
  desc: string;
}

export interface Story {
  id: string;
  /** Reusable world asset selected when this session was created. */
  worldId?: string;
  title: string;
  genre: string;
  color: string;
  updated: string;
  group: '置顶' | '今天' | '最近 7 天' | '更早';
  pinned?: boolean;
  messages: Message[];
  characters: Character[];
  world: World;
  map: MapNode[];
  tokens: number;
}

export interface WorldAsset {
  id: string;
  name: string;
  genre: string;
  summary: string;
  source: 'demo' | 'written' | 'imported';
  characters: Character[];
  world: World;
  updated: string;
}

export interface Settings {
  theme: 'light' | 'dark' | 'system';
  accent: string;
  storyFont: 'serif' | 'sans';
  fontSize: number;
  lineHeight: number;
  chatWidth: 'narrow' | 'normal' | 'wide';
  showJev: boolean;
  jevDetail: 'full' | 'compact';
  imageEnabled: boolean;
  llm: {
    provider: string;
    base: string;
    key: string;
    model: string;
    temperature: number;
    maxTokens: number;
    context: number;
    style: string;
  };
  jev: {
    endpoint: string;
    key: string;
    model: string;
    tick: string;
    depth: number;
    strict: number;
    seed: string;
    autoBranch: boolean;
  };
  image: {
    provider: string;
    endpoint: string;
    key: string;
    model: string;
    style: string;
    size: string;
    autoOnLock: boolean;
  };
  storage: {
    backend: string;
    path: string;
    autosave: boolean;
    snapshots: number;
    encrypt: boolean;
  };
}
