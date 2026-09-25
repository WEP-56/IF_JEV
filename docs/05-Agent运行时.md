# 05 Agent 运行时

LLM 层采用正规的 agent 架构：模型通过工具读取世界、提交提议，runtime 由 Jev 和引擎主导。实现基于已有的 Rust agent **Onemore**（`ai-agent-example/`），只重新设计 tools，并把 runtime 改成由 Jev 主导。

## 1. 为什么用 agent

- **结构化输出靠工具参数。** 每个工具都有 JSON Schema，参数先校验再执行，不再从自由文本里解析 JSON。
- **能自我修正。** 提议被 Jev 否决时，模型在工具结果里看到原因，可以当场改写。
- **复用成熟的底层。** provider 适配、流式、重试、prompt cache、取消，Onemore 都已经实现。

## 2. 核心原则

1. **模型只能读视图、提交提议。** 任何工具都不直接写入事件日志。提议的效果先进入任务工作区，由引擎在回合末对账后提交。
2. **模型不能自己决定任务结束。** 模型停止调用工具时，host 先检查任务的完成条件；不满足就注入具体理由，让模型继续。
3. **Jev 审查每一条提议。** 提议类工具的调用由 host 收集，按视图分组，批量交给 Jev。
4. **每个任务都是一次新会话。** 不跨任务携带历史，世界信息只通过视图获得。这样视角隔离才有保证，提示词也短。
5. **否决理由同样受视角隔离约束。** 涉及秘密的失败，只给出通用的修改方向，理由里不能说出秘密本身。

## 3. 与 Onemore 的对应

Onemore 的核心 loop（`run_agent_loop`）只负责编排模型调用，所有有状态的行为都通过 `AgentLoopHost` 的回调接入。IF 实现一个 `IfTaskHost`，它就是"由 Jev 主导的 runtime"。

| 挂载点 | Onemore 默认行为 | IF 中的行为 |
|---|---|---|
| `prepare_prompt` | 组装 system 段落和对话记录 | 按任务编译视图：稳定前缀在前，任务内容在后（见 [08](08-视图与世界书.md)） |
| `execute_tool_turn` | schema 校验 → 权限 → hooks → 并发执行 | schema 校验 → 读类工具按视图权限直接执行 → 提议类工具收集后按视图分组、批量交给 Jev → 通过的记录效果，否决的返回理由 → 整批原子写入任务工作区 |
| `intercept_stop` | stop hook、planning 提醒 | 完成判定：检查任务的完成条件，不满足就返回让模型继续的理由 |
| `ToolTurnResult.stop_after_commit` | hook 要求停止 | Jev 判定任务已经完成（例如停止条件已达成）时，由 host 结束任务 |
| `poll_queues` / `take_steering` | 用户插话 | 用户点"停止"；引擎注入修复指令 |
| `record_usage` | 用量统计 | 计入回合指标 |
| `finish` | 取消后的清理 | 把任务记录归档到回合记录 |

### 3.1 复用边界

| 处理 | 模块 |
|---|---|
| 保留 | `agent_loop`：`run_agent_loop`、`AgentLoopHost`、`RetryPolicy`（只在尚未开始流式输出时重试）。`provider`：Anthropic Messages、OpenAI Responses、SSE、稳定前缀与 prompt cache key。`message`：`ChatMessage`、`Block`、`Usage`。`tools` 的基础类型：`Tool`、`ToolSpec`、`ToolRegistry`、`ToolOutput`、`ToolError`、schema 预检 |
| 替换 | `ToolContext`：workspace 和 plan 换成 IF 的任务上下文（视图读取句柄 + 效果记录）。`ToolEffect`：PlanUpdated 换成 IF 的提议效果。工具执行器：`DefaultToolExecutor` 是 crate 私有的，而且面向编码场景的权限，改为在 host 中自行实现。上下文组装：`ContextProvider` 换成视图编译。会话存储：任务生命周期很短，放在内存即可，审计信息写进回合记录 |
| 去掉 | 编码类工具（读写文件、编辑、命令、git、搜索、glob 等）、文件与命令权限、skills、planning 提醒、compaction（由视图编译代替）、TUI、JSONL RPC（改用 Tauri IPC）、进程 Job Object。MCP 保留为后续的扩展点 |
| 需要补 | ① **OpenAI Chat Completions 适配器**：中转服务、本地模型、Ollama 和大多数"OpenAI 兼容"接口只支持它，酒馆用户主要用的也是它。② 本地 JSON Schema 校验器补上 `items`、`minItems`、`maxItems`：IF 的工具参数大量使用数组，而现在的子集校验器不检查数组元素。③ 新增错误码 `JudgeRejected`，与 `HookRejected` 区分开。④ Gemini 原生适配器【后续】 |

## 4. 任务目录

| 任务 | 用途 | 视图 | 工具 | 放行检查 | 完成条件 | 轮数上限【初始值】 |
|---|---|---|---|---|---|---|
| T-rewrite | 导演指令改写 | 解析视图 | `propose_if_rewrite` | `q.input.is_directive` 为否；`q.input.rewrite_preserves` | 1–3 条通过 | 3 |
| T-parse | IF 解析 | 解析视图 | 读类、`submit_if_draft` | `q.if.parse_faithful`、`q.if.type` | 一份草案通过 | 4 |
| T-reconcile | 冲突重释 | 上帝视图（只含冲突相关部分） | `propose_reconciliation` | `q.if.reconcile_plausible` | 有方案通过，或明确放弃 | 3 |
| T-impact | 影响候选 | 上帝视图 + `read_pov` | 读类、`propose_candidate` | `q.cand.relevance`、`q.cand.in_character`、`q.cand.knowledge_gap` | 每个受强烈影响的主体至少一条；互斥组完整 | 4 |
| T-drive | 自驱候选（继续回合） | 同 T-impact | 同 T-impact | 同 T-impact | 焦点角色至少一条 | 4 |
| T-scenes | 场景候选 | 导演视图 | 读类、`propose_scene` | `q.scene.resolves_thread`（针对受保护的线） | 满 K 条，且覆盖至少 3 类走向 | 4 |
| T-plan | 场景计划 | 导演视图（揭示范围受控） | `submit_scene_plan` | 计划检查 | 一份计划通过 | 3 |
| T-render | 正文 | 叙事视图 | 无写类工具；正文走文本输出，用分隔标记切分节拍（见 §7） | 全套节拍检查 | 停止条件达成，或达到节拍上限 | 节拍上限 × 3 |
| T-extract | 正文回收 | 检查视图（已展示的节拍） | `lookup_prop`、`record_observation` | `q.extract.faithful`、`q.extract.kind`、`q.extract.load_bearing` | `q.extract.missing` 为否 | 4 |
| T-offscreen | 后台结算 | 上帝视图（后台主体） | `propose_offscreen_event` | 只做约束类检查；是否发生由引擎掷骰 | 每个活跃的后台主体都已考虑到 | 3 |
| T-observe | 观测措辞 | 引擎选定的信息 | `submit_observation_text` | `q.observe.leaks_secret` | 通过 | 2 |
| T-inspire | IF 灵感 | 玩家视图 | `propose_if` | `q.input.is_directive` 为否 | 3 条 | 2 |
| T-create | 引导创建世界 | 创建视图 + 与用户对话 | 创建类工具 | `q.world.consistent`、`q.world.goal_conflict` | 草案检查通过，且用户确认 | 与用户对话，不设上限 |

T-scenes 的走向类别：顺势、逆转、慢热、第三方介入、升级。

## 5. 工具目录

### 5.1 读类

只读，可以并行（`ParallelSafe`）。返回内容一律经过当前任务视图的过滤。单个工具结果不超过 24,000 字符（Onemore 现有的限制）。

| 工具 | 返回 |
|---|---|
| `lookup_subject(name_or_id)` | 主体摘要 |
| `lookup_prop(subject, keyword)` | 相关命题及其当前值 |
| `read_pov(subject)` | 该角色的视图摘要（仅 T-impact、T-drive 可用） |
| `read_thread(id)`、`read_tendencies()` | 故事线、趋势（仅导演类任务可用） |
| `read_recent(n)` | 最近的节拍或场景摘要（限于玩家可见的范围） |
| `search_lore(query)` | 设定条目 |

### 5.2 提议类

只记录效果，由 host 批量审查。候选 ID 由 host 分配，写在工具结果里，例如"已登记为 cand_004"。

| 工具 | 主要参数 |
|---|---|
| `propose_if_rewrite` | 改写文本、类型提示 |
| `submit_if_draft` | 类型、核心命题、时间锚点、作用范围、锁定建议、规则边界、不承诺项、消化策略建议 |
| `propose_reconciliation` | 解释文本、需要补充的事实 |
| `propose_candidate` | 主体、内容、内在或外在、发生类或互斥组（含选项）、`depends_on`、所依据的认知、影响的命题 |
| `propose_scene` | 概要、视角人物、焦点、**在场主体**、实现哪些预演变化、推进哪些故事线、走向类别、揭示的内容 |
| `submit_scene_plan` | Scene Plan（见 [02 §10](02-世界模型.md)） |
| `record_observation` | 类别、内容、涉及的主体与命题、所在节拍、原文引用 |
| `propose_offscreen_event` | 涉及的主体、内容、世界时间范围 |
| `submit_observation_text` | 观测结果的措辞 |
| `propose_if` | IF 文本 |
| 创建类 | `upsert_subject`、`add_fact`、`add_rule`、`add_thread`、`add_lore`、`submit_world_draft` |

## 6. 否决与自我修正

- 被否决的调用返回 `JudgeRejected`。说明文字根据未通过的问题 ID 生成，例如："未通过：这个反应需要用到顾言并不知道的信息。请只根据顾言已知的信息改写。"
- 涉及秘密的否决不说出秘密，例如："第 3 节拍包含本场景不允许揭示的内容，请删去对该人物来历的任何暗示。"
- 同一个任务中同类失败累计达到上限【初始值 2】时，host 结束任务，交给回合流程兜底（见 [04 §4](04-回合流程.md)）。

## 7. 正文的流式与放行

**已确认：正文走普通文本输出，不用工具参数。** 原方案（`write_beat` 工具）已废弃，理由是长段文学文本塞进 JSON 参数里质量不可控。T-render 在一次模型调用里连续输出正文，用分隔标记切分节拍。

- 分隔标记：模型每写完一个节拍输出一行标记（例如 `<<<BEAT>>>`），host 在流式回调里边收边切。
- 缓冲与放行：每个节拍完整收到后**先不展示**，送检查视图跑全套节拍检查（[04 §4](04-回合流程.md)）；通过才放行展示，界面可以做逐字动画。未通过则丢弃该节拍，把有针对性的修改说明追加进对话，让模型重写这一段。
- 标记必须由引擎校验：节拍数、标记是否成对、有没有正文混在标记之外。模型漏写或写错标记时，按"这个节拍不合规"处理，退回重写。
- 检查是逐节拍串行的，但模型可以继续往下写；host 通过背压控制（未裁决的节拍超过上限【初始值 2】时暂停读取）避免一次性生成过多未验证内容。
- 等待期间，界面显示真实的阶段进度：解析 → 裁定 → 候选 → 掷骰 → 选场景 → 写作。
- 取消：用户点"停止"时中止模型调用，已展示的节拍保留，未裁决的缓冲丢弃，场景在最后一个已展示的节拍处结束。
- 引擎不再需要 `write_beat` 工具；T-render 的工具集只保留读类和 `submit_scene_plan` 之外的必要项。

## 8. 线程模型

- Onemore 是同步实现（ureq 阻塞 HTTP + 标准线程，没有异步运行时），可以直接嵌入 Tauri 的 Rust 端。
- 每个打开的世界一个工作线程，命令与事件走有界通道；事件经 Tauri 的 emit 推送给前端；取消用 `AtomicBool`。
- Jev 请求同样用阻塞 HTTP；按视图分组的多个请求用 scoped threads 并行发出。
