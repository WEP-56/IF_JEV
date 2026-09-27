# IF 项目交接说明

> 面向下一位接手者的快速恢复文档。设计细节以 `docs/00–15` 为准；本文件只记录当前工程状态、已验证入口和下一阶段顺序。
> 最后核对：**2026-09-27**（作者世界：`if-app::authored` —— 一份手写的世界包直接落成 `.ifworld`，
> 补上导入路径产不出的**命题**；示例世界「临江市」与测试夹具「渡口」各一份，见 §5）。
> 此前一次是同日把 `if-pipeline` 接上司机（`tasks` 层五个 agent 任务 + `if-app::turn_runner`）。
> 此时 `main` = **`c315e42`**，工作区干净，**无 CI**（仓库没有 `.github/`）。

## 0. 冷启动速览（先读这一节）

**现状一句话**：世界库（导入 / 手写 / 持久化）、会话（创建 / 重启恢复 / 删除）、播种、
IF 注入与裁定卡、**一个可提交的 IF 回合**（`if-pipeline`：
T-impact → 分层裁决 → 场景候选 → 导演选择 → 场景计划 → 逐节拍检查 → 对账 → 事件草稿），
以及**把这个回合接上司机的那一层**（`if-pipeline::tasks` 五个 agent 任务 + `if-app::turn_runner`
串成一条回合并接进 `world_worker`）现在都在仓库里且单测全绿。
界面上「确认裁定卡」之后**真的会推进一个回合**：推演出的正文按节拍逐段上屏。
**这一轮的合同是用 scripted provider 跑通的**（`if-pipeline` 的输入是提议，provider 真假不影响
回合合同被验证）；真实 LLM / Jev 的正文质量与连通性仍待真机验收（见下表的验收项）。

**下一步只有一件事**：**T-parse / 语义抽取**——把设定条目里「承重的那一半」抽成命题 / 事实 /
规则草案，接 `q.extract.faithful` 校验（见 §5 第 1 项）。这也是 `if-lore` 与 `importer/`
迁位的触发条件（世界书激活现在住在 `if-pipeline::lore`）。

> ⚠️ 2026-09-27 真机之后，它的性质变了：**不是「下一件该做的事」，而是「回合能不能跑的前提」**。
> 世界没有命题时，T-impact 只能提出**无键候选**（决策键回退到候选 ID、跨世界线不可复现），
> 而且最初连这一步都跑不过去（模型在空世界里无键可填、干脆不调工具——见 §3 那条真机缺陷）。

**当前基线（动手前先跑一遍）**：

```text
cargo test --workspace                    475 passed / 0 failed（if-pipeline 132、if-app 118）
cargo clippy --workspace --all-targets    零 lint（只剩 E: 盘硬链接环境提示）
cd app && npx tsc --noEmit                干净
cd app && npm run build                   通过（1916 模块 / 345.47 kB）
```

**真机验收到了哪一步**（用户执行，别当成自己验过了）：

| 能力 | 状态 |
|---|---|
| 导入世界 → 关掉再打开，资产还在；重导不翻倍 | ✅ 已真机验收 |
| 选世界建会话 → 看到播种简报与投影出来的世界 | ✅ 已真机验收 |
| 建会话 → **重启 → 会话还在**（侧栏列表） | ✅ 已真机验收（`4c182c5` 修的） |
| 删会话 → 重启不回来、世界文件一并清掉 | ✅ 已真机验收（2026-09-26） |
| 输入 `IF xxxx` → 落裁定卡 → 确认写进事件日志 | ✅ 落卡已复验通过（2026-09-26，`725fff4` 修掉 schema 缺陷后）；确认那一步待验 |
| 真实 Jev / LLM 的连通性与 smoke 判定（`test_llm` / `test_judge`） | ✅ 已真机验过，见 [14](14-Jev实测.md) |
| 三种剩余的真实方言（V3 `assets` / `@@` 装饰器 / 独立 `lorebook_v3`） | ⬜ 用户已明确推迟（见 §4） |
| 一个回合真的推进一步（正文上屏、世界改变） | ⬜ **首次真机失败在 T-impact**（2026-09-27，契约问题已修，见 §3）→ **待再验**：确认裁定卡后应看到场景卡 + 正文按节拍逐段上屏。再验时**建议先用「临江市」示例世界**——它有 8 条命题，T-impact 有键可填（首次失败的那张卡一条命题都没有） |

**三件容易被误当成 bug 的事**（都是刻意的，别去「修」）：

1. **聊天记录不持久化**——`.ifworld` 里是**事件**，前端 `messages` 只是缓存。
   重开一条会话看到的是「开场白 + 一条现状摘要」，不是上次的对话原文。
2. **侧栏里混着演示故事**（`initialStories`）——它们背后没有世界文件，是前端早期示例数据。
   真实会话排在它们前面，且只有真实会话能发 IF。把两者分开是下一刀。
3. **示例世界「临江市」与那条演示故事「雨城纪事」同源同名**，但不是一回事：前者是
   `examples/worlds/linjiang.world.json`，能被建会话；后者只是一段前端常量（第 2 条）。
   侧栏分组做完之前，两者会同时出现在用户眼前——**别把它们合并**，那会丢掉「有世界文件 /
   没有世界文件」这条区别。

## 1. 项目定位

IF 是一个面向反事实互动叙事的垂直领域 AI agent harness。**Tauri 桌面叙事应用是首个宿主**，用来验证 harness 的 agent 运行时、工具、Jev 判断和确定性世界提交。

**方向提醒（2026-09-26 明确）**：IF 以**酒馆的世界书 / 角色卡机制为世界材料基础**，目标是一个较为被动、客观推进的**世界沙盒 agent**——用户导入世界书与角色卡，Jev 与 LLM 协作推进，**用户侧只发 `IF xxxx` 反事实事件**来引导世界发展。它不是酒馆的复刻，而是「酒馆的材料 + 反事实的交互」。因此工程顺序是：**酒馆适配 → harness / agent 开发 → 从实测中逐步改善**。

用户通过三种动作与宿主世界交互：

- **IF**：注入一条权威反事实断言；
- **观测**：获取受视图权限限制的信息，不改变世界；
- **继续**：让世界自行推进一个场景。

v1 只支持沙盒模式，同一时间只打开一个世界。事件日志是唯一真相，世界状态由事件折叠成投影；前端状态只能作为缓存。核心能力的验收重点是 harness 合同和可重放回合，沙盒 UI 是其首个消费方。

## 2. 代码结构

```text
app/                    React + TypeScript + Vite 前端
crates/if-app/          Tauri 命令、事件、设置、密钥、世界工作线程、IF 预解析、酒馆导入、世界库映射、会话创建
  src/importer/         酒馆角色卡 / 世界书导入（card / lorebook / png / model / value）
  src/library.rs        导入结果 ↔ 世界库的映射（来源身份键、内容哈希、摘要）
  src/seed.rs           ImportedWorld → if-domain 的确定性播种（主体 / 设定条目；见 10 §3.0）
  src/authored.rs       作者世界包 → if-domain 的确定性映射（主体 / 命题 / 事实 / 规则 / 设定 / 故事线）
  examples/worlds/      作者世界包（`*.world.json`）与格式说明：示例「临江市」
  src/session.rs        会话的创建 / 列举 / 恢复 / 删除（选世界资产 → 写 .ifworld → 登记引用）
  src/slug.rs           名字 → 文件名 / ID 片段的共用规则（保留中文）
crates/if-domain/       领域类型、事件补丁、投影、世界线、回合记录、裁定卡
crates/if-pipeline/     回合编排（见 §3）：context / candidates / scenes / beats / commit / turn / audit
crates/if-views/        视图编译、可见性判定、预算裁剪、稳定指纹
crates/if-policy/       阈值表、命运骰子、分层裁决、观察带与趋势、导演评分
crates/if-store/        SQLite 事件日志、投影、快照、世界线
  src/library.rs        世界库：世界资产 / 来源 / 设定条目 / 会话引用（独立 library.db）
crates/if-agent/        Agent loop、provider、工具 schema
crates/if-judge/        Jev、LLM 裁判、测试桩与重试
docs/                   正式设计与工程文档
```

crate 划分见 [12 §3](12-工程架构.md)。`if-lore` **尚未创建**（酒馆导入暂放 `if-app/importer/`，
迁移时机见 [12 §3](12-工程架构.md)）；已落地 crate 之间的依赖是单向的
`if-domain → if-views / if-policy → if-pipeline → if-app`。

关键原则：

1. 每个 `.ifworld` 文件由一个 `WorldWorker` 专属线程持有 SQLite 连接。
2. 事件只追加，不直接修改投影；分支和回滚只移动世界线头指针。
3. API 密钥走系统钥匙串，不进入 localStorage 或世界文件。
4. 真实 LLM/Jev 和叙事质量由用户验收；Agent 负责静态、模拟和可自动化测试。
5. 单个源文件 ≤ 1000 行；接近上限按职责拆模块（`AGENTS.md`）。
6. **`if-pipeline` 不认识网络、也不认识 SQLite**：需要模型只向 `if_judge::Judge` 发问，
   需要文本只接收**提议**，产出是 `if_domain::event::EventDraft`。所以整个回合能在没有网络、
   没有数据库的情况下端到端测试——这正是 [12 §9](12-工程架构.md) 要的东西。

## 3. 当前已打通

### Rust / Tauri

会话生命周期（**已能把一个世界资产玩起来，且已真机验收**，[10 §3](10-世界创建与导入.md)）：

- `create_session(world_id, label?)`：选一个已保存的世界资产 → 在 `worlds/` 下开一个新的 `.ifworld`
  → **播种**（见下）→ 在 `library.db` 写一条 `world_sessions` 引用。返回 `SessionView`。
  `SessionView.session_id` 就是那条引用的 ID，前端靠它重开 / 删除。
- `list_sessions(world_id)`：某个世界已有的会话（新建前用它问「要用哪个会话」）。
- `list_all_sessions()`：**全部**会话，最近的在前。**启动时侧栏靠它恢复**——
  不读库的话，重启之后侧栏只剩演示故事，而会话其实好好地躺在 `world_sessions` 里。
- `open_session(session_id)`：从 `world_sessions` 找回 `.ifworld` 重新打开，**不重新播种**。
- `delete_session(session_id)`：移出世界库 + 删掉世界文件（含 `-wal` / `-shm`）。
  删的若是当前打开的世界，**先关掉它**（Windows 上打开着的文件删不掉）。
  ✅ **2026-09-26 真机验收可用**。
- `open_world(path)` / `close_world` / `get_world_snapshot`；`world://opened` / `world://closed` 事件。
- **没有 `create_world` 命令**——[10 §3](10-世界创建与导入.md) 要求新建会话必须先选世界；
  「空世界」走世界库的「手动撰写」（那也是一个资产，只是 payload 里没有角色与设定）。
  ⚠️ 别和 `if-store` 的 **`Store::create_world` 方法**搞混：那个还在（它往事件日志里写
  `world_created`，是所有世界的起点），被删的只是同名的 Tauri 命令。

**播种**（`if-app::seed`，[10 §3.0](10-世界创建与导入.md)）是「导入 → `if-domain` 确定性映射」的落点：

- 固定顺序 **主体 → `scenario` 常驻世界段条目 → 卡内设定条目**；
- 角色 → `Subject`，`scenario` → 常驻 `LoreEntry`，世界书条目 → `LoreEntry`（1:1 拷贝，停用的不写入）；
- `first_message` **只进播种报告、不写事件**，`system_prompt` / `post_history_instructions`
  **不自动转 `WorldRule`**——两者都会成为「没经裁定卡的既成事实」（P13）；
- 拿不准的事项全部写进 `SeedReport.notes`，由前端渲染成「需要你拿主意的地方」。
- ⚠️ **事件 ID 是预分配的**：`Subject::created_by` / `LoreEntry::source` 存「引入它的事件 ID」，
  播种是一次成批写入，所以 `if-store` 暴露 `Store::next_seq()`，`append_batch` 从它顺序发号。
  这条契约由两侧钉住：`if-store::tests::event_ids_follow_next_seq`（发号规则）与
  `if-app::seed::tests::plan_matches_store_allocation`（推出来的号 == 实际写出来的号）。
  改 `append_batch` 的分配方式时这两个测试会红——那是设计如此。
  同一条契约在 `if-pipeline::commit::DraftCursor` 上是第三种用法（回合的草稿也先算号再写，
  因为 `Fact::source` / `Tendency::contributors` 存的也是引入它的事件 ID）。

IF 流程（**裁定卡生命周期 + UI 都已实现**）：

- `submit_if` / `submit_if_model`：在世界工作线程中串行追加输入，先落一张 **`pending` 裁定卡**。
- `confirm_if` / `reinterpret_if` / `cancel_if`：确认才产生 `if_injected` 事件并更新投影。
- 冲突预检：本地确定性检查能标出与已有事实相反的断言（`IfConflict` + `IfConflictResolution::Reinterpret`）。
- `if-domain::turn` 内有 `IfRulingCard` + `IfCardStatus{Pending,Confirmed,Cancelled}` + `IfConflict`，`world_worker.rs` 有对应单测。
- **前端 UI 已接**（`app/src/components/ChatView.tsx`）：`pending_if` 一出现就渲染卡片
  （类型 / 时间锚点 / 锁定建议 / 核心命题 / 范围 / 警告 / 冲突），措辞可编辑，
  `取消` / `确认并锁定` / `按重释确认` 三个动作都已接线。
  → ✅ **2026-09-26 复验通过**：`IF 所有人从此无法说谎` → 落出 `pending` 裁定卡，
  核心命题与两条「边界未明确」的警告都合理。此前报「结构模型返回的 IF 草案无效：
  missing field `input`」，原因与修法见下面那条缺陷记录。
  前端用的是 `submit_if_model`；确定性的 `submit_if`（不调模型）只有命令、没有 UI 入口。
  ⚠️ 同一次真机还暴露了**两个字段上的偏差**（`suggested_lock` / `scope`）——已在下一节一并修掉。

**修掉的真机缺陷：工具 schema 与 `IfDraft` 不对齐**（2026-09-26，`diagnostics.rs`）

`IfDraft::input`（**用户的原话**）是必填字段，`world_worker` 拿它填
`record.input` 与 `IfInjection.input`——裁定卡上显示的就是这一句。但 `submit_if_draft`
的 schema 既没声明 `input`、也没把它列进 `required`，模型**照 schema 如实返回**，
`serde_json::from_value::<IfDraft>` 于是当场失败，用户拿到一句
「missing field `input`」——只有开发者看得懂，也不知道能做什么。

修法**不是**把 `input` 加进 schema 让模型复述（那会让裁定卡显示模型改写过的句子），
而是**由引擎回填**：`diagnostics::draft_from_tool_args` 在反序列化前塞进用户原话，
模型多嘴回了 `input` 也一律覆盖；报错同时列出「模型实际返回了哪些字段」，
并提示可以重试或换更严格的结构模型。

护栏有两条（`diagnostics::tests`，共 8 项）：

1. **结构对齐**——`IfDraft` 的每个字段要么在工具 schema 的 `properties` 里且 `required`，
   要么在豁免表里（`input` = 引擎回填，`rewrite_candidates` = 带 serde default、只有确定性
   解析器会产）。**以后给 `IfDraft` 加字段而忘了同步 schema，它会变红**。
2. **整条路径**——为此把 `parse_if_with_model` 拆成「读配置 + `build_provider`」与
   `parse_if_with(provider, …)` 两半，于是测试能注入
   `if_agent::provider::scripted::ScriptedProvider`，**不联网、不花额度地跑真机同一条
   代码路径**（发问 → `tool_uses()` → 补 `input` → 反序列化）。顺带钉住：模型只说话不调
   工具时报「未调用 `submit_if_draft`」、上游报错**原样透出**而不是伪装成「草案无效」、
   已取消的任务不再解析结果。

> 教训（与 `if-pipeline` 那 6 个缺陷同类）：**工具 schema 是一份契约，但它不被任何
> 运行时代码校验**——`parse_if_with` 绕开 agent loop 直接读 `tool_uses()`，
> 没人拿 schema 去验模型的返回。契约与 Rust 结构体只能靠测试对齐。
> 另一条：**只测纯函数不够**。`draft_from_tool_args` 从第一天起就是对的，炸的是
> 「调用方怎么拿到 args」——所以补测试时补的是整条路径，不是再给纯函数加断言。

**同一张卡暴露的第二组缺陷（已修：`suggested_lock` / `scope` / 卡片枚举）**

真机上 `IF 所有人从此无法说谎` 显示的卡片是 `rule · now · 锁定 L3`、`范围 individual`。
两处都与 docs/01 不符，另有第三处是展示问题：

| 字段 | 模型给的 | docs/01 规定 | 依据 |
|---|---|---|---|
| `suggested_lock` | `L3` | **`L2`（规则型）** | §6 锁定等级表——「kind → lock」是**全函数**，文档举的例子就是这句话 |
| `scope` | `individual` | 全局（`global`） | §5 作用范围；「所有人」是确定性线索 |
| 卡片渲染 | `rule` / `now` / `individual` | 中文 | 裸英文枚举直接露给用户 |

三处一并修掉，**方向都是「把引擎有资格决定的东西从模型手里拿回来」**：

1. **`suggested_lock` 改由 `kind` 推导**：新增 `ParsedIfKind::default_lock()`
   （docs/01 §6 的表一字不差地变成代码，确定性解析器与模型路径**共用同一个来源**），
   字段从工具 schema 里删掉——模型连填的地方都没有。
   `L2` 与 `L3` 在冲突让位时规则不同，这不是「两种做法都行」。
2. **`scope` 补齐四档**：`injection_from_draft` 原来只认 `"global"`、其余全塌成
   `Individual`，`Group` / `Region` **永远拿不到**；现在四档都在，认不出的字符串
   按最窄的个体处理（宁可少认影响范围，也不要把局部 IF 当成世界规则去冲突）。
   确定性解析器也不再产自造的 `individual_or_local`，只产 `IfScope` 认得的字符串。
3. **卡片枚举中文化**（`ChatView.tsx`）：类型 / 时间锚点 / 范围 / 状态都加了标签；
   **认不出的值原样显示**——后端加了新枚举而前端忘了同步时，那是唯一的提示。

更根本的一处：**新增 `ModelDraft`，把「模型该填什么」变成一个 Rust 结构**。
`input` / `suggested_lock` / `rewrite_candidates` 三样归引擎，于是那条不变式从
「逐字段对齐 + 豁免表」升级为**集合相等**——`schema.properties ≡ ModelDraft 字段`，
没有豁免、没有解释空间。另外给每个字段补了 `description`、给 `kind` / `time_anchor` /
`scope` 补了 `enum` 说明：**第一版一个说明都没有，模型只能猜**，这才有上面那两处错。

> 教训（与上面同类）：**模型答错的字段，先问「这事该不该问模型」**。
> 这两处都不是模型不听话，是引擎把确定性的事外包了出去。

**真机缺陷：T-impact 在「还没有命题」的世界里无键可填，模型干脆不调工具**（2026-09-27）

把驱动接上后，第一次真机完整回合失败在第一步：

```text
推演失败：回合失败（T-impact）：任务 T-impact 没有产出任何提议：模型在 4 轮里都没有调用 propose_candidate
```

**根因不是模型**（同一个结构模型刚刚顺利落出了裁定卡），是**契约在空世界里不可满足**：

- 播种只建**主体 + 设定条目**；命题 / 事实 / 规则是 T-parse 的产物，而 T-parse 还没做（见 §4）。
  于是新建世界的 `Projection` 里 `propositions` / `facts` 全空。
  实测聂小倩世界：17 条事件 = `world_created` 1 + `subject_created` 1 + `lore_added` 13 + `if_injected` 2，
  展开成投影是**主体 1 / 命题 0 / 事实 0 / 规则 0 / 故事线 0 / 设定条目 13**。
- 上帝视图（`tasks::impact::prompt`）对空字段有 `skip_serializing_if`，所以模型看到的 JSON
  里**根本没有 `facts` 这个键**（诊断输出见下）；而提示词却写「依据与影响都写视图里的命题键」，
  schema 更把 `affects` 描述成「**至少写一个**」。
- 模型拿到的是「必须引用一个不存在的键，又不许编造」——它选择了不调用工具，4 轮提醒都没用。

顺带两处错：

1. 工具说明写的是 `propositions[].key`，**视图里根本没有这个字段**——命题在视图里以
   `facts[].key` 出现。模型照着一个不存在的路径找，只会白找。
2. 失败信息把「**一次都没调用**」与「**调用了但参数都没过**」说成同一句
   （都是「都没有调用」）。这两类的修法完全相反：前者是契约 / 提示词，后者是 schema / 模型笔法。

修法（`tasks/impact.rs` + `tasks/host.rs` + `turn_runner.rs`）：

- **契约容得下空命题**：`affects` / `based_on` 明确允许空数组，字段引用改回 `facts[].key`。
  这**不是**让模型随便填——是承认「世界还没有命题」是一个**合法状态**：引擎会退回用候选 ID
  兜底决策键，并在 `if-policy` 里记一条 `unstable` 警告（本回合能跑通，跨世界线不可复现）。
- **提示词把输出方式单列一条**：只能用工具作答、不要用自然语言解释。
- **失败信息说实话**：`ProposalHost` 记下模型最后一次说的话、被拒的工具调用次数与最后一次
  参数错误，`run_proposal_task` 据此给出「一次都没调用 / 调用了 N 次但参数没过」＋模型原话
  （与 `IfDraft::input` 那次是同一条教训）。护栏：
  `tasks::tests::a_task_that_never_calls_the_tool_says_so_and_quotes_the_model`、
  `a_task_whose_calls_all_fail_validation_says_it_called_and_why`。
- **过程提示不再被吞**：`turn_runner` 原先把所有 `AgentEvent` 丢进空闭包，
  于是「输出撞到 max_tokens 上限」「上游重试」这些线索全丢了；现在收进回合警告、
  失败时随错误信息一起带出。

诊断入口（这次新增）：`cargo run -p if-app --example dump_turn_prompt -- <世界文件> [IF 文本]` ——
不联网、不落盘，直接打印「模型到底收到了什么」。它把上面的结论从推测变成事实：
聂小倩世界的上帝视图 8414 字符、11 条设定 + 1 个主体、**零命题**。

> 教训：**说「必须先有 X 才能做 Y」的时候，先确认 X 在这个世界里存在**。
> 驱动接通 ≠ 能玩——世界模型里没有命题，回合的第一步就是空中楼阁。
> 这把 T-parse 从「下一件该做的事」变成了「回合能不能跑的前提」（见 §5 第 1 项）。
>
> 另一条：**两个不同的失败不要共用一句报错**。这次的报错文案本身就在误导排查方向。

解析与导入（确定性、无网络）：

- `parse_if` / `parse_if_model`：IF 预解析，输出导演指令标记、类型初判、时间锚点、作用范围、锁定建议、不承诺项与警告。
- `import_world_json` / `import_world_file`：酒馆导入。自动识别 PNG（`chara` / `ccv3` 文本块，优先 `ccv3`）与 UTF-8 JSON；同时兼容 CCv3 规范与酒馆运行时两套字段方言；含装饰器剥离、`position` → 归段映射、宏检测（`{{user}}` 等）。条目字段按「顶层 → `extensions`」分层读取（真实卡把引擎状态放在 `extensions`，见 [13 §6.4](13-酒馆兼容.md)）。
- `examples/inspect_card.rs`：对真实卡文件做导入体检（`cargo run -p if-app --example inspect_card -- <文件...>`），确定性、不联网、不落盘。
- `examples/dump_turn_prompt.rs`：把某个真实 `.ifworld` 的**上帝视图** dump 出来
  （`cargo run -p if-app --example dump_turn_prompt -- <文件> [IF 文本]`），确定性、不联网、不落盘。
  任务报「模型没有产出提议」时**先看它**：视图里有几个命题 / 事实 / 主体，一眼就能判断
  是「模型没干活」还是「契约在空世界里不可满足」。
- `tests/library_roundtrip.rs`：真实 PNG 卡 → 解析 → 映射 → 落库 → 读回 → 重导幂等。
  样本在 `sk-example/`（不入库），**文件不在就跳过**，不会在别人的 clone 上跑出假红。
  这是 `inspect_card` 覆盖不到的那一段——它不落盘。
- 诊断：`test_llm` / `probe_jev` / `test_judge`（`probe_jev` **只有命令、没有 UI 入口**）。

世界库（**世界资产已持久化**，[10 §2](10-世界创建与导入.md)）：

- 独立 `library.db`（与 `settings.json` 同目录），与会话的 `.ifworld` 是两个库、两个对象。
- `list_worlds` / `get_world_asset` / `delete_world_asset`：列表给摘要（含条目数、来源数、会话数），详情给 payload + 全部条目。
- `import_world_to_library` / `import_world_file_to_library` / `create_written_world`：写入类命令统一返回 `AssetChange { detail, assets }`，省一趟往返。
- **按来源整组替换**：来源身份键是「来源类别 + 名字」（不含卡版本 / 文件名 / 内容哈希），同一来源重导先删该来源的旧条目再写入新的。内容哈希另存一列回答「是不是同一份」。
- `delete_world_asset` 在被 `world_sessions` 引用时**拒绝**，并说明有几个会话在用。

### 回合编排（`if-pipeline`，[04](04-回合流程.md)）

**一个可提交的 IF 回合已能端到端跑通（`cargo test -p if-pipeline`，132 项单测）**，
而且**接上了司机**：`tasks` 层把提议从模型拿回来，`if-app::turn_runner` 把它串进
`world_worker`，界面上确认裁定卡之后能推进一个回合。

模块与各自在回合里的位置：

| 模块 | 对应步骤 | 做什么 |
|---|---|---|
| `context` | 全程 | 一次回合共用的输入（场景序号 / 世界时间 / 焦点与在场 / 严格度 / 阈值表）+ 按它造视图与策略 + **世界书激活**（`activate_for_turn` 是 [08 §5](08-视图与世界书.md) 的第一个真实调用方） |
| `audit` | 全程 | 回合内唯一的判定序号游标。**三段判定共用**——各自从 1 发号会让 `Beat::judgments` 的 ID 在 `TurnRecord` 里指错记录 |
| `candidates` | 6–7 | 约束门（在人物 / 知识缺口，**只比阈值、不掷骰**）→ 发生类与互斥类判定 → 按 `depends_on` 分层裁决。约束门先跑，被否决的候选不再花一次发生类判定 |
| `scenes` | 8–9 | 受保护故事线的**硬否决**（在请求发出之前拿掉，不问判定）→ 导演评分（Jev 四项 + 引擎四项）→ 按 `scene_select@<叙述序号>` 的骰子抽取 |
| `beats` | 11 | 逐节拍检查：事实（按锁定降序截断 **12** 条）/ 规则（**8** 条）/ 每个在场角色的认知边界 / 本场景禁止项 / 未批准揭示的秘密 / 停止条件 / 节拍目标。重试上限 **2**；非必需节拍跳过，必需节拍止损 |
| `commit` | 12–13 | 对账（[02 §11](02-世界模型.md) 四条规则 + 「同命题只提交一次，保留**锁定最强**的那条」）→ 固定顺序的事件草稿（**命题先于引用它的事实**） |
| `turn` | 6–13 | `open()`（6–9）与 `resolve()`（10–13）两段驱动，中间夹 T-plan；`inject_constraints` 把受保护线的禁止项写进计划；`record()` 汇成 `TurnRecord` |
| `tasks` | 5 / 8–9 / 10 / 11 / 12 | **司机层，也是唯一认识 LLM 的一层**：T-impact 提候选（上帝视图）、T-scenes 提场景（导演视图）、T-plan 写计划（导演视图）、T-render 出正文并切节拍（叙事视图）、T-extract 回收正文（检查视图）。有确定性依据的字段一律不问模型（候选 ID 引擎发号、`forbidden_resolutions` 由 `inject_constraints` 注入、决策键从命题键推导）；每个工具 schema 与「模型要填的结构体」**集合相等**，各有测试盯着。读类工具（`lookup_subject` 等）v1 不提供——视图直接进提示词 |
| `driver` | 5–13 | `run_if_turn`：把五个任务与两段驱动串起来（T-impact → `open_with` → T-plan → T-render → `resolve_with`）。`TurnProviders` 把模型分成**结构槽位与叙事槽位**（[11 §6](11-模型配置.md)），T-render 走叙事模型、其余走结构模型 |

几条必须记住的性质：

- **约束类永不掷骰**（[06 §1](06-裁决策略.md)）：合规检查与「是否符合人设」只比阈值，
  骰子只出现在发生类与互斥类上。缺判定按「不通过 / 不发生」处理——宁可让模型重写一次，
  也不要让违规正文上屏。
- **否决理由不能说出秘密**（[05 §2 · §6](05-Agent运行时.md)）：检查视图里可以点名秘密，
  但回给模型的理由必须是「涉及尚未批准揭示的内容」。`BeatBlock` 因此分两个字段：
  `reason` 是能说给模型听的那一份，`template` / `probability` 只进判定记录。
- **同一输入同一结果**（[12 §7](12-工程架构.md)）：所有集合用 `BTreeMap` / 有序 `Vec`，
  骰子由世界种子驱动，不读时钟。节拍上屏的现实时刻是唯一例外，由调用方显式传入、且不进投影。
- **两段驱动的顺序不能拧反**：场景要先选出来，才谈得上给它写计划。`turn` 因此是
  `open()`（6–9）与 `resolve()`（10–13）两段，中间**留一道缝**给 T-plan——
  把两段并成一个「先选场景再交计划」的大函数，只会逼调用方在选场景之前就把计划交上来。
  `driver::run_if_turn` 是**跨过这道缝的编排者**（按 5 → 8–9 → 10 → 11 → 12 的顺序
  调两段与夹在中间的任务），缝本身还在 `turn` 里。

**本轮顺手修掉的 6 个缺陷（都由新测试逮住，别再引入）**：

1. **节拍检查的问题键会重复**：一个节拍要查多条事实 / 规则 / 禁止项 / 秘密，而键写成
   `beat_{n}.fact`，第二条就覆盖第一条，`JudgeRequest::validate` 直接判重并拒绝**整个请求**
   ——后果是「世界书条目一多，节拍检查就整个跑不起来」。现在键带被检查对象的标识
   （事实用命题 ID，规则用规则 ID，禁止项用位序）。钉在
   `question::tests::repeated_checks_on_one_beat_never_collide`。
2. **`reason_for` 的模板比对永远不相等**：它拿「去掉 `@版本` 的名字」去比
   `Q_BEAT_VIOLATES_FACT` 这类**本身带 `@1`** 的常量，于是每条分支都匹配不上，
   所有否决理由静默退化成「未通过检查」——模型只知道没过、不知道改哪儿。
   现在两侧都削版本，并有一张 `BLOCK_REASONS` 表和
   `beats::tests::every_blocked_template_has_a_reason_for_the_model` 盯着它。
3. **判定记录 ID 跨段重复**：影响裁决、场景选择、逐节拍检查各自从 `jdg_0001` 起编号，
   于是 `jdg_0001` 在 `TurnRecord` 里同时命中两条不同记录，审计会查错。
   现在由 `audit::Audit` 统一发号，`Opening.audit` 把游标带到第二段。
   钉在 `turn::tests::judgment_ids_are_unique_across_the_whole_turn`。
4. **事件草稿没带场景 / 节拍**：`EventDraft` 的 `scene` / `beat` 两个字段是**会落库并读回**的
   （`events.scene` / `events.beat`），不填就是永远 NULL——事后按场景查事件是一片空。
   现在 `DraftCursor` 在推草稿时盖上。钉在
   `turn::tests::a_turn_stamps_its_scene_and_beat_on_every_event_it_writes`。
5. **对账去重按插入顺序**：同一条变化常常既是 Observed（从正文抽出来的）又是
   「正文里确实发生了的预演」，按插入顺序去重会留下先插进去的 Observed（`Lock::L0`），
   **把 IF 带来的 L2 静默降成自由状态**。现在保留锁定最强的那一条。
   钉在 `commit::tests::the_strongest_lock_survives_when_the_text_confirms_a_change`。
6. **导演评分用错了「当前场景序号」**：`overdue_bonus` 拿 `projection.scenes.len()` 当当前序号，
   而 `Thread::last_advanced` 记的是**场景序号**（[02 §10](02-世界模型.md)）——
   被否决而跳过的场景不进投影，逾期度会永远低估。现在用 `ctx.scene_index`。

**接司机时又逮住一个同源的（第 7 个）——场景 ID 每回合从头重发**：
候选 ID 跨回合重号无害（它们是提议期的临时编号），但**场景 ID 会进投影，重号即覆盖**。
第一版让每回合都从 `scene_0001` 发号，于是第二场戏覆盖第一场、`projection.scenes` 永远
只剩一场（`a_card_is_only_played_once` 因此红过：left 1 / right 2）。现在 `TaskInputs::first_scene`
由 `driver` 传 `ctx.scene_index`、`Workspace::starting_at` 从世界已有的场景数接着发号。
钉在 `tasks::scenes::tests::scene_ids_start_where_the_world_left_off`。

**司机是怎么接进来的（`if-app::turn_runner`）**：`TurnConfig` 打包「结构模型 / 叙事模型 / Jev /
严格度」这些可发送的纯数据；`run()` 拼好 provider 与 judge（`jev_key` 为空则用 `LlmJudge`，
非空则用 `JevJudge`，见 D11）后转调 `run_with()`：读投影与设置 → `pending_if()` 取
**最近一条回合记录里的 `Confirmed` 裁定卡** → `context()` 装上下文 → `run_if_turn` →
`persist()`（先 `append_batch(scene.drafts)` 再 `save_turn`）。三处值得记住：

- **严格度映射**：`strictness_from_percent(0–100)` 以 `DEFAULT_STRICTNESS`（=70）为**中性点**
  分段线性映射到 `[0.5, 2.0]`。默认值必须是中性（1.0），否则「默认设置」本身就悄悄把阈值
  顶到概率上限之外——`q.key.equivalent` 的 0.8 会被乘成 1.12，判定直接失效。
- **没有 pending 卡时不报错**：`pending_if` 返回 `None`，这一回合退化为「无输入的继续回合」。
  同一张卡只会演绎一次（`a_card_is_only_played_once` 钉住幂等）。
- **取消走独立开关**：回合会占住工作线程几十秒，排队的 `cancel` 请求轮不到执行，
  所以 `Request::RunTurn` 自带 `Arc<AtomicBool>`，Tauri 命令 `cancel_turn` 持同一份。

**回合级端到端重放**（[12 §9](12-工程架构.md) 要求的、TODO 里空着的那条）现在有了：
`turn::tests::replaying_the_same_turn_reconstructs_the_same_projection` 用**真实 SQLite**
（内存库 + `append_batch`）跑两遍同一条世界线 —— 事件从库折出投影、喂给回合、
回合产物写回库、再折一次 —— 断言两份投影**逐字节相同**（含序列化键序）。
它同时验了骰子的确定性、草稿的可折叠性与投影只由事件决定这三件事。

### 视图与裁决两层

纯计算、无网络，**已被 `if-pipeline` 调用**（不再是「没有调用方」）：

- `if-views`：把投影编译成「某个消费方有资格看到的形式」。八种视图
  （parse / god / director / pov / narration / check / player / creation）的状态装配、
  预算裁剪、`public / private / secret` 可见性判定外加 L1 保护期，以及判定记录要存的稳定指纹。
  P9（视角隔离）的唯一落点就在这里。**检查视图含未批准揭示的秘密**，叙事视图不含。
- `if-policy`：阈值表（[06 §2](06-裁决策略.md)，含一致性严格度的线性收紧与上下限夹紧）、
  命运骰子的决策键生成与抽样、按 `depends_on` 的分层拓扑排序（≤3 层因果深度）、
  观察带与趋势转化（`p ≥ τ_watch` → 初始压力 `p × 0.5`）、导演评分与场景选择。
- `if-judge`：`Judge` trait + Jev / LLM 裁判 / `StubJudge`。**`StubJudge` 的默认值是 0.5**，
  而「触发类」模板的方向是 `HigherFlags`（越高越可疑）——所以要构造「干净的一回合」，
  必须显式把 `q.beat.*` 那几条压到 0，别以为是桩坏了。

### 前端

- Tauri 启动时读取世界快照，顶部栏显示真实世界名和事件序号。
- Tauri 模式发送 IF 时走 `submit_if_model`，在聊天区显示事件序号和解析草案。
- **确认裁定卡 → 推演一个回合**（命令 `run_turn` + 前端 `playTurn`）：先落一条「场景卡」
  系统消息（场景 ID / 目标 / 视角 / 在场 / 停止条件 / 禁止结果 / 激活的设定条目），
  再一条「任务 ×N ｜ 写入事件 M 条」，然后**警告与被拦节拍**（不折叠——被静默丢掉的正文
  是「我以为我写的还在」的源头），最后正文**按节拍逐段上屏**（节拍之间留时间，因为节拍是
  这个世界「一次推进」的最小单位）。「按重释确认」走同一条路径。世界视图刷新的是**投影**
  （`report.snapshot.projection`），不是重建的壳子。没选出场景或本回合没正文时，如实说明。
- 世界库二级页（`WorldLibrary`）+ 新建会话世界选择器（`WorldPicker`），列表读 `library.db` 的摘要
  （`loreCount` / `sourceCount` / `sessionCount`），选中后再读详情、重建世界视图来播种会话。
- **导入预览（`WorldImportPreview`）**：读文件 → `import_world_file` → 展示角色 / 设定条目 / 警告 / 来源字段 → 用户确认后经 `import_world_file_to_library` 写入世界库。
- 非 Tauri 的 Vite 预览仍使用演示数据和模拟回合；导入 / 手动撰写 / 删除在世界库上**明确提示不支持**，不写假数据。
- 世界库读不出来时**显式横幅报错并清空列表**，不留上一次的陈旧快照。

### 已验证命令

```powershell
cd E:\IF
cargo test --workspace              # 当前 463 个测试通过（if-pipeline 132、if-app 106）
cargo clippy --workspace --all-targets   # 零 lint（只剩 E: 盘「不支持硬链接」的环境提示）

cd E:\IF\app
npx tsc --noEmit
npm run build                       # vite 生产构建
npm run tauri dev                   # 需要桌面验收时
```

> ⚠️ 2026-09-26 实测更正：`cargo` **可以在 Git Bash 里直接跑**（`cargo test --workspace` 19–60s 增量通过），旧笔记里「cargo 会静默死掉」的说法不再成立。
> 若用工具调用，把输出重定向到文件再读最稳：`cargo test --workspace > /tmp/t.log 2>&1`。
> 构建日志里每个 crate 一条 `hard linking files in the incremental compilation cache failed`
> 是 `E:` 盘不支持硬链接导致的，与代码无关。

桌面验收可用（IF 预解析）：

```text
IF 所有人从此无法说谎
让林夏表白
IF 王宫刚刚起火了
IF 骑士一直是失踪的王储
```

导入验收可用：把 `crates/if-app/tests/fixtures/peiyu.v2.json`（或任意酒馆 JSON / PNG 卡）拖进世界库的「导入」，应看到导入预览而不是直接落库。确认后**关掉应用再打开**，资产应还在（这就是这个世界库要解的问题）。同一张卡再导一次，条目数不应翻倍。

`sk-example/` 两张真实 PNG 卡是更好的验收对象（1886 KB / 147 条 / 10 万字那个尤其值得跑）：
它顺带压测了 base64 过 IPC 与 418 KB 来源附件落 SQLite 这条链路——
Rust 侧已由 `tests/library_roundtrip.rs` 覆盖，**但「PNG 文件 → base64 → IPC」这一段只有真机能验**。

会话创建 = 首次游玩，可用流程（**这一段只有真机能验**）：

1. `npm run tauri dev` 启动桌面端。
2. 世界库里选一个已导入的世界 → 点选它。
   - 这个世界**还没有会话**：直接创建，聊天区出现「叙述者开场白」（若卡里有 `first_message`）
     加两条系统简报——一条汇总播种结果（N 个主体 · M 条设定 · 其中 K 条常驻 · 事件日志 N 条 · 世界种子 S），
     一条列出「需要你拿主意的地方」（把 `SeedReport.notes` 逐条摊开，不折叠成一句「已创建」）。
   - 这个世界**已有会话**：先弹 `SessionPicker` 问要用哪个，或新建一个。
3. 顶部栏应显示真实世界名与事件序号；右侧「世界」页签的角色 / 设定条目 / 规则应来自**投影**，
   而不是资产里的原稿。
4. **关掉应用再打开**——这是关键一步，**已真机验收 ✅**。侧栏里那条会话还在（排在最前）。
   点它一下，才会打开它的世界文件；聊天区出现一条现状摘要
   （`已载入会话「…」：事件日志 N 条 · 世界时间 … · 主体 M 个`），右侧世界视图重新填上。
   理由：投影要打开 `.ifworld` 才有，而一次只开一个世界，所以重启只列**占位**、点开才载入。
   > 🐞 **2026-09-26 修掉的真机 bug**：第一版把会话列表只放在前端内存里，
   > 于是「建完会话 → 重启 → 侧栏空了」，而 `world_sessions` 里那条引用明明在。
   > 教训：**持久化做对了不等于用户看得见**（[10 §3.0](10-世界创建与导入.md)）。
5. 在会话里发一条 `IF 所有人从此无法说谎` → 聊天区应落一张**裁定卡**
   （措辞可编辑，`取消` / `确认并锁定` / `按重释确认`）。确认后顶部栏事件序号应 +1。
   ⬜ **待真机确认**（`submit_if_model` 要先配好结构模型）。
6. 顺手验三个闸门（⬜ 都还没真机确认）：
   - 挑一个**正在被会话引用**的世界资产点删除 → 应被拒绝，并报出有几个会话在用；
   - 在侧栏对一条会话点删除 → 应消失，**再重启一次也不回来**（那才是真删了引用），
     并且 `%APPDATA%\io.github.wep56.if\worlds\` 下它的 `.ifworld` 应当一并没了；
   - 一个负面用例：**没配结构模型密钥**时发 IF，应看到明确的报错提示，而不是静默无反应。

> 播种只在**创建**时发生。开场白每次都从报告里重放，不会被写成世界事实——
> 这是刻意的（见 §3），不是 bug。
>
> 侧栏里还会留着几条**演示故事**（`initialStories`），它们背后没有世界文件，
> 是前端早期的示例数据。把「真实会话」和「演示故事」分开是下一刀的事——
> 目前真实会话排在演示故事前面，且只有真实会话能发 IF。

## 4. 当前明确边界

- `parse_if` 与 `import_world_*` 都是**启发式 / 确定性**的，不调用真实结构模型；`parse_if` 也不做 Jev 忠实度判定。
- 导入的**语义抽取**（主体 / 事实 / 规则 / 故事线）尚未实现——导入结果只是忠实的规范化记录 + warnings。
- **三种真实方言仍未覆盖**（只有合成 fixture，见 [13 §6.3](13-酒馆兼容.md)）：V3 `assets` / `group_only_greetings`、正文里的 `@@` 装饰器、独立 `lorebook_v3` 与酒馆运行时 World Info JSON。
  `sk-example/` 两张真实卡实测 `assets = 0`、`带装饰器 = 0`——**别把「跑过真实 PNG 卡」当成「V3 分支也验过了」**。
  其中后两项是**导出产物**，在酒馆里导出一次即可覆盖，比找样本容易。
- `LoreSection::Style` 已存在但没有**任何**自动映射：真实卡的 `position` 只区分角色定义前后，归段靠 T-parse 或用户指定。
- **会话引用已接线**：建会话时 `create_session` → `Library::attach_session` 写入 `world_sessions`，
  删除被引用资产会被拒绝。Rust 侧（`if-store::library` 单测）已验证；**「真机点删除被拦住」这一段没验过**。
- **来源改名 = 新来源**：`source_key` 只取「来源类别 + 名字」，用户把卡改名后再导入会与旧的并存（可见、可删），而不是替换。这是刻意的取舍，见 [10 §7.0](10-世界创建与导入.md)。
- **数字型条目 `id` 未被识别**：`{"0":{"id":7,...}}` 这类条目，`uid` 会回落到 map 键 `"0"` 而不是 `7`（`lorebook::parse_entry` 的 `text(entry, "id")` 只读字符串）。影响的是「回指原文件」的精度，不影响来源身份与替换；等真实卡补测时一并处理。
- **裁定卡 UI + 卡之后那半程都已接线**（`app/src/components/ChatView.tsx` 出卡，
  确认 / 按重释后 `run_turn` 推演一个回合）。仍待在真机上确认的是**真实模型下的连贯性**
  （场景卡与正文是否合理、节拍切分是否自然）——这属于主观叙事质量，由用户验收。
- **`if-pipeline` 已经有司机**：`if-pipeline::tasks` 提供 T-impact / T-scenes / T-plan /
  T-render / T-extract 五个任务，`if-app::turn_runner` 把它们串成一条回合、接进
  `world_worker`，命令面新增 `run_turn` / `cancel_turn`。**但合同目前只用 scripted provider
  验证过**——真实 LLM / Jev 下的产出质量（候选是否合理、场景计划是否贴合、正文与节拍是否
  自然）属于主观叙事质量，必须真机验，别把「测试绿」当成「能玩」。
- **世界模型在 T-parse 之前是空的**：导入一个世界只得到**主体 + 设定条目**；
  命题 / 事实 / 规则全是 0（聂小倩实测：主体 1、设定条目 13、其余全 0）。
  所以回合跑的是**无命题候选**——决策键回退到候选 ID，本回合能跑通、跨世界线不可复现
  （`if-policy` 的 `unstable` 警告）。这不是 bug，是 T-parse 缺失的**可观测后果**（§5 第 1 项）。
  **今天要一份有命题的世界，只能走作者世界包**（§5「已完成：作者世界」），
  或者像 `turn_runner` 的测试那样手工 `sow_proposition`。
- **作者世界还没接进应用**：`if-app::authored` 能把一份手写世界包落成 `.ifworld`，
  但入口只有 `cargo run --example build_world`——`create_session` / 世界库只认导入结果
  （`ImportedWorld`）。所以「临江市」现在**建不了会话**，模板页签也还没有（§5 第 3 项）。
- **世界包的校验与引擎侧折叠是两道**：`authored::validate` 报出文件里**哪一类对象的哪一条**，
  `append_batch` 那道只是「整批不写」。改格式时别只测一道——前者管作者体验，后者管数据安全。
- **`{{user}}` 还没被处理**：真实卡（聂小倩）的 profile 与设定条目里留着裸 `{{user}}`，
  会**原样进视图**。D14 要求把它换成由世界推演的主角；在那之前，模型看到的是一个未解析的模板。
- **`if-lore` 仍未创建**：世界书激活（[08 §5](08-视图与世界书.md)）的逻辑在 `if-pipeline::lore`
  里已落地并被 `context::activate_for_turn` 调用，但按 [12 §3](12-工程架构.md) 它**该住在 `if-lore`**。
  迁移与 T-parse 是同一个触发条件（都要开始处理条目的语义），见 §5。
- 前端：角色 / 世界名 / 设定条目 / 规则已经改由**投影**重建（`app/src/projection.ts`），
  导入 / 手动撰写 / 删除走真实 IPC；**推演卡与按节拍展示已接线**（`run_turn` + `playTurn`）。
  仍然没接投影的是 **IF 导图（世界线）面板**与**趋势面板**。
- `open_world` 已有命令，但前端尚未提供世界文件选择器（v1 会话一律经世界库创建）。
- 世界资产的**世界层内容仍是不透明 payload**（`ImportedWorld` 的 JSON）：`if-store` 不解释它，
  重建世界视图靠 `if-app::library` 反序列化。**这是刻意的分层**，不是待换的临时状态——
  映射的产物是**会话里的事件**（`if-app::seed`），不是库里的资产（[12 §3](12-工程架构.md)）。
  只有等 `library.db` 真的需要按世界字段检索时，才有理由把它换成有类型的结构。
- **播种不解释设定条目的语义**：条目的「承重 / 氛围」分流交给 T-parse 与 LLM 草案
  （[10 §3.0](10-世界创建与导入.md)），播种只搬运。所以世界书条目现在全部按 `Public` 落入
  `LoreEntry`，关键词 / 常驻标志被如实带上；激活由 `if-pipeline::context::activate_for_turn` 做。
- **侧栏混着演示故事**：Tauri 模式下真实会话排在 `initialStories` 前面，但两者仍在同一份
  `stories` 状态里。演示故事背后没有世界文件、发 IF 不会落到它们身上。把两者分开（真实会话单独一组）
  是下一刀，不是现在的阻塞项。
- **聊天记录不持久化**：`.ifworld` 里是**事件**，`messages` 只是前端缓存（[12 §5](12-工程架构.md)）。
  所以重开一条会话看到的是「开场白 + 一条现状摘要」，不是上次的对话原文。
  回合的对账已经能决定「哪些变化该提交」，但**正文本身仍然没有落脚处**——
  事件里只有 `beat_displayed`（节拍原文）。要不要把正文存成事件、还是只是可重放的材料，
  得等 T-render 真接进来、看清正文的用途之后再定，别现在猜。
- **删除会话可能留下孤儿文件**：`delete_session` 先删引用再删文件，若文件仍被别处占用
  （杀不掉的句柄、权限不足），引用没了而 `.ifworld` 留在 `worlds/` 下。它不会再出现在任何列表里，
  但也不会被自动清理——需要一个「整理世界文件」的入口【后续】。
- **`if-pipeline` 里几处语义选择还没被文档裁定**（改动前先看这里，别当 bug 顺手改）：
  - `resolve` 里 `fact_set` 的 `Visibility` 只按 `internal` 分（外在 → `Public`，内在 → `Private`）；
  - `SceneCommit.completed_at` 目前总是 `Some(...)`（场景在本回合收束），
    没有表达「演到一半被打断」的路径；
  - `beats` 逐节拍检查的**背压**（[04 §4.7](04-回合流程.md)）不在这里——那是流式读取侧的事，
    本模块只看已经切好的片段。

## 5. 下一阶段工作顺序

### 已完成：酒馆适配第一步（角色卡 / 世界书导入解析）

- ✅ 联网核实 CCv3 规范与酒馆运行时字段，关闭 [13 §6](13-酒馆兼容.md) 待核实清单 1–3。
- ✅ PNG 文本块（`chara` / `ccv3`，含 `tEXt` / `zTXt` / `iTXt`）解析。
- ✅ 双方言世界书条目归一化 + 装饰器剥离 + `position` → 归段。
- ✅ 导入预览 UI 与 IPC 接线。
- ✅ 公开 CC BY 样本纳入回归（`crates/if-app/tests/fixtures/`）。
- ✅ 真实 PNG 卡（spec 3.0）体检并按实测行为修正导入器：字段分层回退、`extensions.position` 优先归段、定时效果零值判定、空条目跳过提示、去掉臆造的 `genre`（[13 §6.4](13-酒馆兼容.md)）。

### 已完成：harness 的确定性两层（视图编译 + 裁决策略）

- ✅ `if-views`：八种视图的状态装配、预算裁剪、可见性判定（`public` / `private` / `secret` + L1 保护期）、稳定指纹。P9 的落点。
- ✅ `if-policy`：阈值表与一致性严格度、命运骰子（决策键 + 抽样）、分层拓扑排序、观察带与趋势转化、导演评分与场景选择。
- ✅ 两处 domain 缺口一并补上：`Candidate::key`（稳定决策键，跨世界线不变）、`ResolutionPolicy::SeededCategorical`（docs/06 §1 的互斥类策略原本在枚举里是缺的）。

### 已完成：世界资产持久化（`library.db`）

- ✅ `if-store::library`：独立 `library.db`，四张表 + 元信息；世界资产 / 来源 / 设定条目 / 会话引用。
- ✅ `if-app::library`：`ImportedWorld` ↔ 存储形状的映射，含来源身份键与内容哈希。
- ✅ 按来源整组替换（重导不留残影、不误伤别的来源）；来源键打错会被拒绝而不是造孤儿行。
- ✅ 前端世界库读写全部走 IPC；列表与详情分开，删除被引用时如实报错。
- ✅ 会话引用在建会话时登记（见下）。

### 已完成：会话创建与播种（**第一次能玩了**）

- ✅ `if-app::seed`：`ImportedWorld → if-domain` 的确定性映射（主体 / 常驻情境条目 / 设定条目），
  外加一份会说清「哪些没做、为什么」的 `SeedReport`。21 项单测。
- ✅ `if-store::Store::next_seq()`：让播种能在**写之前**算出事件 ID；分配规则由
  `event_ids_follow_next_seq` + `plan_matches_store_allocation` 两侧钉住。
- ✅ `if-app::session`：创建（写 `.ifworld` + 登记引用）/ 列举 / 恢复 / 删除，
  13 项单测（用**真实文件与真实 SQLite**，不用内存库——要验的就是「关掉再打开还在」）。
- ✅ 命令面：`create_session` / `list_sessions` / `list_all_sessions` / `open_session` / `delete_session`；**删掉了 `create_world`**。
- ✅ 前端：选世界 → （有会话时先问用哪个）→ 真建会话 → 用**投影**重建角色 / 世界 / 设定条目面板，
  并把播种报告的「需要你拿主意的地方」渲染成可见消息。
- ✅ **重启后会话还在**（真机反馈后补上）：侧栏一启动就读 `library.db` 的会话，渲染成**占位**条目，
  点开才打开世界文件。删除会话会把引用与世界文件一起删掉。
  （第一版没有这一步，结果是「建完会话重启就找不到了」——见 [10 §3.0](10-世界创建与导入.md)。）
- ✅ 顺手修掉一个真缺陷：`new_world_path` 原先只保留 ASCII，中文标签会静默塌成 `world`——
  抽出 `if-app::slug`（保留 CJK），两个调用方共用。
- ⚠️ 播种**不猜语义**：只有确定性搬运，不判断条目是事实还是氛围；`{{user}}` 也还没处理（D14，
  见 [10 §7.1](10-世界创建与导入.md)）。报告里会把这些列出来，而不是默默做掉。

### 已完成：`if-pipeline`（一个可提交的 IF 回合）

- ✅ 七个模块全部落地：`context` / `audit` / `candidates` / `scenes` / `beats` / `commit` / `turn`，
  101 项单测（`cargo test -p if-pipeline`）；接上 `tasks` 层后共 132 项。
- ✅ `EventDraft` 从 `if-store` 下沉到 `if-domain`——`if-pipeline` 不该为了写一个草稿而依赖 SQLite
  （`if-store::EventDraft` 的路径由 re-export 保持不变）。
- ✅ 世界书激活接进回合（[08 §5](08-视图与世界书.md)）：`context::activate_for_turn` 是它第一个真实调用方。
- ✅ 回合级端到端重放（TODO 里空着的那条）：同一条世界线在**真实 SQLite** 上跑两遍 → 投影逐字节相同。
- ✅ 顺手修掉 §3 列的 6 个缺陷，每个都有对应测试钉住。
- ✅ **已接上司机**（见下一节）。

### 已完成：把 `if-pipeline` 接上司机（五个 agent 任务 + 回合驱动）

- ✅ `if-pipeline::tasks`：T-impact / T-scenes / T-plan / T-render / T-extract 五个任务
  + `ProposalHost` + `Workspace`（发号起点由 `TaskInputs::first_scene` 指定）。
- ✅ `driver::run_if_turn`：串起五个任务与两段驱动；`TurnProviders` 把模型分成
  **结构槽位与叙事槽位**，T-render 走叙事模型、其余走结构模型。
- ✅ `if-app::turn_runner`：`TurnConfig` / `TurnReport`，`run()` 拼 provider + judge →
  `pending_if()` 取最近回合的 `Confirmed` 卡 → `run_if_turn` → `persist`。
- ✅ Tauri 命令 `run_turn` / `cancel_turn`；`world_worker` 的 `RunTurn` 请求自带取消开关。
- ✅ 前端：`session.ts` 的类型与 IPC，`App.tsx` 的 `playTurn`（场景卡 + 任务 + 警告 +
  被拦节拍 + 按节拍逐段上屏的正文）。
- ✅ 顺手修掉场景 ID 重号覆盖（见 §3 第 7 个缺陷）。
- ⚠️ **合同用 scripted provider 验证**；真实 LLM / Jev 下的连贯性由用户真机验收。
  首次真机（2026-09-27，聂小倩卡）**失败在第一步 T-impact**——不是模型不听话，是契约在
  「还没有命题」的世界里不可满足；已修（见 §3 那条真机缺陷 + `dump_turn_prompt` 诊断入口）。
  **待再验一次。**

### 已完成：作者世界（补上导入路径产不出的**命题**）

- ✅ `if-app::authored`：`AuthoredWorld`（一份手写世界包）→ 事件的确定性映射。
  顶层字段与 `if-domain` 的领域类型一一对应，只有一处刻意的不同——**凡是「由哪个事件引入」
  的字段（`created_by` / `source`）都不在文件里**，由加载器回填。
  写入顺序固定：**主体 → 命题 → 事实 → 规则 → 设定 → 故事线**；事件 ID 先算号再写，
  与 `seed` 同一套契约（`Store::next_seq` → 预分配 → `append_batch`）。
- ✅ **先校验再映射**：悬空引用（事实挂不存在的命题、规则作用于不存在的主体、条件里的命题 ID
  不存在）、ID / 规范键重复、字段名打错（`deny_unknown_fields`）——**一次报出全部问题**，
  并指出是哪一类对象的哪一条。引擎侧 `append_batch` 那道保险说不出文件里的位置，所以校验在这里。
- ✅ `examples/build_world`：世界包 → `.ifworld`（`--check` 只校验）。它是 `authored` 目前
  **唯一的非测试调用方**。
- ✅ 示例世界 **「临江市」**（`examples/worlds/linjiang.world.json`）：主体 5 / 命题 8 / 事实 8 /
  规则 3 / 设定 7 / 故事线 3。按 [01 §5](01-IF规则.md) 的**六种 IF 类型**各留了落点
  （状态型 `c_shen.trust.c_zhou`、认知型 `c_zhou.knows_tampering`、规则型带机制与触发器、
  真相型 `c_reservoir.data_tampered`〔secret〕……）。格式说明见同目录 `README.md`。
- ✅ 测试夹具 **「渡口」**（`tests/fixtures/worlds/ferry.world.json`，2 / 3 / 3 / 1 / 2 / 1）
  + **成对**的端到端测试（`src/authored/tests.rs`）：同一份剧本、同一个裁判，
  **有命题 → 候选落在真实命题上、决策键稳定；抽走命题 → 引擎如实报出「没有稳定决策键」**。
  这一对把「命题是 IF 的着力点」钉成一条可执行的事实，而不是文档里的一句话。
- ✅ 顺手修：T-extract 的工具描述写「视图里 propositions 的 key」，而 `ViewState` 里根本没有
  `propositions`——命题是以 `facts[].key` 暴露的（`impact.rs` 早已这么写）。这正是
  §3 那条真机缺陷的同一类问题：**契约没对齐，模型只能编**。
- ⚠️ **还没接进应用**：`create_session` 只认导入结果（`ImportedWorld`），作者世界目前只能经
  `build_world` 落成文件。接进去是可做的下一步（见下第 3 项）。

### 下一步（按优先级）

1. **T-parse / 语义抽取**（**当前最高优先**）：把设定条目里「承重的那一半」抽成命题 / 事实 /
   规则草案，接 `q.extract.faithful` 校验（[10 §3.0](10-世界创建与导入.md)、[02 §9.1](02-世界模型.md)）。
   这也是 `if-lore` 与 `importer/` 迁位的触发条件（[12 §3](12-工程架构.md)）——
   世界书激活现在住在 `if-pipeline::lore`，该搬过去。
   **真机已经证明它不是「锦上添花」而是前提**：没有命题，回合的第一步只能提无键候选
   （§3 那条真机缺陷）。
2. **真机再验一次完整回合**（配好结构 / 叙事模型密钥后由用户执行）：
   发 `IF xxx` → 确认裁定卡 → 看场景卡与正文是否合理、节拍切分是否自然。
   **这是唯一能证明「真正能玩」的一步**；我负责准备可运行入口 + 结果记录模板。
   > 2026-09-27 第一次尝试**失败在 T-impact**（契约问题，已修）。若再跑仍失败，
   > 报错现在会带上**模型原话 + 过程提示**（max_tokens / 重试），据此就能定位；
   > 视结果再决定正文要不要落成事件（§4）。
3. **把作者世界接进 `create_session`**（小，但让上面那条「建议先用临江市」变得可执行）：
   `authored` 加载器已经写好了，缺的是三处接线——资产 `payload` 加一个判别字段、
   `session::load` 认两种材料、`CreateRequest` 收一个「导入 / 作者」的枚举。
   做完之后世界库里就能看到「临江市」，建会话、发 IF、推进回合一条龙；在那之前
   它只能经 `cargo run --example build_world` 落成文件。
   顺带该决定：作者世界在界面上的入口是「新建世界时多一个页签」，还是「导入 `.world.json`」。
4. **IF 导图 / 世界线面板接投影**：前端目前只重建了角色 / 世界观 / 设定条目；
   `Story.map` 里的节点还是前端编的，没接世界线与分支。
5. **收尾两把小刀**：侧栏把「真实会话」与「演示故事」分到不同分组；
   给个「整理世界文件」入口清理孤儿 `.ifworld`（见 §4）。
6. **阈值校准**（[06 §2](06-裁决策略.md)，每模板约 40 条标注集）——v1 动工前的必做项。
   `q.beat.violates_fact` 合规侧余量最小（实测合规 0.21 vs 阈值 0.3），优先。
   现在 `if-pipeline` 把阈值用在真实路径上了，校准的收益从「理论」变成「每回合都感觉得到」。
7. ⏸️ **真实文件补测（剩余）**——独立 `lorebook_v3`、酒馆运行时导出的 World Info JSON、
   含 V3 扩展字段的真实卡、带装饰器的卡（[13 §6.3](13-酒馆兼容.md)）。
   **用户 2026-09-26 明确说「暂时没空测试，晚点吧」**：别把它当成压着别人的待办，
   也别因为它没做就不敢动关键路径——这几项是**导出产物**，在酒馆里导出一次即可覆盖。
   顺手可做的是数字型条目 `id` 的 `uid` 识别（§4 有说明）。

## 6. 接手时先读什么

建议顺序：

1. 本文件；
2. `TODO.md`；
3. [15 v1 范围](15-v1范围.md)；
4. [04 回合流程](04-回合流程.md)（`if-pipeline` 就是它的实现，读它能少走一半弯路）、
   [06 裁决策略](06-裁决策略.md)、[02 §10–§11](02-世界模型.md)（场景 / 节拍 / 三阶段对账）；
5. [13 酒馆兼容](13-酒馆兼容.md)（酒馆适配的字段依据，§0.1 的双方言表是重点）；
6. [10 世界库与导入](10-世界创建与导入.md) §2 / §7；
7. 代码：**`crates/if-pipeline/src/`（这一轮的主体，先读 `lib.rs` 的模块表再按表读）**、
   `crates/if-pipeline/src/tasks/`（**司机层**：唯一认识 LLM 的地方，先读 `mod.rs` 的模块表，
   再按 `driver.rs` → `host.rs` → 各任务读）、
   `crates/if-app/src/turn_runner.rs`（**把司机接进 `if-app` 的回合驱动**，读它能看清
   「确认裁定卡 → 推进一回合」这条链）、
   `crates/if-app/examples/dump_turn_prompt.rs`（诊断：把某个世界的上帝视图 dump 出来，
   排查「模型没有产出提议」的第一站）、
   `crates/if-app/examples/build_world.rs` + `crates/if-app/examples/worlds/README.md`
   （**作者世界包**的入口与格式：想看「一份适合 IF 的世界长什么样」，读 `linjiang.world.json`）、
   `crates/if-app/src/importer/`、`world_worker.rs`、`library.rs`、`seed.rs`、`authored.rs`、
   `session.rs`、`slug.rs`、
   `crates/if-store/src/library.rs`、`crates/if-store/src/store.rs`（看 `next_seq` 与 `append_batch`）、
   `app/src/library.ts`、`app/src/session.ts`、`app/src/projection.ts`、`app/src/App.tsx`
   （`playTurn` 把一回合摆到聊天区）、
   `app/src/components/ChatView.tsx`（裁定卡 UI 在这里，`pending_if` 一出现就渲染）；
8. 若要改回合编排，先读 `crates/if-views/src/`（视图与可见性）与 `crates/if-policy/src/`
   （阈值表、命运骰子、分层裁决、导演评分）——它们是纯计算层，读起来没有副作用，
   接的时候只需要「喂输入、取输出」。

不要从旧会话推断产品状态；以仓库文档、测试和当前工作区代码为准。真实账号、真实 Jev/LLM 和主观 UI/叙事质量验收仍由用户执行。
