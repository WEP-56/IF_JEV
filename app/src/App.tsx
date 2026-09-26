import { useCallback, useEffect, useRef, useState } from 'react';
import TitleBar, { type MenuItem } from './components/TitleBar';
import { cn } from './utils/cn';
import Sidebar from './components/Sidebar';
import ChatView from './components/ChatView';
import RightPanel from './components/RightPanel';
import SettingsModal from './components/SettingsModal';
import type { Character, LibraryAsset, Message, Settings, Story, WorldAsset } from './types';
import { defaultSettings, initialStories, initialWorldAssets, mockJev, mockNarration, newStory, now, uid } from './data';
import WorldLibrary from './components/WorldLibrary';
import WorldPicker from './components/WorldPicker';
import WorldImportPreview from './components/WorldImportPreview';
import { fileToBase64, type ImportedWorld } from './import';
import {
  createWrittenWorld,
  deleteWorldAsset,
  demoLibraryAssets,
  demoWorldById,
  importFileToLibrary,
  listWorlds,
  loadWorldAsset,
  toLibraryAssets,
  toWorldAssetFromDetail,
} from './library';
import { isNative, tauriInvoke } from './ipc';

type NativeWorldSnapshot = {
  path: string;
  label: string;
  active_line_id: string;
  head_seq: number;
  event_count: number;
  projection?: { injections?: unknown[] };
  pending_if?: IfRulingCard;
};

type IfRulingCard = {
  turn: string;
  status: 'pending' | 'confirmed' | 'cancelled';
  injection: {
    input: string;
    kind: string;
    core: string;
    time_anchor: string;
    scope: string;
    lock: string;
    digestion?: string;
    non_commitments: string[];
  };
  warnings: string[];
  conflicts?: { event: string; existing_core: string; lock: string; reason: string }[];
};

const LS_KEY = 'if-preview-settings-v2';

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (raw) {
      const saved = JSON.parse(raw) as Partial<Settings>;
      return {
        ...defaultSettings,
        ...saved,
        llm: { ...defaultSettings.llm, ...saved.llm, key: '' },
        jev: { ...defaultSettings.jev, ...saved.jev, key: '' },
        image: { ...defaultSettings.image, ...saved.image, key: '' },
      };
    }
  } catch {
    /* ignore */
  }
  return defaultSettings;
}

export default function App() {
  const [stories, setStories] = useState<Story[]>(initialStories);
  /**
   * 世界库列表。Tauri 模式下是 `library.db` 的快照（重启后还在）；
   * 浏览器预览模式退到演示资产，并且只读——不假装已经存下来了。
   */
  const [assets, setAssets] = useState<LibraryAsset[]>(() => demoLibraryAssets(initialWorldAssets));
  const [libraryError, setLibraryError] = useState<string | null>(null);
  const [activeId, setActiveId] = useState(initialStories[0].id);
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [rightOpen, setRightOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [worldLibraryOpen, setWorldLibraryOpen] = useState(false);
  const [worldPickerOpen, setWorldPickerOpen] = useState(false);
  /** 导入解析结果，等待用户审阅确认后才写入世界库（docs/10 §7）。 */
  const [importPreview, setImportPreview] = useState<{ imported: ImportedWorld; fileName: string; dataBase64: string } | null>(null);
  const [importNotice, setImportNotice] = useState<{ tone: 'error' | 'info'; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [nativeWorld, setNativeWorld] = useState<NativeWorldSnapshot | null>(null);
  const [nativeWorldStatus, setNativeWorldStatus] = useState<'checking' | 'open' | 'empty' | 'error'>('checking');
  const timers = useRef<number[]>([]);
  const pendingNativeMessageId = useRef<string | null>(null);

  const story = stories.find((s) => s.id === activeId) ?? stories[0];
  const [dark, setDark] = useState(true);
  const [searchSignal, setSearchSignal] = useState(0);

  useEffect(() => {
    let disposed = false;
    const load = async () => {
      if (!window.__TAURI__?.core?.invoke) {
        setNativeWorldStatus('empty');
        return;
      }
      try {
        const snapshot = await tauriInvoke<NativeWorldSnapshot>('get_world_snapshot');
        if (!disposed) {
          setNativeWorld(snapshot);
          setNativeWorldStatus('open');
        }
      } catch {
        if (!disposed) setNativeWorldStatus('empty');
      }
    };
    void load();
    const listen = window.__TAURI__?.event?.listen;
    let unlistenOpened: (() => void) | undefined;
    let unlistenClosed: (() => void) | undefined;
    if (listen) {
      void listen<NativeWorldSnapshot>('world://opened', ({ payload }) => {
        if (!disposed) {
          setNativeWorld(payload);
          setNativeWorldStatus('open');
        }
      }).then((fn) => { unlistenOpened = fn; });
      void listen('world://closed', () => {
        if (!disposed) {
          setNativeWorld(null);
          setNativeWorldStatus('empty');
        }
      }).then((fn) => { unlistenClosed = fn; });
    }
    return () => {
      disposed = true;
      unlistenOpened?.();
      unlistenClosed?.();
    };
  }, []);

  /* ---------- 导航历史 ---------- */
  const [hist, setHist] = useState<{ back: string[]; fwd: string[] }>({ back: [], fwd: [] });
  const go = (id: string) => {
    if (id === activeId) return;
    setHist((h) => ({ back: [...h.back, activeId], fwd: [] }));
    setActiveId(id);
  };
  const goBack = () => {
    const prev = hist.back.filter((id) => stories.some((s) => s.id === id)).pop();
    if (!prev) return;
    setHist((h) => ({ back: h.back.slice(0, h.back.lastIndexOf(prev)), fwd: [activeId, ...h.fwd] }));
    setActiveId(prev);
  };
  const goForward = () => {
    const next = hist.fwd.find((id) => stories.some((s) => s.id === id));
    if (!next) return;
    setHist((h) => ({ back: [...h.back, activeId], fwd: h.fwd.slice(h.fwd.indexOf(next) + 1) }));
    setActiveId(next);
  };

  /* ---------- theme ---------- */
  useEffect(() => {
    const { key: _llmKey, ...llm } = settings.llm;
    const { key: _jevKey, ...jev } = settings.jev;
    const { key: _imageKey, ...image } = settings.image;
    localStorage.setItem(LS_KEY, JSON.stringify({ ...settings, llm, jev, image }));
    const root = document.documentElement;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
      const d = settings.theme === 'dark' || (settings.theme === 'system' && mq.matches);
      root.classList.toggle('dark', d);
      setDark(d);
    };
    apply();
    root.style.setProperty('--accent', settings.accent);
    mq.addEventListener('change', apply);
    return () => mq.removeEventListener('change', apply);
  }, [settings]);

  const toggleTheme = () => setSettings((s) => ({ ...s, theme: dark ? 'light' : 'dark' }));

  const newStoryFn = useCallback(() => setWorldPickerOpen(true), []);

  const createSession = useCallback((world: WorldAsset) => {
    const template = initialStories.find((item) => item.world.name === world.name) ?? initialStories[0];
    const s = { ...newStory(), worldId: world.id, title: `${world.name} · 新会话`, genre: world.genre, color: template.color, characters: world.characters, world: world.world, messages: [{ id: uid(), role: 'system' as const, content: `已选择世界「${world.name}」· 会话准备开始`, time: now() }] };
    setStories((prev) => [s, ...prev]);
    setActiveId((cur) => {
      setHist((h) => ({ back: [...h.back, cur], fwd: [] }));
      return s.id;
    });
    setWorldPickerOpen(false);
  }, []);

  /**
   * 重新读世界库。Tauri 模式下读 `library.db`；读不出来就**显式报错并清空列表**，
   * 不留下上一次的陈旧快照让人以为东西还在。浏览器预览用演示资产顶替（只读）。
   */
  const reloadLibrary = useCallback(async () => {
    if (!isNative()) {
      setAssets(demoLibraryAssets(initialWorldAssets));
      setLibraryError(null);
      return;
    }
    try {
      setAssets(await listWorlds());
      setLibraryError(null);
    } catch (error) {
      setAssets([]);
      setLibraryError(`读取世界库失败：${describe(error)}`);
    }
  }, []);

  useEffect(() => {
    if (worldLibraryOpen || worldPickerOpen) void reloadLibrary();
  }, [worldLibraryOpen, worldPickerOpen, reloadLibrary]);

  /**
   * 选中一个资产 → 读详情 → 用详情里的角色与世界视图播种新会话。
   * 列表只有摘要，所以详情要现读（也顺手把「资产被改动过」这件事同步进来）。
   */
  const selectWorld = useCallback(async (id: string) => {
    try {
      const asset = isNative()
        ? toWorldAssetFromDetail(await loadWorldAsset(id))
        : demoWorldById(initialWorldAssets, id);
      if (!asset) {
        setImportNotice({ tone: 'error', text: `世界库里没有资产 ${id}` });
        return;
      }
      createSession(asset);
    } catch (error) {
      setImportNotice({ tone: 'error', text: `读取世界资产失败：${describe(error)}` });
    }
  }, [createSession]);

  /**
   * 导入酒馆角色卡 / 世界书：读文件 → Rust 侧确定性解析 → 预览。
   * 不在这里编造资产：解析失败就如实报错，未确认就不写库。
   */
  const importWorld = useCallback(async (file: File) => {
    if (!isNative()) {
      setImportNotice({ tone: 'info', text: '导入解析需要运行 Tauri 桌面应用；浏览器预览模式不使用演示数据顶替。' });
      return;
    }
    setImportNotice({ tone: 'info', text: `正在解析 ${file.name}…` });
    try {
      const dataBase64 = await fileToBase64(file);
      const imported = await tauriInvoke<ImportedWorld>('import_world_file', { fileName: file.name, dataBase64 });
      // 原始 base64 留到确认时用：写入世界库走的是「文件进、解析+落库」一条命令，
      // 免得把解析结果再从 JS 塞回 Rust（那样导入层的映射就绕过了一次）。
      setImportPreview({ imported, fileName: file.name, dataBase64 });
      setImportNotice(null);
    } catch (error) {
      setImportNotice({ tone: 'error', text: `导入失败：${describe(error)}` });
    }
  }, []);

  /** 用户确认后才写库。同一张卡再次导入会替换它的条目，而不是叠一份。 */
  const confirmImport = useCallback(async () => {
    if (!importPreview) return;
    setImportNotice({ tone: 'info', text: `正在写入世界库：${importPreview.fileName}…` });
    try {
      const change = await importFileToLibrary(importPreview.fileName, importPreview.dataBase64);
      setAssets(toLibraryAssets(change.assets));
      setLibraryError(null);
      setImportPreview(null);
      setImportNotice({
        tone: 'info',
        text: `已写入世界库：${change.detail.asset.name}（${change.detail.lore.length} 条设定，来源 ${change.detail.asset.id}）`,
      });
      setWorldLibraryOpen(false);
      setWorldPickerOpen(true);
    } catch (error) {
      setImportNotice({ tone: 'error', text: `写入世界库失败：${describe(error)}` });
    }
  }, [importPreview]);

  /** 手动撰写：建一个空骨架资产，主体与世界设置留到世界库里补（docs/10 §2）。 */
  const createWorldManually = useCallback(async () => {
    if (!isNative()) {
      setImportNotice({ tone: 'info', text: '手动撰写需要运行 Tauri 桌面应用；浏览器预览模式不写入任何东西。' });
      return;
    }
    try {
      const change = await createWrittenWorld(`手动世界 ${assets.length + 1}`, '待撰写', '手动撰写的世界资产。');
      setAssets(toLibraryAssets(change.assets));
      setImportNotice({ tone: 'info', text: `已新建世界：${change.detail.asset.name}` });
    } catch (error) {
      setImportNotice({ tone: 'error', text: `新建世界失败：${describe(error)}` });
    }
  }, [assets.length]);

  /** 删除资产。被会话引用时后端会拒绝并说明原因，这里把它显示出来而不是假装成功。 */
  const removeWorld = useCallback(async (id: string) => {
    if (!isNative()) {
      setAssets((prev) => prev.filter((world) => world.id !== id));
      return;
    }
    try {
      setAssets(await deleteWorldAsset(id));
      setLibraryError(null);
    } catch (error) {
      setLibraryError(`删除失败：${describe(error)}`);
    }
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      const k = e.key.toLowerCase();
      if (mod && k === 'n') { e.preventDefault(); newStoryFn(); }
      if (mod && k === ',') { e.preventDefault(); setSettingsOpen(true); }
      if (mod && k === 'b') { e.preventDefault(); setSidebarOpen((v) => !v); }
      if (mod && k === 'j') { e.preventDefault(); setRightOpen((v) => !v); }
      if (mod && k === 'k') { e.preventDefault(); setSidebarOpen(true); setSearchSignal((n) => n + 1); }
      if (e.key === 'Escape') setSettingsOpen(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [newStoryFn]);

  /* ---------- helpers ---------- */
  const update = (id: string, fn: (s: Story) => Story) => setStories((prev) => prev.map((s) => (s.id === id ? fn(s) : s)));
  const patchMsg = (sid: string, mid: string, fn: (m: Message) => Message) =>
    update(sid, (s) => ({ ...s, messages: s.messages.map((m) => (m.id === mid ? fn(m) : m)) }));
  const later = (fn: () => void, ms: number) => {
    timers.current.push(window.setTimeout(fn, ms));
  };
  const clearTimers = () => {
    timers.current.forEach(clearTimeout);
    timers.current = [];
  };

  const streamNarration = (sid: string, text: string, choices: string[], onDone?: () => void) => {
    const nid = uid();
    update(sid, (s) => ({ ...s, messages: [...s.messages, { id: nid, role: 'narrator', content: '', time: now(), streaming: true }] }));
    let i = 0;
    const step = () => {
      i = Math.min(text.length, i + 2 + Math.floor(Math.random() * 3));
      const done = i >= text.length;
      patchMsg(sid, nid, (m) => ({ ...m, content: text.slice(0, i), streaming: !done, choices: done ? choices : undefined }));
      if (done) {
        setBusy(false);
        onDone?.();
      } else later(step, 28);
    };
    later(step, 250);
  };

  /* ---------- send event ---------- */
  const handleSend = (text: string, tag: string) => {
    const sid = story.id;
    if (nativeWorldStatus === 'open' && window.__TAURI__?.core?.invoke) {
      setBusy(true);
      const ev: Message = { id: uid(), role: 'event', eventTag: tag, content: text, time: now() };
      pendingNativeMessageId.current = ev.id;
      update(sid, (s) => ({ ...s, updated: now(), messages: [...s.messages, ev] }));
      void tauriInvoke<NativeWorldSnapshot>('submit_if_model', { runId: crypto.randomUUID(), input: text })
        .then((snapshot) => {
          setNativeWorld(snapshot);
          update(sid, (s) => ({ ...s, messages: [...s.messages, { id: uid(), role: 'system', content: `IF 已创建待确认裁定卡（解析：${text}）。`, time: now() }] }));
        })
        .catch((error: unknown) => {
          update(sid, (s) => ({
            ...s,
            messages: [
              ...s.messages.filter((message) => message.id !== pendingNativeMessageId.current),
              {
              id: uid(),
              role: 'system',
              content: `IF 提交失败：${error instanceof Error ? error.message : String(error)}`,
              time: now(),
              },
            ],
          }));
          pendingNativeMessageId.current = null;
        })
        .finally(() => setBusy(false));
      return;
    }
    const isFirst = story.characters.length === 0;
    setBusy(true);
    const ev: Message = { id: uid(), role: 'event', eventTag: tag, content: text, time: now() };
    const jid = uid();
    const jev: Message = { id: jid, role: 'jev', time: now(), jev: mockJev(text, tag) };
    update(sid, (s) => ({ ...s, updated: now(), messages: [...s.messages, ev, jev] }));

    const t0 = performance.now();
    let d = 0;
    const tick = () => {
      d++;
      patchMsg(sid, jid, (m) => ({ ...m, jev: { ...m.jev!, done: d, latency: d >= 5 ? Math.round(performance.now() - t0) : undefined } }));
      if (d < 5) later(tick, 380 + Math.random() * 250);
      else {
        const { text: story_text, choices } = mockNarration(text);
        later(
          () =>
            streamNarration(sid, story_text, choices, () => {
              update(sid, (s) => {
                const jump = /快进|推进/.test(text) || tag === '时间跳跃' ? 3 : 0;
                const day = Math.max(1, s.world.day + jump);
                const mains = s.map.filter((n) => n.main);
                const parent = mains[mains.length - 1];
                const node = {
                  id: uid(),
                  parent: parent?.id,
                  label: text.replace(/^IF\s*/, '').slice(0, 8),
                  type: 'event' as const,
                  day: `D${day}`,
                  main: true,
                  desc: `用户注入：${text}`,
                };
                const base: Story = {
                  ...s,
                  tokens: s.tokens + story_text.length * 2 + 600,
                  map: [...s.map, node],
                  world: { ...s.world, day },
                };
                if (!isFirst) return base;
                // 新世界：由 JEV 根据首个事件生成初始状态
                return {
                  ...base,
                  title: text.replace(/^IF\s*/, '').slice(0, 10),
                  genre: '原创',
                  color: 'bg-amber-500',
                  world: {
                    ...base.world,
                    name: text.replace(/^IF\s*/, '').slice(0, 10),
                    genre: '原创 · 待细化',
                    era: 'JEV 推定',
                    summary: `起点事件「${text}」已写入世界底层。`,
                    rules: ['首个事件为不可违背的世界公理', '每个 tick 推演一次因果链'],
                    vars: [
                      { label: '世界张力', value: '68', pct: 68, trend: 'up' },
                      { label: '秩序', value: '54', pct: 54, trend: 'down' },
                      { label: '未知度', value: '90%', pct: 90, trend: 'neutral' },
                    ],
                    locations: [{ name: '起点', status: '已生成', danger: 30 }],
                  },
                  characters: [
                    {
                      id: uid(),
                      name: '无名观测者',
                      title: '待定型',
                      initials: '?',
                      gradient: 'from-stone-400 to-stone-700',
                      tags: ['草稿'],
                      bio: '第一个注意到世界发生变化的人。TA 的姓名、过去与动机会随事件逐渐清晰。',
                      stats: [{ label: '理智', value: 75, color: 'bg-sky-500' }],
                      state: [{ k: '当前目标', v: '弄清发生了什么' }],
                      locked: false,
                      location: '起点',
                    },
                  ],
                };
              });
            }),
          200,
        );
      }
    };
    later(tick, 420);
  };

  const handleConfirmIf = (input?: string) => {
    if (!nativeWorld?.pending_if) return;
    setBusy(true);
    void tauriInvoke<NativeWorldSnapshot>('confirm_if', input ? { input } : {})
      .then((snapshot) => {
        setNativeWorld(snapshot);
        update(story.id, (s) => ({
          ...s,
          messages: s.messages.map((message) => message.id === pendingNativeMessageId.current ? { ...message, content: input?.trim() || message.content } : message)
            .concat({ id: uid(), role: 'system', time: now(), content: `裁定卡已确认，IF 已提交到事件日志（seq ${snapshot.head_seq}）。` }),
        }));
        pendingNativeMessageId.current = null;
      })
      .catch((error: unknown) => update(story.id, (s) => ({
        ...s,
        messages: [...s.messages, { id: uid(), role: 'system', time: now(), content: `确认裁定卡失败：${error instanceof Error ? error.message : String(error)}` }],
      })))
      .finally(() => setBusy(false));
  };

  const handleCancelIf = () => {
    if (!nativeWorld?.pending_if) return;
    setBusy(true);
    void tauriInvoke<NativeWorldSnapshot>('cancel_if')
      .then((snapshot) => {
        setNativeWorld(snapshot);
        update(story.id, (s) => ({
          ...s,
          messages: s.messages.filter((message) => message.id !== pendingNativeMessageId.current)
            .concat({ id: uid(), role: 'system', time: now(), content: '裁定卡已取消，世界状态未改变。' }),
        }));
        pendingNativeMessageId.current = null;
      })
      .catch((error: unknown) => update(story.id, (s) => ({
        ...s,
        messages: [...s.messages, { id: uid(), role: 'system', time: now(), content: `取消裁定卡失败：${error instanceof Error ? error.message : String(error)}` }],
      })))
      .finally(() => setBusy(false));
  };

  const handleReinterpretIf = () => {
    if (!nativeWorld?.pending_if) return;
    setBusy(true);
    void tauriInvoke<NativeWorldSnapshot>('reinterpret_if')
      .then((snapshot) => {
        setNativeWorld(snapshot);
        update(story.id, (s) => ({ ...s, messages: [...s.messages, { id: uid(), role: 'system', time: now(), content: `冲突已按重释处理，IF 已提交到事件日志（seq ${snapshot.head_seq}）。` }] }));
      })
      .catch((error: unknown) => update(story.id, (s) => ({ ...s, messages: [...s.messages, { id: uid(), role: 'system', time: now(), content: `重释 IF 失败：${error instanceof Error ? error.message : String(error)}` }] })))
      .finally(() => setBusy(false));
  };

  const handleStop = () => {
    clearTimers();
    setBusy(false);
    update(story.id, (s) => ({
      ...s,
      messages: s.messages.map((m) =>
        m.streaming ? { ...m, streaming: false } : m.jev && m.jev.done < 5 ? { ...m, jev: { ...m.jev, done: 5, latency: 0 } } : m,
      ),
    }));
  };

  const handleRegenerate = () => {
    if (busy) return;
    const last = [...story.messages].reverse().find((m) => m.role === 'narrator');
    if (!last) return;
    setBusy(true);
    update(story.id, (s) => ({ ...s, messages: s.messages.filter((m) => m.id !== last.id) }));
    streamNarration(story.id, last.content ?? '', last.choices ?? []);
  };

  const handleBranchFrom = (nodeId?: string) => {
    update(story.id, (s) => {
      const mains = s.map.filter((n) => n.main);
      const from = s.map.find((n) => n.id === nodeId) ?? mains[mains.length - 1];
      return {
        ...s,
        map: [
          ...s.map,
          { id: uid(), parent: from.id, label: `平行线 #${s.map.filter((n) => n.type === 'branch').length + 1}`, type: 'branch', day: from.day, main: false, desc: `从「${from.label}」分出的平行世界线，JEV 已保存该时刻的完整状态快照。` },
        ],
        messages: [...s.messages, { id: uid(), role: 'system', time: now(), content: `已从「${from.label}」分出平行世界线 · 快照已写入 IF 导图` }],
      };
    });
    setRightOpen(true);
  };

  const handleUpdateChar = (c: Character) => update(story.id, (s) => ({ ...s, characters: s.characters.map((x) => (x.id === c.id ? c : x)) }));

  const handlePortrait = (c: Character) =>
    update(story.id, (s) => ({
      ...s,
      characters: s.characters.map((x) => (x.id === c.id ? { ...x, portrait: true } : x)),
      messages: [
        ...s.messages,
        {
          id: uid(),
          role: 'image',
          time: now(),
          image: { name: `${c.name} · 立绘`, gradient: c.gradient, caption: `${settings.image.model} · ${settings.image.size}` },
        },
      ],
    }));

  const handleSelect = (id: string) => {
    if (busy) handleStop();
    go(id);
  };

  const handleDelete = (id: string) => {
    setStories((prev) => {
      const next = prev.filter((s) => s.id !== id);
      if (!next.length) {
        const s = newStory();
        setActiveId(s.id);
        return [s];
      }
      if (id === activeId) setActiveId(next[0].id);
      return next;
    });
  };

  const duplicate = (id: string) => {
    const src = stories.find((s) => s.id === id)!;
    const copy = { ...src, id: uid(), title: `${src.title}（副本）`, pinned: false, group: '今天' as const };
    setStories((prev) => [copy, ...prev]);
    go(copy.id);
  };

  const menus: { label: string; items: MenuItem[] }[] = [
    {
      label: '文件',
      items: [
        { label: '新建会话', shortcut: 'Ctrl+N', onClick: newStoryFn },
        { label: '世界库', onClick: () => setWorldLibraryOpen(true) },
        { label: '复制当前世界线', onClick: () => duplicate(story.id) },
        { divider: true },
        { label: '导入世界…' },
        { label: '导出当前故事…' },
        { divider: true },
        { label: '设置', shortcut: 'Ctrl+,', onClick: () => setSettingsOpen(true) },
      ],
    },
    {
      label: '编辑',
      items: [
        { label: '重新叙述', onClick: handleRegenerate, disabled: busy },
        { label: '从当前节点分支', onClick: () => handleBranchFrom() },
        { divider: true },
        { label: '搜索故事', shortcut: 'Ctrl+K', onClick: () => { setSidebarOpen(true); setSearchSignal((n) => n + 1); } },
      ],
    },
    {
      label: '视图',
      items: [
        { label: sidebarOpen ? '隐藏侧栏' : '显示侧栏', shortcut: 'Ctrl+B', onClick: () => setSidebarOpen((v) => !v) },
        { label: rightOpen ? '隐藏右栏' : '显示右栏', shortcut: 'Ctrl+J', onClick: () => setRightOpen((v) => !v) },
        { divider: true },
        { label: settings.showJev ? '隐藏 JEV 推演' : '显示 JEV 推演', onClick: () => setSettings((s) => ({ ...s, showJev: !s.showJev })) },
        { label: dark ? '浅色主题' : '深色主题', onClick: toggleTheme },
      ],
    },
    {
      label: '帮助',
      items: [
        { label: '快捷键' },
        { label: '源代码仓库' },
        { divider: true },
        { label: '关于 IF', onClick: () => setSettingsOpen(true) },
      ],
    },
  ];

  return (
    <div className="flex h-full flex-col bg-chrome">
      <TitleBar
        menus={menus}
        nativeWorld={nativeWorld}
        nativeWorldStatus={nativeWorldStatus}
        onToggleSidebar={() => setSidebarOpen((v) => !v)}
        canBack={hist.back.length > 0}
        canForward={hist.fwd.length > 0}
        onBack={goBack}
        onForward={goForward}
      />

      <div className="flex min-h-0 flex-1">
        {sidebarOpen && (
          <Sidebar
            stories={stories}
            activeId={story.id}
            dark={dark}
            onSelect={handleSelect}
            onNew={newStoryFn}
            onLibrary={() => setWorldLibraryOpen(true)}
            onSettings={() => setSettingsOpen(true)}
            onDelete={handleDelete}
            onPin={(id) => update(id, (s) => ({ ...s, pinned: !s.pinned }))}
            onDuplicate={duplicate}
            onRename={(id, title) => update(id, (s) => ({ ...s, title }))}
            onToggleTheme={toggleTheme}
            searchSignal={searchSignal}
          />
        )}

        {/* 内容面板：圆角内嵌于窗体 */}
        <div
          className={cn(
            'flex min-w-0 flex-1 overflow-hidden border-t border-line bg-bg',
            sidebarOpen && 'rounded-tl-[12px] border-l',
          )}
        >
          <ChatView
            story={story}
            settings={settings}
            busy={busy}
            rightOpen={rightOpen}
            onToggleRight={() => setRightOpen(!rightOpen)}
            onSend={handleSend}
            pendingIf={nativeWorld?.pending_if}
            onConfirmIf={handleConfirmIf}
            onCancelIf={handleCancelIf}
            onReinterpretIf={handleReinterpretIf}
            onStop={handleStop}
            onRegenerate={handleRegenerate}
            onBranch={() => handleBranchFrom()}
          />
          {rightOpen && (
            <RightPanel
              key={story.id}
              story={story}
              settings={settings}
              onUpdateChar={handleUpdateChar}
              onPortrait={handlePortrait}
              onBranchFrom={handleBranchFrom}
            />
          )}
        </div>
      </div>

      {settingsOpen && (
        <SettingsModal
          settings={settings}
          onChange={setSettings}
          onTestLlm={async (current) => {
            if (current.llm.key.trim()) await tauriInvoke('set_api_key', { slot: 'structure', key: current.llm.key });
            await tauriInvoke('save_settings', { settings: nativeSettings(current) });
            const result = await tauriInvoke<{ label: string; text: string; tool_calls: { name: string }[]; latency_ms: number }>('test_llm', {
              runId: crypto.randomUUID(), slot: 'structure', prompt: '输出一句测试文本，并调用工具给出一句 IF 断言。', withTool: true,
            });
            return `${result.label} · ${result.latency_ms}ms · ${result.text || result.tool_calls.map((call) => call.name).join(', ')}`;
          }}
          onTestJev={async (current) => {
            if (current.jev.key.trim()) await tauriInvoke('set_api_key', { slot: 'jev', key: current.jev.key });
            await tauriInvoke('save_settings', { settings: nativeSettings(current) });
            const result = await tauriInvoke<{ model: string; answers: Record<string, { type: string; noul?: number }>; cost_usd: number; latency_ms: number }>('test_judge', {
              runId: crypto.randomUUID(), backend: 'jev', input: {
                state: { scene: '晴朗的白天', question: '天空是否为蓝色？' },
                questions: [{ key: 'smoke.sky_blue', template: 'diagnostic.noul@1', target: 'sky', spec: { type: 'noul', instructions: '天空是蓝色的吗？', true_means: '是蓝色', false_means: '不是蓝色' } }],
              },
            });
            const answer = result.answers['smoke.sky_blue'];
            if (!answer) throw new Error('Jev 未返回 smoke.sky_blue 判定');
            return `${result.model} · p=${answer?.noul?.toFixed(2) ?? '无结果'} · $${result.cost_usd.toFixed(6)} · ${result.latency_ms}ms`;
          }}
          onClearKey={async (slot) => {
            await tauriInvoke('set_api_key', { slot, key: '' });
            setSettings((current) => slot === 'structure'
              ? { ...current, llm: { ...current.llm, key: '' } }
              : { ...current, jev: { ...current.jev, key: '' } });
          }}
          onClose={() => setSettingsOpen(false)}
          onClearData={() => {
            clearTimers();
            setBusy(false);
            setStories(initialStories);
            setActiveId(initialStories[0].id);
            setHist({ back: [], fwd: [] });
            setSettings(defaultSettings);
          }}
        />
      )}
      {worldPickerOpen && <WorldPicker worlds={assets} onSelect={selectWorld} onCreateWorld={() => { setWorldPickerOpen(false); setWorldLibraryOpen(true); }} onImport={importWorld} onClose={() => setWorldPickerOpen(false)} />}
      {worldLibraryOpen && <WorldLibrary worlds={assets} error={libraryError} onClose={() => setWorldLibraryOpen(false)} onCreate={() => { void createWorldManually(); }} onImport={importWorld} onDelete={(id) => { void removeWorld(id); }} />}
      {importPreview && <WorldImportPreview imported={importPreview.imported} onConfirm={() => { void confirmImport(); }} onCancel={() => { setImportPreview(null); }} />}
      {importNotice && (
        <div className={`absolute bottom-6 left-1/2 z-[60] flex max-w-[80%] -translate-x-1/2 items-center gap-2 rounded-lg border px-3 py-2 text-[12px] shadow-xl ${importNotice.tone === 'error' ? 'border-rose-500/50 bg-rose-500/10 text-rose-400' : 'border-line bg-elev text-muted'}`}>
          <span className="min-w-0">{importNotice.text}</span>
          <button onClick={() => setImportNotice(null)} className="shrink-0 rounded px-1 text-muted hover:text-fg" aria-label="关闭提示">×</button>
        </div>
      )}
    </div>
  );
}

type NativeSettings = {
  structure: Record<string, unknown>;
  narrative: Record<string, unknown>;
  jev: { endpoint: string; model: string; timeout_secs: number };
  engine: { strictness: number; causal_depth: number; beat_retries: number; auto_confirm_ruling: boolean };
};

function nativeSettings(settings: Settings): NativeSettings {
  const api = settings.llm.provider === 'Anthropic' ? 'anthropic_messages' : settings.llm.provider === 'OpenAI' ? 'open_ai_responses' : 'chat_completions';
  const provider = {
    name: settings.llm.provider, api, base_url: settings.llm.base, api_key: '', model: settings.llm.model,
    max_tokens: settings.llm.maxTokens, temperature: settings.llm.temperature, reasoning_effort: null,
    prompt_cache: true, replay_encrypted_reasoning: false,
  };
  return {
    structure: provider, narrative: provider,
    jev: { endpoint: settings.jev.endpoint, model: settings.jev.model, timeout_secs: 30 },
    engine: { strictness: settings.jev.strict, causal_depth: settings.jev.depth, beat_retries: 2, auto_confirm_ruling: false },
  };
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
