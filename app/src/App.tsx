import { useCallback, useEffect, useRef, useState } from 'react';
import TitleBar, { type MenuItem } from './components/TitleBar';
import { cn } from './utils/cn';
import Sidebar from './components/Sidebar';
import ChatView from './components/ChatView';
import RightPanel from './components/RightPanel';
import SettingsModal from './components/SettingsModal';
import type { Character, Message, Settings, Story } from './types';
import { defaultSettings, initialStories, mockJev, mockNarration, newStory, now, uid } from './data';

declare global {
  interface Window {
    __TAURI__?: {
      core?: { invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T> };
      window?: {
        getCurrentWindow: () => {
          minimize: () => Promise<void>;
          toggleMaximize: () => Promise<void>;
          close: () => Promise<void>;
          startDragging: () => Promise<void>;
        };
      };
    };
  }
}

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
  const [activeId, setActiveId] = useState(initialStories[0].id);
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [rightOpen, setRightOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const timers = useRef<number[]>([]);

  const story = stories.find((s) => s.id === activeId) ?? stories[0];
  const [dark, setDark] = useState(true);
  const [searchSignal, setSearchSignal] = useState(0);

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

  const newStoryFn = useCallback(() => {
    const s = newStory();
    setStories((prev) => [s, ...prev]);
    setActiveId((cur) => {
      setHist((h) => ({ back: [...h.back, cur], fwd: [] }));
      return s.id;
    });
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
        { label: '新建故事', shortcut: 'Ctrl+N', onClick: newStoryFn },
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

function tauriInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return Promise.reject(new Error('真实模型测试需要运行 Tauri 桌面应用'));
  return invoke<T>(command, args);
}
