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

      <div className="h-full flex-1" />

      <div className="flex h-full text-fg/70">
        <span className="grid w-11 place-items-center hover:bg-subtle"><Minus size={15} strokeWidth={1.5} /></span>
        <span className="grid w-11 place-items-center hover:bg-subtle"><Square size={12} strokeWidth={1.5} /></span>
        <span className="grid w-11 place-items-center hover:bg-[#e81123] hover:text-white"><X size={16} strokeWidth={1.5} /></span>
      </div>
    </div>
  );
}
