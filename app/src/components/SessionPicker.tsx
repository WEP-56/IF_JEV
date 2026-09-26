import { Play, Plus, X } from 'lucide-react';
import { formatStamp } from '../library';
import type { SessionRef } from '../session';
import type { LibraryAsset } from '../types';

interface Props {
  asset: LibraryAsset;
  sessions: SessionRef[];
  onResume: (session: SessionRef) => void;
  onNew: () => void;
  onClose: () => void;
}

/**
 * 选中一个已经有会话的世界之后，问一句「继续哪一个」。
 *
 * 一个世界资产可以有多个会话，各自的 `.ifworld` 是分开的（docs/10 §1），
 * 所以这里不是「要不要覆盖」而是「进哪一条历史」。
 */
export default function SessionPicker({ asset, sessions, onResume, onNew, onClose }: Props) {
  return (
    <div className="absolute inset-0 z-40 grid place-items-center bg-black/45 p-6">
      <div className="w-full max-w-xl rounded-xl border border-line bg-elev shadow-2xl">
        <header className="flex items-center gap-3 border-b border-line px-5 py-4">
          <Play size={18} className="text-accent" />
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-[15px] font-semibold">{asset.name}</h2>
            <p className="text-[11px] text-muted">这个世界已经有 {sessions.length} 个会话</p>
          </div>
          <button onClick={onClose} className="grid h-7 w-7 place-items-center rounded-md text-muted hover:bg-subtle" title="关闭" aria-label="关闭">
            <X size={16} />
          </button>
        </header>

        <div className="max-h-[45vh] space-y-2 overflow-y-auto p-4">
          {sessions.map((session) => (
            <button
              key={session.id}
              onClick={() => onResume(session)}
              className="flex w-full items-center gap-3 rounded-lg border border-line p-3 text-left hover:border-accent/60 hover:bg-subtle"
            >
              <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-accent/15 text-accent">
                <Play size={16} />
              </span>
              <span className="min-w-0 flex-1">
                <strong className="block truncate text-[13px]">{session.label}</strong>
                <small className="mt-1 block truncate text-[11px] text-muted">
                  {[formatStamp(session.created_at), session.id].filter(Boolean).join(' · ')}
                </small>
              </span>
            </button>
          ))}
        </div>

        <footer className="flex items-center justify-between gap-3 border-t border-line px-5 py-3">
          <span className="text-[11px] text-muted">各会话的历史各自独立，互不影响</span>
          <button onClick={onNew} className="flex shrink-0 items-center gap-1.5 rounded-lg border border-line px-3 py-2 text-[12px] hover:bg-subtle">
            <Plus size={14} />新建会话
          </button>
        </footer>
      </div>
    </div>
  );
}
