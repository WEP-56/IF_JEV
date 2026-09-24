import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Search, SquarePen, Settings as SettingsIcon, Pin, MoreHorizontal, Trash2, Copy, Pencil, Sun, Moon, PlusCircle, X, ChevronDown,
} from 'lucide-react';
import type { Story } from '../types';
import { cn } from '../utils/cn';

interface Props {
  stories: Story[];
  activeId: string;
  dark: boolean;
  onSelect: (id: string) => void;
  onNew: () => void;
  onSettings: () => void;
  onDelete: (id: string) => void;
  onPin: (id: string) => void;
  onDuplicate: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onToggleTheme: () => void;
  searchSignal: number;
}

const GROUPS: Story['group'][] = ['置顶', '今天', '最近 7 天', '更早'];

export default function Sidebar(p: Props) {
  const [q, setQ] = useState('');
  const [searching, setSearching] = useState(false);
  const [menu, setMenu] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (p.searchSignal) {
      setSearching(true);
      setTimeout(() => searchRef.current?.focus(), 0);
    }
  }, [p.searchSignal]);

  const filtered = useMemo(() => {
    const k = q.trim();
    if (!k) return p.stories;
    return p.stories.filter((s) => s.title.includes(k) || s.genre.includes(k) || s.messages.some((m) => m.content?.includes(k)));
  }, [q, p.stories]);

  const row = 'flex w-full items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-[13.5px] text-fg/90 hover:bg-subtle';

  return (
    <aside className="flex h-full w-[260px] shrink-0 flex-col bg-chrome">
      <div className="flex items-center px-4 pt-2 pb-3">
        <span className="text-[17px] font-bold tracking-tight">IF</span>
        <ChevronDown size={14} className="mt-0.5 ml-1 text-muted" />
      </div>

      <div className="px-2">
        <button onClick={p.onNew} className={cn(row, 'group')}>
          <SquarePen size={16} className="text-fg/70" />
          新故事
          <PlusCircle size={15} className="ml-auto text-muted opacity-0 group-hover:opacity-100" />
        </button>
        {searching ? (
          <div className="flex items-center gap-2.5 rounded-lg bg-subtle px-2.5 py-[7px]">
            <Search size={16} className="text-fg/70" />
            <input
              ref={searchRef}
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => e.key === 'Escape' && (setQ(''), setSearching(false))}
              placeholder="搜索故事"
              className="min-w-0 flex-1 bg-transparent text-[13.5px] outline-none placeholder:text-muted"
            />
            <button onClick={() => { setQ(''); setSearching(false); }} className="text-muted hover:text-fg">
              <X size={14} />
            </button>
          </div>
        ) : (
          <button
            onClick={() => {
              setSearching(true);
              setTimeout(() => searchRef.current?.focus(), 0);
            }}
            className={row}
          >
            <Search size={16} className="text-fg/70" />
            搜索
            <span className="ml-auto text-[11.5px] text-muted">Ctrl K</span>
          </button>
        )}
      </div>

      <div className="mt-4 flex-1 overflow-y-auto px-2 pb-2" onClick={() => setMenu(null)}>
        {filtered.length === 0 && <div className="px-3 py-6 text-[13px] text-muted">无匹配结果</div>}
        {GROUPS.map((g) => {
          const items = filtered.filter((s) => (s.pinned ? '置顶' : s.group === '置顶' ? '今天' : s.group) === g);
          if (!items.length) return null;
          return (
            <div key={g} className="mb-4">
              <div className="px-2.5 pb-1 text-[12.5px] text-muted">{g}</div>
              {items.map((s) => {
                const active = s.id === p.activeId;
                return (
                  <div key={s.id} className="relative">
                    <div
                      onClick={() => p.onSelect(s.id)}
                      onDoubleClick={() => {
                        setEditing(s.id);
                        setDraft(s.title);
                      }}
                      className={cn(
                        'group flex h-[34px] cursor-default items-center gap-2 rounded-lg px-2.5 text-[13.5px]',
                        active ? 'bg-subtle text-fg' : 'text-fg/85 hover:bg-subtle',
                      )}
                    >
                      {editing === s.id ? (
                        <input
                          autoFocus
                          value={draft}
                          onChange={(e) => setDraft(e.target.value)}
                          onBlur={() => {
                            p.onRename(s.id, draft || s.title);
                            setEditing(null);
                          }}
                          onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
                          className="min-w-0 flex-1 rounded-md bg-bg px-1.5 py-0.5 outline-none ring-1 ring-accent"
                        />
                      ) : (
                        <span className="min-w-0 flex-1 truncate">{s.title}</span>
                      )}
                      {s.pinned && menu !== s.id && <Pin size={12} className="shrink-0 text-muted group-hover:hidden" />}
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          setMenu(menu === s.id ? null : s.id);
                        }}
                        className={cn('shrink-0 rounded p-0.5 text-muted hover:text-fg', menu === s.id ? 'block' : 'hidden group-hover:block')}
                      >
                        <MoreHorizontal size={16} />
                      </button>
                    </div>
                    {menu === s.id && (
                      <div
                        onClick={(e) => e.stopPropagation()}
                        className="fade-up absolute top-9 right-0 z-30 w-40 rounded-xl border border-line bg-elev p-1 text-[13px] shadow-2xl shadow-black/30"
                      >
                        {[
                          { icon: Pin, label: s.pinned ? '取消置顶' : '置顶', fn: () => p.onPin(s.id) },
                          { icon: Pencil, label: '重命名', fn: () => { setEditing(s.id); setDraft(s.title); } },
                          { icon: Copy, label: '复制世界线', fn: () => p.onDuplicate(s.id) },
                        ].map((it) => (
                          <button
                            key={it.label}
                            onClick={() => { it.fn(); setMenu(null); }}
                            className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 hover:bg-subtle"
                          >
                            <it.icon size={14} className="text-muted" /> {it.label}
                          </button>
                        ))}
                        <div className="mx-2 my-1 h-px bg-line" />
                        <button
                          onClick={() => { p.onDelete(s.id); setMenu(null); }}
                          className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-rose-500 hover:bg-rose-500/10"
                        >
                          <Trash2 size={14} /> 删除
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>

      <div className="flex items-center gap-1 px-2 pt-1 pb-2">
        <button onClick={p.onSettings} className="flex flex-1 items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-[13.5px] hover:bg-subtle">
          <SettingsIcon size={16} className="text-fg/70" />
          设置
        </button>
        <button onClick={p.onToggleTheme} className="grid h-8 w-8 place-items-center rounded-lg text-muted hover:bg-subtle hover:text-fg" title="切换主题">
          {p.dark ? <Sun size={15} /> : <Moon size={15} />}
        </button>
      </div>
    </aside>
  );
}
