import { BookOpen, FileUp, Layers, Pencil, Plus, Search, Trash2, X } from 'lucide-react';
import { useMemo, useRef, useState } from 'react';
import { originLabel } from '../library';
import type { LibraryAsset } from '../types';

interface Props {
  worlds: LibraryAsset[];
  /** 世界库打不开或读不出来时的原因，直接显示，不用空列表搪塞。 */
  error?: string | null;
  onClose: () => void;
  onCreate: () => void;
  onImport: (file: File) => void;
  onDelete: (id: string) => void;
}

export default function WorldLibrary({ worlds, error, onClose, onCreate, onImport, onDelete }: Props) {
  const [query, setQuery] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const filtered = useMemo(() => {
    const q = query.trim();
    return q ? worlds.filter((w) => `${w.name} ${w.genre} ${w.summary}`.includes(q)) : worlds;
  }, [query, worlds]);

  return (
    <div className="absolute inset-0 z-40 flex flex-col bg-bg">
      <header className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-6">
        <BookOpen size={18} className="text-accent" />
        <div className="min-w-0 flex-1">
          <h1 className="text-[15px] font-semibold">世界库</h1>
          <p className="text-[11px] text-muted">管理可复用的世界资产，创建会话时选择其中一个</p>
        </div>
        <button onClick={onClose} className="grid h-8 w-8 place-items-center rounded-lg text-muted hover:bg-subtle hover:text-fg" title="关闭世界库" aria-label="关闭世界库"><X size={17} /></button>
      </header>
      <div className="flex items-center gap-2 border-b border-line px-6 py-3">
        <div className="flex min-w-0 flex-1 items-center gap-2 rounded-lg border border-line bg-subtle px-3 py-2">
          <Search size={15} className="text-muted" />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="搜索世界" className="min-w-0 flex-1 bg-transparent text-[13px] outline-none placeholder:text-muted" />
        </div>
        <button onClick={onCreate} className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-2 text-[13px] text-white hover:brightness-110"><Plus size={15} />手动撰写</button>
        <button onClick={() => inputRef.current?.click()} className="flex items-center gap-1.5 rounded-lg border border-line px-3 py-2 text-[13px] hover:bg-subtle"><FileUp size={15} />导入</button>
        <input ref={inputRef} type="file" accept=".json,.png" className="hidden" onChange={(e) => { const file = e.target.files?.[0]; if (file) onImport(file); e.currentTarget.value = ''; }} />
      </div>
      {error && <div className="border-b border-rose-500/40 bg-rose-500/10 px-6 py-2 text-[12px] text-rose-400">{error}</div>}
      <main className="grid min-h-0 flex-1 grid-cols-[repeat(auto-fill,minmax(280px,1fr))] content-start gap-3 overflow-y-auto p-6">
        {filtered.map((world) => (
          <article key={world.id} className="flex min-h-[180px] flex-col rounded-xl border border-line bg-elev p-4">
            <div className="flex items-start gap-3">
              <div className="grid h-10 w-10 shrink-0 place-items-center rounded-lg bg-accent/15 text-accent"><BookOpen size={19} /></div>
              <div className="min-w-0 flex-1"><h2 className="truncate text-[14px] font-semibold">{world.name}</h2><p className="mt-0.5 truncate text-[11px] text-muted">{[world.genre, originLabel(world.origin), world.sourceFiles[0]].filter(Boolean).join(' · ')}</p></div>
            </div>
            <p className="mt-3 line-clamp-3 text-[12px] leading-5 text-fg/75">{world.summary || '暂无简介'}</p>
            <div className="mt-auto flex items-center gap-3 pt-4 text-[11px] text-muted">
              <span className="flex items-center gap-1"><Layers size={12} />{world.loreCount} 条设定</span>
              {world.sourceCount > 0 && <span>{world.sourceCount} 个来源</span>}
              {world.sessionCount > 0 && <span>{world.sessionCount} 个会话</span>}
              <span className="ml-auto">{world.updated}</span>
            </div>
            <div className="mt-3 flex gap-1 border-t border-line pt-2"><button className="flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-muted hover:bg-subtle hover:text-fg"><Pencil size={13} />编辑</button><button onClick={() => onDelete(world.id)} disabled={world.sessionCount > 0} className="ml-auto rounded-md px-2 py-1 text-muted hover:bg-rose-500/10 hover:text-rose-500 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-muted" title={world.sessionCount > 0 ? `有 ${world.sessionCount} 个会话引用它，不能删除` : '删除世界'} aria-label={`删除${world.name}`}><Trash2 size={13} /></button></div>
          </article>
        ))}
        {!filtered.length && !error && <div className="col-span-full py-16 text-center text-[13px] text-muted">{worlds.length ? '没有匹配的世界' : '世界库还是空的，先导入一张角色卡或手动撰写'}</div>}
      </main>
    </div>
  );
}
