import { useState, type ReactNode } from 'react';
import {
  X, Bot, Cpu, ImageIcon, HardDrive, Palette, Info, Eye, EyeOff, Check, Loader2, PlugZap, Sun, Moon, Monitor, FolderOpen, Download, Upload, Trash2,
} from 'lucide-react';
import type { Settings } from '../types';
import { ACCENTS } from '../data';
import { cn } from '../utils/cn';

type Section = 'llm' | 'jev' | 'image' | 'storage' | 'appearance' | 'about';

interface Props {
  settings: Settings;
  onChange: (s: Settings) => void;
  onClose: () => void;
  onClearData: () => void;
}

export default function SettingsModal({ settings: s, onChange, onClose, onClearData }: Props) {
  const [sec, setSec] = useState<Section>('llm');
  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => onChange({ ...s, [k]: v });
  const setLlm = (patch: Partial<Settings['llm']>) => set('llm', { ...s.llm, ...patch });
  const setJev = (patch: Partial<Settings['jev']>) => set('jev', { ...s.jev, ...patch });
  const setImg = (patch: Partial<Settings['image']>) => set('image', { ...s.image, ...patch });
  const setSto = (patch: Partial<Settings['storage']>) => set('storage', { ...s.storage, ...patch });

  const nav: { group: string; items: { id: Section; label: string; icon: typeof Bot; badge?: string }[] }[] = [
    {
      group: '提供商',
      items: [
        { id: 'llm', label: '大语言模型', icon: Bot },
        { id: 'jev', label: 'JEV 主脑', icon: Cpu },
        { id: 'image', label: '生图模型', icon: ImageIcon, badge: '可选' },
      ],
    },
    {
      group: '通用',
      items: [
        { id: 'storage', label: '存储', icon: HardDrive },
        { id: 'appearance', label: '外观', icon: Palette },
        { id: 'about', label: '关于', icon: Info },
      ],
    },
  ];

  const titles: Record<Section, string> = {
    llm: '大语言模型',
    jev: 'JEV 主脑',
    image: '生图模型',
    storage: '存储',
    appearance: '外观',
    about: '关于 IF',
  };

  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/45 p-6" onMouseDown={onClose}>
      <div
        onMouseDown={(e) => e.stopPropagation()}
        className="fade-up flex h-[min(660px,90vh)] w-[min(900px,95vw)] overflow-hidden rounded-2xl border border-line bg-bg shadow-2xl shadow-black/40"
      >
        <nav className="w-52 shrink-0 bg-chrome p-2.5 pt-4">
          {nav.map((g) => (
            <div key={g.group} className="mb-3">
              <div className="px-2.5 pb-1 text-[12.5px] text-muted">{g.group}</div>
              {g.items.map((it) => (
                <button
                  key={it.id}
                  onClick={() => setSec(it.id)}
                  className={cn(
                    'flex w-full items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-[13.5px]',
                    sec === it.id ? 'bg-subtle' : 'text-fg/80 hover:bg-subtle/60',
                  )}
                >
                  <it.icon size={14} className={sec === it.id ? 'text-accent' : 'text-muted'} />
                  {it.label}
                  {it.badge && <span className="ml-auto text-[10.5px] text-muted">{it.badge}</span>}
                </button>
              ))}
            </div>
          ))}
        </nav>

        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-center px-7 pt-5 pb-2">
            <h2 className="text-[15px] font-semibold">{titles[sec]}</h2>
            <button onClick={onClose} className="ml-auto rounded p-1 text-muted hover:bg-subtle hover:text-fg">
              <X size={16} />
            </button>
          </div>
          <div className="flex-1 overflow-y-auto px-7 pb-4">
            {sec === 'llm' && (
              <>
                <Field label="提供商">
                  <Select
                    value={s.llm.provider}
                    onChange={(v) => setLlm({ provider: v })}
                    options={['Anthropic', 'OpenAI', 'DeepSeek', 'Google Gemini', 'Ollama（本地）', 'OpenAI 兼容']}
                  />
                </Field>
                <Field label="API 地址">
                  <Input value={s.llm.base} onChange={(v) => setLlm({ base: v })} mono />
                </Field>
                <Field label="API Key">
                  <Secret value={s.llm.key} onChange={(v) => setLlm({ key: v })} />
                </Field>
                <Field label="模型">
                  <Select
                    value={s.llm.model}
                    onChange={(v) => setLlm({ model: v })}
                    options={['claude-sonnet-4', 'claude-opus-4', 'gpt-4.1', 'deepseek-chat', 'gemini-2.5-pro', 'qwen3-72b']}
                  />
                </Field>
                <Field label="叙事风格">
                  <Segmented value={s.llm.style} onChange={(v) => setLlm({ style: v })} options={['文学叙事', '轻小说', '剧本体', '白描']} />
                </Field>
                <Field label="温度">
                  <Slider value={s.llm.temperature} min={0} max={1.5} step={0.05} onChange={(v) => setLlm({ temperature: v })} fmt={(v) => v.toFixed(2)} />
                </Field>
                <Field label="单次最大输出">
                  <Slider value={s.llm.maxTokens} min={256} max={8192} step={256} onChange={(v) => setLlm({ maxTokens: v })} fmt={(v) => `${v}`} />
                </Field>
                <Field label="上下文窗口">
                  <Slider value={s.llm.context} min={8} max={1000} step={8} onChange={(v) => setLlm({ context: v })} fmt={(v) => `${v}k`} />
                </Field>
                <TestRow label="测试连接" />
              </>
            )}

            {sec === 'jev' && (
              <>
                <Field label="服务地址">
                  <Input value={s.jev.endpoint} onChange={(v) => setJev({ endpoint: v })} mono />
                </Field>
                <Field label="访问令牌">
                  <Secret value={s.jev.key} onChange={(v) => setJev({ key: v })} placeholder="（本地部署可留空）" />
                </Field>
                <Field label="模型版本">
                  <Select value={s.jev.model} onChange={(v) => setJev({ model: v })} options={['JEV-2-Pro (32B)', 'JEV-2 (8B)', 'JEV-2-Lite (3B)', 'JEV-1.5']} />
                </Field>
                <Field label="时间粒度">
                  <Segmented value={s.jev.tick} onChange={(v) => setJev({ tick: v })} options={['小时', '天', '周', '月']} />
                </Field>
                <Field label="因果推演深度">
                  <Slider value={s.jev.depth} min={1} max={8} step={1} onChange={(v) => setJev({ depth: v })} fmt={(v) => `${v} 层`} />
                </Field>
                <Field label="一致性严格度">
                  <Slider value={s.jev.strict} min={0} max={100} step={5} onChange={(v) => setJev({ strict: v })} fmt={(v) => `${v}%`} />
                </Field>
                <Field label="随机种子">
                  <Input value={s.jev.seed} onChange={(v) => setJev({ seed: v })} mono />
                </Field>
                <Field label="记录平行分支">
                  <Toggle value={s.jev.autoBranch} onChange={(v) => setJev({ autoBranch: v })} />
                </Field>
                <Field label="显示推演卡片">
                  <Toggle value={s.showJev} onChange={(v) => set('showJev', v)} />
                </Field>
                <Field label="推演卡片默认">
                  <Segmented value={s.jevDetail === 'full' ? '展开' : '折叠'} onChange={(v) => set('jevDetail', v === '展开' ? 'full' : 'compact')} options={['展开', '折叠']} />
                </Field>
                <TestRow label="测试连接" />
              </>
            )}

            {sec === 'image' && (
              <>
                <Field label="启用生图模型">
                  <Toggle value={s.imageEnabled} onChange={(v) => set('imageEnabled', v)} />
                </Field>
                <div className={cn(!s.imageEnabled && 'pointer-events-none opacity-40')}>
                  <Field label="提供商">
                    <Select
                      value={s.image.provider}
                      onChange={(v) => setImg({ provider: v })}
                      options={['ComfyUI（本地）', 'Stable Diffusion WebUI', 'OpenAI Images', 'Flux API']}
                    />
                  </Field>
                  <Field label="服务地址">
                    <Input value={s.image.endpoint} onChange={(v) => setImg({ endpoint: v })} mono />
                  </Field>
                  <Field label="API Key">
                    <Secret value={s.image.key} onChange={(v) => setImg({ key: v })} placeholder="（本地无需）" />
                  </Field>
                  <Field label="Checkpoint">
                    <Input value={s.image.model} onChange={(v) => setImg({ model: v })} mono />
                  </Field>
                  <Field label="画风预设">
                    <Segmented value={s.image.style} onChange={(v) => setImg({ style: v })} options={['写实电影感', '厚涂插画', '日系动漫', '水墨']} />
                  </Field>
                  <Field label="尺寸">
                    <Select value={s.image.size} onChange={(v) => setImg({ size: v })} options={['512 × 768', '768 × 1024', '1024 × 1024', '1216 × 832']} />
                  </Field>
                  <Field label="定型后自动生成立绘">
                    <Toggle value={s.image.autoOnLock} onChange={(v) => setImg({ autoOnLock: v })} />
                  </Field>
                  <TestRow label="生成测试图" />
                </div>
              </>
            )}

            {sec === 'storage' && (
              <>
                <div className="my-4 rounded-xl border border-line p-3.5">
                  <div className="flex h-2 overflow-hidden rounded-full bg-subtle">
                    <div className="bg-accent" style={{ width: '18%' }} />
                    <div className="bg-sky-500" style={{ width: '9%' }} />
                    <div className="bg-violet-500" style={{ width: '31%' }} />
                  </div>
                  <div className="mt-2.5 flex gap-4 text-[12px] text-muted">
                    <span>对话 56MB</span>
                    <span>快照 28MB</span>
                    <span>图片 228MB</span>
                    <span className="ml-auto">312MB</span>
                  </div>
                </div>
                <Field label="存储后端">
                  <Segmented value={s.storage.backend} onChange={(v) => setSto({ backend: v })} options={['本地文件夹', 'SQLite', 'WebDAV']} />
                </Field>
                <Field label="数据目录">
                  <div className="flex gap-2">
                    <Input value={s.storage.path} onChange={(v) => setSto({ path: v })} mono />
                    <button className="flex shrink-0 items-center gap-1 rounded-lg border border-line px-2.5 text-[12.5px] hover:bg-subtle">
                      <FolderOpen size={13} /> 浏览
                    </button>
                  </div>
                </Field>
                <Field label="自动保存">
                  <Toggle value={s.storage.autosave} onChange={(v) => setSto({ autosave: v })} />
                </Field>
                <Field label="快照数量">
                  <Slider value={s.storage.snapshots} min={5} max={200} step={5} onChange={(v) => setSto({ snapshots: v })} fmt={(v) => `${v}`} />
                </Field>
                <Field label="加密本地数据">
                  <Toggle value={s.storage.encrypt} onChange={(v) => setSto({ encrypt: v })} />
                </Field>
                <Field label="导入 / 导出">
                  <div className="flex gap-2">
                    <Btn icon={Upload}>导入世界</Btn>
                    <Btn icon={Download}>导出全部</Btn>
                  </div>
                </Field>
                <Field label="危险操作">
                  <button
                    onClick={onClearData}
                    className="flex items-center gap-1.5 rounded-lg border border-rose-500/40 px-2.5 py-1 text-[12.5px] text-rose-500 hover:bg-rose-500/10"
                  >
                    <Trash2 size={13} /> 重置为示例数据
                  </button>
                </Field>
              </>
            )}

            {sec === 'appearance' && (
              <>
                <Field label="主题">
                  <div className="flex gap-2">
                    {[
                      { v: 'light' as const, l: '浅色', i: Sun },
                      { v: 'dark' as const, l: '深色', i: Moon },
                      { v: 'system' as const, l: '跟随系统', i: Monitor },
                    ].map((t) => (
                      <button
                        key={t.v}
                        onClick={() => set('theme', t.v)}
                        className={cn(
                          'flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-[12.5px]',
                          s.theme === t.v ? 'border-accent bg-accent/10 text-accent' : 'border-line hover:bg-subtle',
                        )}
                      >
                        <t.i size={13} /> {t.l}
                      </button>
                    ))}
                  </div>
                </Field>
                <Field label="强调色">
                  <div className="flex gap-2">
                    {ACCENTS.map((a) => (
                      <button
                        key={a.value}
                        title={a.name}
                        onClick={() => set('accent', a.value)}
                        className="grid h-6 w-6 place-items-center rounded-full ring-offset-2 ring-offset-elev transition"
                        style={{ background: a.value, boxShadow: s.accent === a.value ? `0 0 0 2px var(--elev), 0 0 0 3.5px ${a.value}` : undefined }}
                      >
                        {s.accent === a.value && <Check size={13} className="text-white" />}
                      </button>
                    ))}
                  </div>
                </Field>
                <Field label="正文字体">
                  <Segmented value={s.storyFont === 'serif' ? '宋体' : '黑体'} onChange={(v) => set('storyFont', v === '宋体' ? 'serif' : 'sans')} options={['宋体', '黑体']} />
                </Field>
                <Field label="正文字号">
                  <Slider value={s.fontSize} min={13} max={22} step={1} onChange={(v) => set('fontSize', v)} fmt={(v) => `${v}px`} />
                </Field>
                <Field label="行距">
                  <Slider value={s.lineHeight} min={1.4} max={2.4} step={0.1} onChange={(v) => set('lineHeight', v)} fmt={(v) => v.toFixed(1)} />
                </Field>
                <Field label="阅读区宽度">
                  <Segmented
                    value={{ narrow: '窄', normal: '适中', wide: '宽' }[s.chatWidth]}
                    onChange={(v) => set('chatWidth', v === '窄' ? 'narrow' : v === '宽' ? 'wide' : 'normal')}
                    options={['窄', '适中', '宽']}
                  />
                </Field>
                <div className="my-4 rounded-xl border border-line bg-bg p-5">
                  <p className={cn('text-fg/90', s.storyFont === 'serif' && 'story-serif')} style={{ fontSize: s.fontSize, lineHeight: s.lineHeight }}>
                    雨是在周六凌晨开始下的。<strong>她还不知道，这场雨会下整整三十天。</strong>
                  </p>
                </div>
              </>
            )}

            {sec === 'about' && (
              <div className="py-5">
                <div className="flex items-center gap-3">
                  <div className="grid h-12 w-12 place-items-center rounded-xl bg-accent text-[20px] font-black italic text-white">IF</div>
                  <div>
                    <div className="text-[16px] font-semibold">IF</div>
                    <div className="text-[12.5px] text-muted">v0.1.0 · 本地开源 · 前端预览</div>
                  </div>
                </div>
                <div className="mt-5 grid grid-cols-3 gap-2 text-[12.5px]">
                  {[
                    ['IF', '事件'],
                    ['JEV', '状态机'],
                    ['LLM', '叙事'],
                  ].map(([a, b]) => (
                    <div key={a} className="rounded-xl border border-line px-3 py-2">
                      <div className="text-[14px] font-bold text-accent">{a}</div>
                      <div className="mt-0.5 text-muted">{b}</div>
                    </div>
                  ))}
                </div>
                <p className="mt-5 text-[12.5px] leading-relaxed text-muted">
                  无账号 · 无遥测 · 数据存于本地。本预览不含模型调用。
                </p>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[170px_1fr] items-center gap-6 border-b border-line py-3.5 last:border-0">
      <div className="text-[13.5px]">{label}</div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

const inputCls = 'h-8 w-full rounded-lg border border-line bg-elev px-2.5 text-[13px] outline-none focus:border-accent/60';

function Input({ value, onChange, mono, placeholder }: { value: string; onChange: (v: string) => void; mono?: boolean; placeholder?: string }) {
  return <input value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} className={cn(inputCls, mono && 'font-mono text-[11.5px]')} />;
}

function Secret({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder?: string }) {
  const [show, setShow] = useState(false);
  return (
    <div className="relative">
      <input
        type={show ? 'text' : 'password'}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className={cn(inputCls, 'pr-8 font-mono text-[11.5px]')}
      />
      <button onClick={() => setShow(!show)} className="absolute top-1/2 right-1.5 -translate-y-1/2 p-1 text-muted hover:text-fg">
        {show ? <EyeOff size={13} /> : <Eye size={13} />}
      </button>
    </div>
  );
}

function Select({ value, onChange, options }: { value: string; onChange: (v: string) => void; options: string[] }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)} className={cn(inputCls, 'cursor-pointer')}>
      {options.map((o) => (
        <option key={o}>{o}</option>
      ))}
    </select>
  );
}

function Toggle({ value, onChange }: { value: boolean; onChange: (v: boolean) => void }) {
  return (
    <button onClick={() => onChange(!value)} className={cn('relative h-[18px] w-8 rounded-full transition', value ? 'bg-accent' : 'bg-subtle ring-1 ring-line')}>
      <span className={cn('absolute top-[2px] h-[14px] w-[14px] rounded-full bg-white shadow transition-all', value ? 'left-[16px]' : 'left-[2px]')} />
    </button>
  );
}

function Slider({ value, min, max, step, onChange, fmt }: { value: number; min: number; max: number; step: number; onChange: (v: number) => void; fmt: (v: number) => string }) {
  return (
    <div className="flex items-center gap-3">
      <input type="range" min={min} max={max} step={step} value={value} onChange={(e) => onChange(+e.target.value)} className="flex-1" />
      <span className="w-16 text-right text-[12.5px] tabular-nums text-muted">{fmt(value)}</span>
    </div>
  );
}

function Segmented({ value, onChange, options }: { value: string; onChange: (v: string) => void; options: string[] }) {
  return (
    <div className="inline-flex rounded-lg bg-subtle p-[3px]">
      {options.map((o) => (
        <button
          key={o}
          onClick={() => onChange(o)}
          className={cn('rounded-md px-3 py-1 text-[12.5px] transition', value === o ? 'bg-bg text-fg shadow-sm dark:bg-white/12' : 'text-muted hover:text-fg')}
        >
          {o}
        </button>
      ))}
    </div>
  );
}

function Btn({ icon: Icon, children }: { icon: typeof Bot; children: ReactNode }) {
  return (
    <button className="flex items-center gap-1.5 rounded-lg border border-line px-2.5 py-1 text-[12.5px] hover:bg-subtle">
      <Icon size={13} /> {children}
    </button>
  );
}

function TestRow({ label }: { label: string }) {
  const [st, setSt] = useState<'idle' | 'loading' | 'ok'>('idle');
  return (
    <div className="flex items-center gap-3 py-3">
      <button
        onClick={() => {
          setSt('loading');
          setTimeout(() => setSt('ok'), 1000);
        }}
        className="flex items-center gap-1.5 rounded-lg border border-line px-2.5 py-1 text-[12.5px] hover:bg-subtle"
      >
        {st === 'loading' ? <Loader2 size={13} className="animate-spin" /> : <PlugZap size={13} />}
        {label}
      </button>
      {st === 'ok' && (
        <span className="fade-up flex items-center gap-1 text-[12.5px] text-emerald-600 dark:text-emerald-400">
          <Check size={13} /> 连接成功 · {180 + Math.floor(Math.random() * 300)}ms
        </span>
      )}
    </div>
  );
}
