import { AlertTriangle, BookOpen, Check, FileText, Sparkles, X } from 'lucide-react';
import { useState } from 'react';
import { formatLabel, logicLabel, sectionLabel, type ImportedWorld } from '../import';

interface Props {
  imported: ImportedWorld;
  /** 用户确认后写入世界库。 */
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 导入预览：把 Rust 侧解析出的世界资产原样呈现给用户审阅（docs/10 §7 第 4 步）。
 * 这里不改进导入内容，只显示映射结果、来源方言与需要人工确认的警告。
 */
export default function WorldImportPreview({ imported, onConfirm, onCancel }: Props) {
  const [tab, setTab] = useState<'summary' | 'lore'>('summary');
  const character = imported.characters[0];
  const constantCount = imported.lore.filter((entry) => entry.constant).length;
  const disabledCount = imported.lore.filter((entry) => !entry.enabled).length;

  return (
    <div className="absolute inset-0 z-50 grid place-items-center bg-black/45 p-6">
      <div className="flex max-h-[86vh] w-full max-w-3xl flex-col rounded-xl border border-line bg-elev shadow-2xl">
        <header className="flex items-center gap-3 border-b border-line px-5 py-4">
          <BookOpen size={18} className="text-accent" />
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-[15px] font-semibold">导入预览 · {imported.name}</h2>
            <p className="text-[11px] text-muted">
              {formatLabel(imported.source_format)}
              {imported.spec_version ? ` ${imported.spec_version}` : ''} · 来源 {imported.source_kind}
              {imported.source_file ? ` · ${imported.source_file}` : ''}
            </p>
          </div>
          <button onClick={onCancel} className="grid h-7 w-7 place-items-center rounded-md text-muted hover:bg-subtle" title="关闭" aria-label="关闭预览">
            <X size={16} />
          </button>
        </header>

        <div className="flex items-center gap-1 border-b border-line px-5 py-2">
          {([
            ['summary', `概览（${imported.characters.length} 角色 / ${imported.lore.length} 条目）`],
            ['lore', '设定条目'],
          ] as const).map(([key, label]) => (
            <button
              key={key}
              onClick={() => setTab(key)}
              className={`rounded-md px-3 py-1.5 text-[12px] ${tab === key ? 'bg-accent/15 text-accent' : 'text-muted hover:bg-subtle'}`}
            >
              {label}
            </button>
          ))}
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {tab === 'summary' ? (
            <div className="grid gap-4">
              {(imported.warnings.length > 0 || imported.macros.length > 0) && (
                <section className="grid gap-2">
                  {imported.macros.length > 0 && (
                    <div className="flex gap-2 rounded-lg border border-line bg-subtle px-3 py-2">
                      <Sparkles size={15} className="mt-0.5 shrink-0 text-accent" />
                      <p className="text-[12px] leading-5 text-fg/80">
                        检测到宏：<span className="font-mono">{imported.macros.join(' ')}</span>
                        {imported.macros.includes('{{user}}') && ' —— 按 D14，{{user}} 会转成由世界推演的主角。'}
                      </p>
                    </div>
                  )}
                  {imported.warnings.length > 0 && (
                    <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2">
                      <div className="flex items-center gap-2 text-[12px] font-medium text-amber-500">
                        <AlertTriangle size={14} /> 需要确认的事项（{imported.warnings.length}）
                      </div>
                      <ul className="mt-1.5 grid gap-1 pl-5 text-[11px] leading-5 text-amber-500/90">
                        {imported.warnings.map((warning) => (
                          <li key={warning} className="list-disc">{warning}</li>
                        ))}
                      </ul>
                    </div>
                  )}
                </section>
              )}

              {character ? (
                <section className="rounded-lg border border-line p-3">
                  <div className="flex items-center gap-2">
                    <h3 className="text-[13px] font-semibold">{character.name}</h3>
                    {character.nickname && <span className="rounded bg-subtle px-1.5 py-0.5 text-[11px] text-muted">昵称 {character.nickname}</span>}
                    {character.tags.slice(0, 4).map((tag) => (
                      <span key={tag} className="rounded bg-subtle px-1.5 py-0.5 text-[11px] text-muted">{tag}</span>
                    ))}
                  </div>
                  <dl className="mt-2 grid gap-1 text-[11px] text-muted">
                    {[
                      ['描述', character.description],
                      ['性格', character.personality],
                      ['场景', character.scenario],
                      ['开场', character.first_message],
                      ['台词样例', character.example_messages],
                    ].map(([label, value]) => value ? (
                      <div key={label} className="flex gap-2">
                        <dt className="w-14 shrink-0 text-fg/60">{label}</dt>
                        <dd className="line-clamp-2 text-fg/75">{value}</dd>
                      </div>
                    ) : null)}
                    {character.alternate_greetings.length > 0 && (
                      <div className="flex gap-2">
                        <dt className="w-14 shrink-0 text-fg/60">备用开场</dt>
                        <dd className="text-fg/75">{character.alternate_greetings.length} 条（开局选择，对应多条初始世界线）</dd>
                      </div>
                    )}
                  </dl>
                </section>
              ) : (
                <p className="rounded-lg border border-line bg-subtle px-3 py-2 text-[12px] text-muted">
                  这份文件是<b>世界书</b>，不含角色卡。导入后会作为设定条目进入世界库。
                </p>
              )}

              {imported.lore.length > 0 && (
                <section className="rounded-lg border border-line p-3 text-[12px]">
                  <h3 className="text-[13px] font-semibold">设定条目</h3>
                  <p className="mt-1 text-[11px] text-muted">
                    共 {imported.lore.length} 条 · 常驻 {constantCount} 条 · 停用 {disabledCount} 条
                    {imported.lore_meta.scan_depth != null && ` · 扫描深度 ${imported.lore_meta.scan_depth}`}
                    {imported.lore_meta.token_budget != null && ` · 预算 ${imported.lore_meta.token_budget}`}
                  </p>
                </section>
              )}
            </div>
          ) : (
            <ul className="grid gap-2">
              {imported.lore.map((entry, index) => (
                <li key={`${entry.source_uid}-${index}`} className="rounded-lg border border-line p-3">
                  <div className="flex items-center gap-2">
                    <FileText size={14} className="shrink-0 text-muted" />
                    <h3 className="min-w-0 flex-1 truncate text-[13px] font-medium">{entry.title}</h3>
                    <span className="rounded bg-subtle px-1.5 py-0.5 text-[11px] text-muted">{sectionLabel(entry.section)}</span>
                    {entry.constant && <span className="rounded bg-accent/15 px-1.5 py-0.5 text-[11px] text-accent">常驻</span>}
                    {!entry.enabled && <span className="rounded bg-subtle px-1.5 py-0.5 text-[11px] text-muted">停用</span>}
                    <span className="text-[11px] text-muted">#{entry.order}</span>
                  </div>
                  {entry.keys.length > 0 && (
                    <p className="mt-1.5 flex flex-wrap items-center gap-1 text-[11px] text-muted">
                      触发：{entry.keys.map((key) => <span key={key} className="rounded bg-subtle px-1.5 py-0.5">{key}</span>)}
                      {entry.secondary_keys.length > 0 && (
                        <>
                          <span className="ml-1 text-fg/50">{logicLabel(entry.logic)}</span>
                          {entry.secondary_keys.map((key) => <span key={key} className="rounded bg-subtle px-1.5 py-0.5">{key}</span>)}
                        </>
                      )}
                    </p>
                  )}
                  <p className="mt-1.5 line-clamp-3 whitespace-pre-wrap text-[12px] leading-5 text-fg/75">{entry.content}</p>
                  <p className="mt-1.5 flex flex-wrap gap-2 text-[10px] text-muted">
                    <span>{entry.source_dialect}</span>
                    {entry.decorators.map((decorator) => <span key={decorator} className="font-mono">{decorator}</span>)}
                    {entry.use_regex && <span>正则键</span>}
                    {entry.vectorized && <span>向量激活（v1 不支持）</span>}
                  </p>
                </li>
              ))}
              {!imported.lore.length && <li className="py-8 text-center text-[12px] text-muted">没有设定条目</li>}
            </ul>
          )}

          {imported.source_fields.length > 0 && (
            <details className="mt-4 rounded-lg border border-line px-3 py-2">
              <summary className="cursor-pointer text-[11px] text-muted">来源字段（{imported.source_fields.length}）—— 核对未映射内容</summary>
              <p className="mt-1.5 font-mono text-[10px] leading-5 text-muted">{imported.source_fields.join('、')}</p>
            </details>
          )}
        </div>

        <footer className="flex items-center justify-between gap-3 border-t border-line px-5 py-3">
          <p className="text-[11px] text-muted">确认后写入世界库；原始字段保留，语义抽取在后续阶段进行。</p>
          <div className="flex shrink-0 gap-2">
            <button onClick={onCancel} className="rounded-lg border border-line px-3 py-2 text-[12px] hover:bg-subtle">取消</button>
            <button onClick={onConfirm} className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-2 text-[12px] text-white hover:brightness-110">
              <Check size={14} /> 加入世界库
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
