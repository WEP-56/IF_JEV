import { useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react';
import { ArrowUp, ArrowDown, Square, PanelRight, Zap, FastForward, Plus, ChevronDown, Upload, MoreHorizontal, Check, Dices, Copy, RotateCcw, GitBranch, type LucideIcon } from 'lucide-react';
import type { Story, Settings, Message } from '../types';
import type { IfRulingCard } from '../session';
import { EVENT_TAGS } from '../data';
import { cn } from '../utils/cn';
import { EventMsg, ImageMsg, JevMsg, NarratorMsg, SystemMsg } from './Messages';

type ChatContextMenu =
  | { kind: 'select-all'; x: number; y: number }
  | { kind: 'message'; x: number; y: number; message: Message };

interface Props {
  story: Story;
  settings: Settings;
  busy: boolean;
  rightOpen: boolean;
  onToggleRight: () => void;
  onSend: (text: string, tag: string) => void;
  /**
   * 待确认的裁定卡。类型直接用后端那一份（`session.ts`），别再手写一遍——
   * 手写的副本漏掉一个字段（比如 `null`）就是一个编译期看不见、运行期才炸的坑。
   */
  pendingIf?: IfRulingCard | null;
  onConfirmIf: (input?: string) => void;
  onCancelIf: () => void;
  onReinterpretIf: () => void;
  onStop: () => void;
  onRegenerate: () => void;
  onBranch: () => void;
}

const RANDOM_EVENTS = [
  '所有手机信号在同一分钟消失',
  '一艘无人驾驶的渡船在江面上漂来',
  '时间快进 3 天',
  '有人开始在网上直播水位计读数',
  '一位陌生人敲响了水文站的门',
];

export default function ChatView(p: Props) {
  const [text, setText] = useState('');
  const [rulingInput, setRulingInput] = useState('');
  const [tag, setTag] = useState('天气');
  const [tagOpen, setTagOpen] = useState(false);
  const [atBottom, setAtBottom] = useState(true);
  const [contextMenu, setContextMenu] = useState<ChatContextMenu | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

  const visible = p.story.messages.filter((m) => p.settings.showJev || m.role !== 'jev');
  const lastNarrator = [...p.story.messages].reverse().find((m) => m.role === 'narrator');

  const toBottom = (smooth = true) => {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: smooth ? 'smooth' : 'auto' });
  };

  useEffect(() => toBottom(false), [p.story.id]);
  useEffect(() => {
    if (atBottom) toBottom();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.story.messages]);

  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = 'auto';
    ta.style.height = Math.min(ta.scrollHeight, 200) + 'px';
  }, [text]);

  useEffect(() => setRulingInput(p.pendingIf?.injection.input ?? ''), [p.pendingIf?.injection.input]);

  const send = (t = text, tg = tag) => {
    if (!t.trim() || p.busy) return;
    p.onSend(t.trim(), tg);
    setText('');
    setTagOpen(false);
    setAtBottom(true);
  };

  const openMessageMenu = (event: ReactMouseEvent<HTMLDivElement>, message: Message) => {
    if (message.role !== 'event' && message.role !== 'narrator') return;
    event.preventDefault();
    event.stopPropagation();
    setContextMenu({
      kind: 'message',
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - 190)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - 150)),
      message,
    });
  };

  const width = p.settings.chatWidth === 'narrow' ? 'max-w-[640px]' : p.settings.chatWidth === 'wide' ? 'max-w-[960px]' : 'max-w-[760px]';

  return (
    <main className="relative flex h-full min-w-0 flex-1 flex-col">
      <header className="flex h-[52px] shrink-0 items-center gap-2 pr-3 pl-5">
        <h1 className="truncate text-[14px] font-semibold">{p.story.title}</h1>
        <span className="text-[12.5px] text-muted">第 {p.story.world.day} 天</span>
        <button className="grid h-7 w-7 place-items-center rounded-md text-muted hover:bg-subtle hover:text-fg">
          <MoreHorizontal size={16} />
        </button>
        <div className="ml-auto flex items-center gap-1">
          <button className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[13px] text-fg/85 hover:bg-subtle">
            <Upload size={14} /> 导出
          </button>
          <button
            onClick={p.onToggleRight}
            className={cn('grid h-8 w-8 place-items-center rounded-lg hover:bg-subtle hover:text-fg', p.rightOpen ? 'text-fg' : 'text-muted')}
            title="角色 / 世界 / 导图 (Ctrl+J)"
          >
            <PanelRight size={16} />
          </button>
        </div>
      </header>

      <div
        ref={scrollRef}
        onContextMenu={(event) => {
          event.preventDefault();
          setContextMenu({
            kind: 'select-all',
            x: Math.max(8, Math.min(event.clientX, window.innerWidth - 190)),
            y: Math.max(8, Math.min(event.clientY, window.innerHeight - 100)),
          });
        }}
        onScroll={(e) => {
          const el = e.currentTarget;
          setAtBottom(el.scrollHeight - el.scrollTop - el.clientHeight < 80);
        }}
        className="flex-1 overflow-y-auto"
      >
        <div className={cn('selectable mx-auto space-y-6 px-8 pt-4 pb-10', width)}>
          {visible.map((m) => {
            switch (m.role) {
              case 'system':
                return <SystemMsg key={m.id} m={m} />;
              case 'event':
                return <EventMsg key={m.id} m={m} onContextMenu={(event) => openMessageMenu(event, m)} />;
              case 'jev':
                return <JevMsg key={m.id} m={m} settings={p.settings} />;
              case 'image':
                return p.settings.imageEnabled ? <ImageMsg key={m.id} m={m} /> : null;
              case 'narrator':
                return (
                  <NarratorMsg
                    key={m.id}
                    m={m}
                    settings={p.settings}
                    isLast={m.id === lastNarrator?.id && !p.busy}
                    onChoice={(c) => send(c.replace(/^注入事件：/, ''), /快进/.test(c) ? '时间跳跃' : '自定义')}
                    onRegenerate={p.onRegenerate}
                    onBranch={p.onBranch}
                    onContextMenu={(event) => openMessageMenu(event, m)}
                  />
                );
            }
          })}
        </div>
      </div>

      {contextMenu && (
        <div className="fixed inset-0 z-40" onMouseDown={() => setContextMenu(null)} onContextMenu={(event) => event.preventDefault()}>
          <div
            role="menu"
            style={{ left: contextMenu.x, top: contextMenu.y }}
            onMouseDown={(event) => event.stopPropagation()}
            className="fade-up fixed z-50 min-w-48 rounded-lg border border-line bg-elev p-1 shadow-xl"
          >
            {contextMenu.kind === 'select-all' ? (
              <ContextMenuItem
                icon={Copy}
                label="全选"
                onClick={() => {
                  if (scrollRef.current) {
                    const range = document.createRange();
                    range.selectNodeContents(scrollRef.current);
                    const selection = window.getSelection();
                    selection?.removeAllRanges();
                    selection?.addRange(range);
                  }
                  setContextMenu(null);
                }}
              />
            ) : (
              <>
                <ContextMenuItem
                  icon={Copy}
                  label={contextMenu.message.role === 'event' ? '复制事件' : '复制正文'}
                  onClick={() => { void navigator.clipboard?.writeText(contextMenu.message.content ?? ''); setContextMenu(null); }}
                />
                {contextMenu.message.role === 'narrator' && contextMenu.message.id === lastNarrator?.id && !p.busy && (
                  <>
                    <ContextMenuItem icon={RotateCcw} label="重新叙述" onClick={() => { p.onRegenerate(); setContextMenu(null); }} />
                    <ContextMenuItem icon={GitBranch} label="从当前节点分支" onClick={() => { p.onBranch(); setContextMenu(null); }} />
                  </>
                )}
              </>
            )}
          </div>
        </div>
      )}

      {!atBottom && (
        <button
          onClick={() => toBottom()}
          className="fade-up absolute bottom-[132px] left-1/2 grid h-9 w-9 -translate-x-1/2 place-items-center rounded-full border border-line bg-elev text-fg/80 shadow-lg hover:text-fg"
        >
          <ArrowDown size={16} />
        </button>
      )}

      {p.pendingIf && (
        <div className="shrink-0 px-8 pb-3">
          <div className={cn('mx-auto rounded-2xl border border-accent/30 bg-elev p-4 shadow-sm', width)}>
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-[13px] font-semibold text-accent">待确认的 IF 裁定卡</div>
                <div className="mt-0.5 text-[12px] text-muted">{p.pendingIf.injection.kind} · {p.pendingIf.injection.time_anchor} · 锁定 {p.pendingIf.injection.lock}</div>
              </div>
              <span className="rounded-full bg-accent/10 px-2 py-1 text-[11px] text-accent">pending</span>
            </div>
            <textarea value={rulingInput} onChange={(event) => setRulingInput(event.target.value)} rows={2} className="mt-3 w-full resize-y rounded-xl border border-line bg-bg px-3 py-2 text-[13px] leading-relaxed outline-none focus:border-accent/60" />
            <div className="mt-2 text-[12px] text-muted">核心命题：{p.pendingIf.injection.core} · 范围：{p.pendingIf.injection.scope}</div>
            {p.pendingIf.warnings.length > 0 && <div className="mt-2 text-[12px] text-amber-500">{p.pendingIf.warnings.join('；')}</div>}
            {!!p.pendingIf.conflicts?.length && <div className="mt-2 space-y-1 text-[12px] text-rose-400">{p.pendingIf.conflicts.map((conflict) => <div key={conflict.event}>冲突：{conflict.existing_core}（{conflict.lock}，{conflict.reason}）</div>)}</div>}
            <div className="mt-3 flex justify-end gap-2">
              <button onClick={p.onCancelIf} disabled={p.busy} className="rounded-lg px-3 py-1.5 text-[13px] text-muted hover:bg-subtle disabled:opacity-40">取消</button>
              <button onClick={() => p.onConfirmIf(rulingInput)} disabled={p.busy || !rulingInput.trim()} className="rounded-lg bg-accent px-3 py-1.5 text-[13px] font-medium text-white hover:opacity-90 disabled:opacity-40">确认并锁定</button>
              {!!p.pendingIf.conflicts?.length && <button onClick={p.onReinterpretIf} disabled={p.busy} className="rounded-lg bg-rose-500 px-3 py-1.5 text-[13px] font-medium text-white hover:opacity-90 disabled:opacity-40">按重释确认</button>}
            </div>
          </div>
        </div>
      )}

      <div className="shrink-0 px-8 pb-5">
        <div className={cn('mx-auto', width)}>
          <div className="rounded-[22px] border border-line bg-elev px-2 pt-2 pb-2 shadow-[0_2px_14px_rgba(0,0,0,0.08)] transition focus-within:border-fg/20">
            <textarea
              ref={taRef}
              rows={1}
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  send();
                }
              }}
              placeholder="世界发生了什么…"
              className="block max-h-[200px] min-h-[44px] w-full resize-none bg-transparent px-3 pt-1.5 text-[14.5px] leading-relaxed outline-none placeholder:text-muted"
            />
            <div className="flex items-center gap-1">
              <button className="grid h-8 w-8 place-items-center rounded-full text-fg/75 hover:bg-subtle hover:text-fg" title="附加设定文档">
                <Plus size={18} />
              </button>

              <div className="relative">
                <button
                  onClick={() => setTagOpen(!tagOpen)}
                  className="flex items-center gap-1 rounded-full px-2.5 py-1.5 text-[13px] text-accent hover:bg-subtle"
                >
                  <Zap size={13} fill="currentColor" />
                  {tag}
                  <ChevronDown size={13} className="opacity-70" />
                </button>
                {tagOpen && (
                  <div className="fade-up absolute bottom-10 left-0 z-30 w-40 rounded-xl border border-line bg-elev p-1 shadow-2xl shadow-black/30">
                    {EVENT_TAGS.map((t) => (
                      <button
                        key={t}
                        onClick={() => {
                          setTag(t);
                          setTagOpen(false);
                        }}
                        className="flex w-full items-center rounded-lg px-2.5 py-1.5 text-left text-[13px] hover:bg-subtle"
                      >
                        {t}
                        {t === tag && <Check size={14} className="ml-auto text-accent" />}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              <button
                disabled={p.busy}
                onClick={() => send('让故事自然推进一个时间单位', '时间跳跃')}
                className="flex items-center gap-1 rounded-full px-2.5 py-1.5 text-[13px] text-muted hover:bg-subtle hover:text-fg disabled:opacity-40"
              >
                <FastForward size={14} /> 自然推进
              </button>
              <button
                onClick={() => setText(RANDOM_EVENTS[Math.floor(Math.random() * RANDOM_EVENTS.length)])}
                className="grid h-8 w-8 place-items-center rounded-full text-muted hover:bg-subtle hover:text-fg"
                title="随机事件"
              >
                <Dices size={15} />
              </button>

              <span className="ml-auto truncate pr-2 text-[12.5px] text-muted">
                {p.settings.jev.model.split(' ')[0]} · {p.settings.llm.model}
              </span>
              {p.busy ? (
                <button onClick={p.onStop} className="grid h-8 w-8 shrink-0 place-items-center rounded-full bg-fg text-bg" title="停止">
                  <Square size={11} fill="currentColor" />
                </button>
              ) : (
                <button
                  onClick={() => send()}
                  disabled={!text.trim()}
                  className="grid h-8 w-8 shrink-0 place-items-center rounded-full bg-accent text-white transition disabled:bg-subtle disabled:text-muted"
                  title="注入事件"
                >
                  <ArrowUp size={17} strokeWidth={2.4} />
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </main>
  );
}

function ContextMenuItem({ icon: Icon, label, onClick }: { icon: LucideIcon; label: string; onClick: () => void }) {
  return (
    <button
      role="menuitem"
      onClick={(event) => { event.stopPropagation(); onClick(); }}
      className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] hover:bg-subtle"
    >
      <Icon size={14} className="text-muted" /> {label}
    </button>
  );
}
