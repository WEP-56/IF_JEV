import { useState, type ReactNode } from 'react';
import { ChevronRight, Check, Copy, RotateCcw, GitBranch, Download, Undo2, Cpu, Zap } from 'lucide-react';
import type { Message, Settings } from '../types';
import { cn } from '../utils/cn';

function renderInline(text: string): ReactNode[] {
  return text.split(/(\*\*[^*]+\*\*)/g).map((p, i) =>
    p.startsWith('**') && p.endsWith('**') ? (
      <strong key={i} className="font-semibold text-fg">
        {p.slice(2, -2)}
      </strong>
    ) : (
      <span key={i}>{p}</span>
    ),
  );
}

export function SystemMsg({ m }: { m: Message }) {
  return <div className="py-1 text-center text-[12.5px] text-muted">{m.content}</div>;
}

export function EventMsg({ m }: { m: Message }) {
  return (
    <div className="fade-up group flex flex-col items-end">
      <div className="selectable max-w-[78%] rounded-[18px] bg-bubble px-4 py-2.5 text-[14.5px] leading-relaxed text-fg">
        {m.content}
      </div>
      <div className="mt-1 flex items-center gap-1 pr-2 text-[12px] text-muted">
        <Zap size={11} className="text-accent" fill="currentColor" />
        {m.eventTag}
        <span className="opacity-0 transition group-hover:opacity-100">· {m.time}</span>
      </div>
    </div>
  );
}

export function JevMsg({ m, settings }: { m: Message; settings: Settings }) {
  const j = m.jev!;
  const running = j.done < j.steps.length;
  const [openSteps, setOpenSteps] = useState(false);
  const [openDiff, setOpenDiff] = useState(settings.jevDetail === 'full');
  const ups = j.diffs.filter((d) => d.trend === 'up').length;
  const downs = j.diffs.filter((d) => d.trend === 'down').length;

  return (
    <div className="fade-up space-y-3">
      {/* 推演过程：一行可折叠 */}
      <div>
        <button
          onClick={() => !running && setOpenSteps(!openSteps)}
          className="flex items-center gap-1 text-[13.5px] text-muted hover:text-fg"
        >
          {running ? (
            <span className="shine-text">JEV 推演中 · {j.steps[j.done]}</span>
          ) : (
            <>
              JEV 推演 {((j.latency ?? 0) / 1000).toFixed(1)}s
              <ChevronRight size={14} className={cn('transition', openSteps && 'rotate-90')} />
            </>
          )}
        </button>
        {openSteps && !running && (
          <div className="fade-up mt-2 space-y-1 border-l border-line pl-3.5 text-[13px] text-muted">
            {j.steps.map((s) => (
              <div key={s} className="flex items-center gap-2">
                <Check size={13} className="text-emerald-500" /> {s}
              </div>
            ))}
            <div className="pt-1 text-[12px]">
              规则 {j.rules.join('、')}
              {j.branchScore ? ` · 一致性 ${j.branchScore}` : ''}
            </div>
          </div>
        )}
        {!openSteps && <div className="mt-2 h-px bg-line" />}
      </div>

      {/* 状态变更卡片 */}
      {!running && (
        <div className="overflow-hidden rounded-2xl bg-elev">
          <div className="flex items-center gap-3 px-3.5 py-3">
            <div className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-bg text-fg/80">
              <Cpu size={17} />
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-[14px] font-semibold">世界状态已更新</div>
              <div className="mt-0.5 text-[12.5px] text-muted">
                {j.tick}
                {ups > 0 && <span className="ml-2 text-rose-400">↑{ups}</span>}
                {downs > 0 && <span className="ml-1.5 text-sky-400">↓{downs}</span>}
                <span className="ml-1.5">· {j.diffs.length} 项</span>
              </div>
            </div>
            <button className="flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-[13px] font-medium hover:bg-subtle">
              回滚 <Undo2 size={14} />
            </button>
            <button
              onClick={() => setOpenDiff(!openDiff)}
              className={cn('rounded-lg border border-line px-2.5 py-1.5 text-[13px] font-medium hover:bg-subtle', openDiff && 'bg-subtle')}
            >
              详情
            </button>
          </div>
          {openDiff && (
            <div className="border-t border-line px-3.5 py-2">
              {j.diffs.map((d, i) => (
                <div key={i} className="flex items-center gap-3 py-1.5 text-[13px]">
                  <span className="min-w-0 flex-1 truncate text-muted">{d.key}</span>
                  <span className="text-muted line-through decoration-muted/50">{d.from}</span>
                  <span className="text-muted">→</span>
                  <span className={cn('font-medium', d.trend === 'up' ? 'text-rose-400' : d.trend === 'down' ? 'text-sky-400' : 'text-fg')}>
                    {d.to}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function NarratorMsg({
  m, settings, onChoice, onRegenerate, onBranch, isLast,
}: {
  m: Message;
  settings: Settings;
  onChoice: (c: string) => void;
  onRegenerate: () => void;
  onBranch: () => void;
  isLast: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const paras = (m.content ?? '').split('\n\n');
  return (
    <div className="fade-up selectable">
      <div
        className={cn('space-y-4 text-fg/95', settings.storyFont === 'serif' && 'story-serif')}
        style={{ fontSize: settings.fontSize, lineHeight: settings.lineHeight }}
      >
        {paras.map((p, i) => (
          <p key={i} className={cn('whitespace-pre-wrap', m.streaming && i === paras.length - 1 && 'caret')}>
            {renderInline(p)}
          </p>
        ))}
      </div>

      {!m.streaming && (
        <>
          <div className="mt-2.5 flex items-center gap-0.5 text-muted">
            {[
              {
                icon: copied ? Check : Copy,
                t: '复制',
                fn: () => {
                  navigator.clipboard?.writeText(m.content ?? '');
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1200);
                },
              },
              { icon: RotateCcw, t: '重新叙述', fn: onRegenerate },
              { icon: GitBranch, t: '从此处分支', fn: onBranch },
            ].map((b) => (
              <button key={b.t} title={b.t} onClick={b.fn} className="grid h-7 w-7 place-items-center rounded-md hover:bg-subtle hover:text-fg">
                <b.icon size={14} />
              </button>
            ))}
            <span className="ml-1.5 text-[12px]">{m.time}</span>
          </div>
          {m.choices && isLast && (
            <div className="mt-4 flex flex-col items-start gap-1.5">
              {m.choices.map((c) => (
                <button
                  key={c}
                  onClick={() => onChoice(c)}
                  className="rounded-xl border border-line px-3.5 py-2 text-left text-[13.5px] text-fg/85 transition hover:bg-subtle hover:text-fg"
                >
                  {c}
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

export function ImageMsg({ m }: { m: Message }) {
  const img = m.image!;
  return (
    <div className="fade-up">
      <div className={cn('group relative h-72 w-56 overflow-hidden rounded-2xl bg-gradient-to-br', img.gradient)}>
        <Silhouette />
        <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/70 to-transparent px-3 pt-10 pb-2.5 text-white">
          <div className="text-[13.5px] font-semibold">{img.name}</div>
          <div className="text-[11.5px] opacity-70">{img.caption}</div>
        </div>
        <button className="absolute top-2 right-2 grid h-7 w-7 place-items-center rounded-lg bg-black/35 text-white opacity-0 backdrop-blur transition group-hover:opacity-100">
          <Download size={14} />
        </button>
      </div>
    </div>
  );
}

export function Silhouette({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 200 260" className={cn('absolute inset-0 h-full w-full', className)} preserveAspectRatio="xMidYMax slice">
      <defs>
        <linearGradient id="rain" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="#fff" stopOpacity="0.35" />
          <stop offset="1" stopColor="#fff" stopOpacity="0" />
        </linearGradient>
      </defs>
      {Array.from({ length: 28 }).map((_, i) => (
        <line key={i} x1={(i * 37) % 200} y1={(i * 53) % 120} x2={((i * 37) % 200) - 6} y2={((i * 53) % 120) + 26} stroke="url(#rain)" strokeWidth="1" />
      ))}
      <circle cx="100" cy="112" r="30" fill="#0b0f1a" opacity="0.85" />
      <path d="M40 260 C45 190 70 160 100 160 C130 160 155 190 160 260 Z" fill="#0b0f1a" opacity="0.85" />
      <path d="M70 108 C70 78 130 78 130 108 C130 96 124 88 100 86 C78 88 70 96 70 108 Z" fill="#0b0f1a" />
    </svg>
  );
}
