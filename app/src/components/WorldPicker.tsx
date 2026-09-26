import { BookOpen, FileUp, Plus, X } from 'lucide-react';
import { originLabel } from '../library';
import type { LibraryAsset } from '../types';

interface Props {
  worlds: LibraryAsset[];
  /** 选中一个资产，由外层去世界库读详情再建会话。 */
  onSelect: (id: string) => void;
  onCreateWorld: () => void;
  onImport: (file: File) => void;
  onClose: () => void;
}

export default function WorldPicker({ worlds, onSelect, onCreateWorld, onImport, onClose }: Props) {
  return <div className="absolute inset-0 z-30 grid place-items-center bg-black/35 p-6">
    <div className="w-full max-w-2xl rounded-xl border border-line bg-elev shadow-2xl">
      <header className="flex items-center gap-3 border-b border-line px-5 py-4"><BookOpen size={18} className="text-accent" /><div className="min-w-0 flex-1"><h2 className="text-[15px] font-semibold">新建会话</h2><p className="text-[11px] text-muted">选择一个世界后开始游戏</p></div><button onClick={onClose} className="grid h-7 w-7 place-items-center rounded-md text-muted hover:bg-subtle" title="关闭" aria-label="关闭"><X size={16} /></button></header>
      <div className="max-h-[55vh] overflow-y-auto p-4">{worlds.length ? <div className="grid gap-2 sm:grid-cols-2">{worlds.map((world) => <button key={world.id} onClick={() => onSelect(world.id)} className="flex gap-3 rounded-lg border border-line p-3 text-left hover:border-accent/60 hover:bg-subtle"><span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-accent/15 text-accent"><BookOpen size={17} /></span><span className="min-w-0"><strong className="block truncate text-[13px]">{world.name}</strong><small className="mt-1 block line-clamp-2 text-[11px] text-muted">{[world.genre, originLabel(world.origin), world.loreCount ? `${world.loreCount} 条设定` : '', world.summary || '暂无简介'].filter(Boolean).join(' · ')}</small></span></button>)}</div> : <div className="py-10 text-center text-[13px] text-muted">还没有可用世界</div>}</div>
      <footer className="flex items-center justify-between border-t border-line px-5 py-3"><span className="text-[11px] text-muted">也可以先去世界库准备资产</span><div className="flex gap-2"><button onClick={onCreateWorld} className="flex items-center gap-1.5 rounded-lg border border-line px-3 py-2 text-[12px] hover:bg-subtle"><Plus size={14} />手动撰写</button><label className="flex cursor-pointer items-center gap-1.5 rounded-lg border border-line px-3 py-2 text-[12px] hover:bg-subtle"><FileUp size={14} />导入<input type="file" accept=".json,.png" className="hidden" onChange={(e) => { const file = e.target.files?.[0]; if (file) onImport(file); e.currentTarget.value = ''; }} /></label></div></footer>
    </div>
  </div>;
}
