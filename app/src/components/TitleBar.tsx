import { useEffect, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, Minus, PanelLeft, Square, X } from 'lucide-react';
import { cn } from '../utils/cn';

export interface MenuItem {
  label?: string;
  shortcut?: string;
  onClick?: () => void;
  disabled?: boolean;
  divider?: boolean;
}

interface Props {
  menus: { label: string; items: MenuItem[] }[];
  onToggleSidebar: () => void;
  canBack: boolean;
  canForward: boolean;
  onBack: () => void;
  onForward: () => void;
  nativeWorld?: { label: string; head_seq: number; event_count: number } | null;
  nativeWorldStatus?: 'checking' | 'open' | 'empty' | 'error';
}

export default function TitleBar(p: Props) {
  const [open, setOpen] = useState<number | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(null);
    };
    window.addEventListener('mousedown', close);
    return () => window.removeEventListener('mousedown', close);
  }, []);

  const iconBtn = 'grid h-7 w-7 place-items-center rounded-md text-muted hover:bg-subtle hover:text-fg disabled:opacity-35 disabled:hover:bg-transparent';
  const currentWindow = () => window.__TAURI__?.window?.getCurrentWindow();

  return (
    <div className="flex h-9 shrink-0 items-center bg-chrome pl-2 text-[13px]">
      <button className={iconBtn} onClick={p.onToggleSidebar} title="切换侧栏 (Ctrl+B)">
        <PanelLeft size={16} />
      </button>
      <button className={iconBtn} onClick={p.onBack} disabled={!p.canBack} title="后退">
        <ArrowLeft size={16} />
      </button>
      <button className={iconBtn} onClick={p.onForward} disabled={!p.canForward} title="前进">
        <ArrowRight size={16} />
      </button>

      <div ref={ref} className="ml-3 flex items-center">
        {p.menus.map((m, i) => (
          <div key={m.label} className="relative">
            <button
              onMouseDown={() => setOpen(open === i ? null : i)}
              onMouseEnter={() => open !== null && setOpen(i)}
              className={cn('rounded-md px-2.5 py-1 text-fg/85 hover:bg-subtle', open === i && 'bg-subtle')}
            >
              {m.label}
            </button>
            {open === i && (
              <div className="fade-up absolute top-8 left-0 z-50 min-w-[210px] rounded-xl border border-line bg-elev p-1 shadow-2xl shadow-black/30">
                {m.items.map((it, k) =>
                  it.divider ? (
                    <div key={k} className="mx-2 my-1 h-px bg-line" />
                  ) : (
                    <button
                      key={k}
                      disabled={it.disabled}
                      onClick={() => {
                        it.onClick?.();
                        setOpen(null);
                      }}
                      className="flex w-full items-center rounded-lg px-2.5 py-1.5 text-left text-[13px] hover:bg-subtle disabled:opacity-40 disabled:hover:bg-transparent"
                    >
                      {it.label}
                      {it.shortcut && <span className="ml-auto pl-6 text-[12px] text-muted">{it.shortcut}</span>}
                    </button>
                  ),
                )}
              </div>
            )}
          </div>
        ))}
      </div>

      <div className="ml-3 flex min-w-0 items-center gap-1.5 text-[11px] text-muted" title={p.nativeWorld?.label ?? '尚未打开 Rust 世界'}>
        <span className={cn('h-1.5 w-1.5 rounded-full', p.nativeWorldStatus === 'open' ? 'bg-emerald-500' : p.nativeWorldStatus === 'error' ? 'bg-rose-500' : 'bg-muted/60')} />
        <span className="max-w-40 truncate">{p.nativeWorldStatus === 'open' ? p.nativeWorld?.label : p.nativeWorldStatus === 'checking' ? '连接世界…' : '演示世界'}</span>
        {p.nativeWorldStatus === 'open' && <span className="tabular-nums text-muted/70">#{p.nativeWorld?.head_seq ?? 0}</span>}
      </div>

      <div
        className="h-full flex-1"
        data-tauri-drag-region
        onMouseDown={(event) => {
          if (event.button === 0 && event.target === event.currentTarget) void currentWindow()?.startDragging();
        }}
        onDoubleClick={() => void currentWindow()?.toggleMaximize()}
      />

      <div className="flex h-full text-fg/70">
        <button type="button" onClick={() => void currentWindow()?.minimize()} className="grid w-11 place-items-center hover:bg-subtle" title="最小化" aria-label="最小化"><Minus size={15} strokeWidth={1.5} /></button>
        <button type="button" onClick={() => void currentWindow()?.toggleMaximize()} className="grid w-11 place-items-center hover:bg-subtle" title="最大化 / 还原" aria-label="最大化或还原"><Square size={12} strokeWidth={1.5} /></button>
        <button type="button" onClick={() => void currentWindow()?.close()} className="grid w-11 place-items-center hover:bg-[#e81123] hover:text-white" title="关闭" aria-label="关闭"><X size={16} strokeWidth={1.5} /></button>
      </div>
    </div>
  );
}
