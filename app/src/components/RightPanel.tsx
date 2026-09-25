import { useMemo, useState, type ReactNode } from 'react';
import { Lock, Unlock, Sparkles, MapPin, ChevronDown, Loader2, GitFork, Undo2 } from 'lucide-react';
import type { Character, MapNode, Story, Settings } from '../types';
import { cn } from '../utils/cn';
import { Silhouette } from './Messages';

type Tab = 'chars' | 'world' | 'map';

interface Props {
  story: Story;
  settings: Settings;
  onUpdateChar: (c: Character) => void;
  onPortrait: (c: Character) => void;
  onBranchFrom: (nodeId: string) => void;
}

export default function RightPanel(p: Props) {
  const [tab, setTab] = useState<Tab>('chars');
  const tabs: { id: Tab; label: string; n?: number }[] = [
    { id: 'chars', label: '角色', n: p.story.characters.length },
    { id: 'world', label: '世界' },
    { id: 'map', label: 'IF 导图', n: p.story.map.length },
  ];
  return (
    <aside className="flex h-full w-[348px] shrink-0 flex-col border-l border-line" onContextMenu={(event) => event.preventDefault()}>
      <div className="flex h-[52px] shrink-0 items-center gap-1 px-3">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={cn(
              'rounded-lg px-3 py-1.5 text-[13.5px] transition',
              tab === t.id ? 'bg-subtle font-medium text-fg' : 'text-muted hover:text-fg',
            )}
          >
            {t.label}
            {t.n !== undefined && <span className="ml-1.5 text-[12px] text-muted">{t.n}</span>}
          </button>
        ))}
      </div>
      <div className="flex-1 overflow-y-auto px-3 pb-4">
        {tab === 'chars' && <CharsTab {...p} />}
        {tab === 'world' && <WorldTab story={p.story} />}
        {tab === 'map' && <MapTab story={p.story} onBranchFrom={p.onBranchFrom} />}
      </div>
    </aside>
  );
}

/* ------------------------------ 角色 ------------------------------ */

function CharsTab({ story, settings, onUpdateChar, onPortrait }: Props) {
  const [open, setOpen] = useState<string | null>(story.characters[0]?.id ?? null);
  const [gen, setGen] = useState<string | null>(null);

  if (!story.characters.length) return <Empty text="还没有角色" />;

  return (
    <div className="space-y-2">
      {story.characters.map((c) => {
        const isOpen = open === c.id;
        return (
          <div key={c.id} className={cn('overflow-hidden rounded-2xl transition', isOpen ? 'bg-elev' : 'hover:bg-subtle')}>
            <button onClick={() => setOpen(isOpen ? null : c.id)} className="flex w-full items-center gap-3 px-3 py-2.5 text-left">
              <div className={cn('grid h-9 w-9 shrink-0 place-items-center rounded-full bg-gradient-to-br text-[14px] font-semibold text-white', c.gradient)}>
                {c.initials}
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5 text-[14px] font-semibold">
                  {c.name}
                  {c.locked && <Lock size={11} className="text-muted" />}
                </div>
                <div className="truncate text-[12.5px] text-muted">{c.title}</div>
              </div>
              <ChevronDown size={15} className={cn('text-muted transition', isOpen && 'rotate-180')} />
            </button>

            {isOpen && (
              <div className="space-y-3.5 px-3 pb-3">
                {c.portrait && settings.imageEnabled && (
                  <div className={cn('relative h-48 overflow-hidden rounded-xl bg-gradient-to-br', c.gradient)}>
                    <Silhouette />
                  </div>
                )}
                <p className="selectable text-[13px] leading-relaxed text-fg/80">{c.bio}</p>
                <div className="flex flex-wrap gap-1">
                  {c.tags.map((t) => (
                    <span key={t} className="rounded-md bg-subtle px-2 py-0.5 text-[12px] text-fg/75">
                      {t}
                    </span>
                  ))}
                </div>
                <div className="space-y-2.5">
                  {c.stats.map((s) => (
                    <div key={s.label}>
                      <div className="mb-1 flex justify-between text-[12.5px]">
                        <span className="text-muted">{s.label}</span>
                        <span className="tabular-nums">{s.value}</span>
                      </div>
                      <div className="h-1 overflow-hidden rounded-full bg-subtle">
                        <div className="h-full rounded-full bg-fg/60" style={{ width: `${s.value}%` }} />
                      </div>
                    </div>
                  ))}
                </div>
                <dl className="space-y-1.5 rounded-xl bg-bg/60 px-3 py-2.5 text-[12.5px]">
                  <div className="flex gap-3">
                    <dt className="w-14 shrink-0 text-muted">位置</dt>
                    <dd className="flex items-center gap-1">
                      <MapPin size={12} className="text-muted" />
                      {c.location}
                    </dd>
                  </div>
                  {c.state.map((s) => (
                    <div key={s.k} className="flex gap-3">
                      <dt className="w-14 shrink-0 text-muted">{s.k}</dt>
                      <dd className="text-fg/90">{s.v}</dd>
                    </div>
                  ))}
                </dl>
                <div className="flex gap-2">
                  <button
                    onClick={() => onUpdateChar({ ...c, locked: !c.locked })}
                    className="flex flex-1 items-center justify-center gap-1.5 rounded-xl border border-line py-2 text-[13px] hover:bg-subtle"
                  >
                    {c.locked ? <Unlock size={14} /> : <Lock size={14} />}
                    {c.locked ? '解除定型' : '定型'}
                  </button>
                  {settings.imageEnabled && (
                    <button
                      disabled={!c.locked || gen === c.id}
                      onClick={() => {
                        setGen(c.id);
                        setTimeout(() => {
                          onPortrait(c);
                          setGen(null);
                        }, 1800);
                      }}
                      className="flex flex-1 items-center justify-center gap-1.5 rounded-xl bg-fg py-2 text-[13px] font-medium text-bg disabled:opacity-30"
                    >
                      {gen === c.id ? <Loader2 size={14} className="animate-spin" /> : <Sparkles size={14} />}
                      {gen === c.id ? '生成中' : c.portrait ? '重绘立绘' : '生成立绘'}
                    </button>
                  )}
                </div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

/* ------------------------------ 世界 ------------------------------ */

function WorldTab({ story }: { story: Story }) {
  const w = story.world;
  if (!w.vars.length && !w.rules.length) return <Empty text="世界尚未成形" />;
  const arrow = (t: string) => (t === 'up' ? <span className="text-rose-400">↑</span> : t === 'down' ? <span className="text-sky-400">↓</span> : null);

  return (
    <div className="space-y-2.5">
      <div className="rounded-2xl bg-elev p-4">
        <div className="text-[18px] font-semibold">{w.name}</div>
        <div className="mt-0.5 text-[12.5px] text-muted">
          {[w.genre, w.era].filter(Boolean).join(' · ')}
        </div>
        {w.summary && <p className="selectable mt-3 text-[13px] leading-relaxed text-fg/80">{w.summary}</p>}
      </div>

      {w.vars.length > 0 && (
        <Card title="状态">
          <div className="space-y-3">
            {w.vars.map((v) => (
              <div key={v.label}>
                <div className="mb-1 flex items-center justify-between text-[13px]">
                  <span className="text-muted">{v.label}</span>
                  <span className="tabular-nums">
                    {v.value} {arrow(v.trend)}
                  </span>
                </div>
                {v.pct !== undefined && (
                  <div className="h-1 overflow-hidden rounded-full bg-subtle">
                    <div
                      className={cn('h-full rounded-full', v.pct > 75 && v.trend === 'up' ? 'bg-rose-400' : 'bg-fg/60')}
                      style={{ width: `${v.pct}%` }}
                    />
                  </div>
                )}
              </div>
            ))}
          </div>
        </Card>
      )}

      {w.rules.length > 0 && (
        <Card title="规则">
          <ol className="space-y-2">
            {w.rules.map((r, i) => (
              <li key={i} className="flex gap-2.5 text-[13px] leading-relaxed">
                <span className="w-4 shrink-0 text-right text-muted tabular-nums">{i + 1}</span>
                <span className="text-fg/85">{r}</span>
              </li>
            ))}
          </ol>
        </Card>
      )}

      {w.factions.length > 0 && (
        <Card title="势力">
          <div className="space-y-2">
            {w.factions.map((f) => (
              <div key={f.name} className="flex items-center gap-2.5 text-[13px]">
                <span className="flex-1 truncate">{f.name}</span>
                <span className="text-[12px] text-muted">{f.attitude}</span>
                <div className="h-1 w-14 overflow-hidden rounded-full bg-subtle">
                  <div className="h-full rounded-full bg-fg/60" style={{ width: `${f.power}%` }} />
                </div>
              </div>
            ))}
          </div>
        </Card>
      )}

      {w.locations.length > 0 && (
        <Card title="地点">
          <div className="-mx-1 space-y-0.5">
            {w.locations.map((l) => (
              <div key={l.name} className="flex items-center gap-2.5 rounded-lg px-1 py-1.5 text-[13px]">
                <span className={cn('h-1.5 w-1.5 shrink-0 rounded-full', l.danger > 70 ? 'bg-rose-400' : l.danger > 40 ? 'bg-amber-400' : 'bg-emerald-400')} />
                <span className="flex-1 truncate">{l.name}</span>
                <span className="text-[12px] text-muted">{l.status}</span>
              </div>
            ))}
          </div>
        </Card>
      )}
    </div>
  );
}

function Card({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="rounded-2xl bg-elev p-4">
      <div className="mb-3 text-[12.5px] text-muted">{title}</div>
      {children}
    </div>
  );
}

function Empty({ text }: { text: string }) {
  return <div className="py-16 text-center text-[13px] text-muted">{text}</div>;
}

/* ------------------------------ IF 导图 ------------------------------ */

const NW = 96;
const NH = 44;
const GX = 12;
const GY = 30;

function layout(nodes: MapNode[]) {
  const kids: Record<string, MapNode[]> = {};
  nodes.forEach((n) => {
    if (n.parent) (kids[n.parent] ??= []).push(n);
  });
  Object.values(kids).forEach((arr) => arr.sort((a, b) => Number(b.main) - Number(a.main)));
  const pos: Record<string, { x: number; y: number }> = {};
  let leaf = 0;
  const walk = (n: MapNode, d: number): number => {
    const ch = kids[n.id] ?? [];
    let x: number;
    if (!ch.length) x = leaf++;
    else {
      const xs = ch.map((c) => walk(c, d + 1));
      x = (xs[0] + xs[xs.length - 1]) / 2;
    }
    pos[n.id] = { x, y: d };
    return x;
  };
  const root = nodes.find((n) => !n.parent);
  if (root) walk(root, 0);
  const maxX = Math.max(0, ...Object.values(pos).map((q) => q.x));
  const maxY = Math.max(0, ...Object.values(pos).map((q) => q.y));
  return { pos, w: (maxX + 1) * (NW + GX) + 20, h: (maxY + 1) * (NH + GY) + 10 };
}

const TYPE: Record<MapNode['type'], { label: string; dot: string }> = {
  origin: { label: '起源', dot: 'bg-fg' },
  event: { label: '事件', dot: 'bg-accent' },
  state: { label: '状态', dot: 'bg-emerald-400' },
  branch: { label: '平行线', dot: 'bg-violet-400' },
};

function MapTab({ story, onBranchFrom }: { story: Story; onBranchFrom: (id: string) => void }) {
  const { pos, w, h } = useMemo(() => layout(story.map), [story.map]);
  const mains = story.map.filter((n) => n.main);
  const current = mains[mains.length - 1]?.id;
  const [sel, setSel] = useState<string | null>(null);
  const selected = story.map.find((n) => n.id === (sel ?? current));
  const P = (id: string) => ({ x: pos[id].x * (NW + GX) + 10, y: pos[id].y * (NH + GY) + 5 });
  const W = Math.max(w, 322);

  return (
    <div className="space-y-2.5">
      <div className="overflow-auto rounded-2xl bg-elev" style={{ maxHeight: 420 }}>
        <div className="relative" style={{ width: W, height: h + 10 }}>
          <svg className="absolute inset-0" width={W} height={h + 10}>
            {story.map
              .filter((n) => n.parent && pos[n.parent])
              .map((n) => {
                const a = P(n.parent!);
                const b = P(n.id);
                const x1 = a.x + NW / 2, y1 = a.y + NH, x2 = b.x + NW / 2, y2 = b.y;
                const my = (y1 + y2) / 2;
                return (
                  <path
                    key={n.id}
                    d={`M${x1} ${y1} C${x1} ${my}, ${x2} ${my}, ${x2} ${y2}`}
                    fill="none"
                    stroke="var(--fg)"
                    strokeOpacity={n.main ? 0.4 : 0.18}
                    strokeWidth={n.main ? 1.5 : 1}
                    strokeDasharray={n.main ? undefined : '3 4'}
                  />
                );
              })}
          </svg>
          {story.map.map((n) => {
            const pt = P(n.id);
            const isSel = (sel ?? current) === n.id;
            return (
              <button
                key={n.id}
                onClick={() => setSel(n.id)}
                style={{ left: pt.x, top: pt.y, width: NW, height: NH }}
                className={cn(
                  'absolute flex flex-col justify-center rounded-xl px-2.5 text-left transition',
                  isSel ? 'bg-fg text-bg' : 'bg-bg hover:bg-subtle',
                  !n.main && !isSel && 'opacity-70',
                )}
              >
                <span className={cn('flex items-center gap-1 text-[10.5px]', isSel ? 'opacity-70' : 'text-muted')}>
                  <span className={cn('h-1.5 w-1.5 rounded-full', TYPE[n.type].dot)} />
                  {n.day}
                  {n.id === current && <span className="ml-auto">当前</span>}
                </span>
                <span className="truncate text-[12.5px] font-medium">{n.label}</span>
              </button>
            );
          })}
        </div>
      </div>

      {selected && (
        <div className="fade-up rounded-2xl bg-elev p-4" key={selected.id}>
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-semibold">{selected.label}</span>
            <span className="ml-auto text-[12px] text-muted">
              {TYPE[selected.type].label} · {selected.day}
            </span>
          </div>
          <p className="selectable mt-2 text-[13px] leading-relaxed text-fg/80">{selected.desc}</p>
          <div className="mt-3 flex gap-2">
            <button
              onClick={() => onBranchFrom(selected.id)}
              className="flex flex-1 items-center justify-center gap-1.5 rounded-xl border border-line py-2 text-[13px] hover:bg-subtle"
            >
              <GitFork size={14} /> 分支
            </button>
            <button className="flex flex-1 items-center justify-center gap-1.5 rounded-xl border border-line py-2 text-[13px] hover:bg-subtle">
              <Undo2 size={14} /> 回溯
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
