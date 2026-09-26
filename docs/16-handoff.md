# IF 项目交接说明

> 面向下一位接手者的快速恢复文档。设计细节以 `docs/00–15` 为准；本文件只记录当前工程状态、已验证入口和下一阶段顺序。
> 最后核对：**2026-09-26**（此前的版本落后于代码，已按工作区实际情况重写；`if-views` / `if-policy` 落地后再次核对）。

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
crates/if-app/          Tauri 命令、事件、设置、密钥、世界工作线程、IF 预解析、酒馆导入
  src/importer/         酒馆角色卡 / 世界书导入（card / lorebook / png / model / value）
crates/if-domain/       领域类型、事件补丁、投影、世界线、回合记录、裁定卡
crates/if-views/        视图编译、可见性判定、预算裁剪、稳定指纹
crates/if-policy/       阈值表、命运骰子、分层裁决、观察带与趋势、导演评分
crates/if-store/        SQLite 事件日志、投影、快照、世界线
crates/if-agent/        Agent loop、provider、工具 schema
crates/if-judge/        Jev、LLM 裁判、测试桩与重试
docs/                   正式设计与工程文档
```

crate 划分见 [12 §3](12-工程架构.md)。`if-lore` / `if-pipeline` **尚未创建**；
已落地 crate 之间的依赖是单向的 `if-domain → if-views → if-policy`，
`if-pipeline` 建成后依赖全部四者。

关键原则：

1. 每个 `.ifworld` 文件由一个 `WorldWorker` 专属线程持有 SQLite 连接。
2. 事件只追加，不直接修改投影；分支和回滚只移动世界线头指针。
3. API 密钥走系统钥匙串，不进入 localStorage 或世界文件。
4. 真实 LLM/Jev 和叙事质量由用户验收；Agent 负责静态、模拟和可自动化测试。
5. 单个源文件 ≤ 1000 行；接近上限按职责拆模块（`AGENTS.md`）。

## 3. 当前已打通

### Rust / Tauri

世界生命周期：

- `create_world`：在应用数据目录的 `worlds/` 下创建 `.ifworld`。
- `open_world` / `close_world` / `get_world_snapshot`；`world://opened` / `world://closed` 事件。

IF 流程（**裁定卡生命周期已实现**）：

- `submit_if` / `submit_if_model`：在世界工作线程中串行追加输入，先落一张 **`pending` 裁定卡**。
- `confirm_if` / `reinterpret_if` / `cancel_if`：确认才产生 `if_injected` 事件并更新投影。
- 冲突预检：本地确定性检查能标出与已有事实相反的断言（`IfConflict` + `IfConflictResolution::Reinterpret`）。
- `if-domain::turn` 内有 `IfRulingCard` + `IfCardStatus{Pending,Confirmed,Cancelled}` + `IfConflict`，`world_worker.rs` 有对应单测。

解析与导入（确定性、无网络）：

- `parse_if` / `parse_if_model`：IF 预解析，输出导演指令标记、类型初判、时间锚点、作用范围、锁定建议、不承诺项与警告。
- `import_world_json` / `import_world_file`：酒馆导入。自动识别 PNG（`chara` / `ccv3` 文本块，优先 `ccv3`）与 UTF-8 JSON；同时兼容 CCv3 规范与酒馆运行时两套字段方言；含装饰器剥离、`position` → 归段映射、宏检测（`{{user}}` 等）。条目字段按「顶层 → `extensions`」分层读取（真实卡把引擎状态放在 `extensions`，见 [13 §6.4](13-酒馆兼容.md)）。
- `examples/inspect_card.rs`：对真实卡文件做导入体检（`cargo run -p if-app --example inspect_card -- <文件...>`），确定性、不联网、不落盘。
- 诊断：`test_llm` / `probe_jev` / `test_judge`。

裁决与视图（纯计算、无网络，尚未接入回合流程）：

- `if-views`：把投影编译成「某个消费方有资格看到的形式」。八种视图（parse / god / director / pov / narration / check / player / creation）的状态装配、预算裁剪、`public / private / secret` 可见性判定外加 L1 保护期，以及判定记录要存的稳定指纹。P9（视角隔离）的唯一落点就在这里。
- `if-policy`：阈值表（docs/06 §2，含一致性严格度的线性收紧与上下限夹紧）、命运骰子的决策键生成与抽样（发生类 / 互斥类 / 数值机制）、按 `depends_on` 的分层拓扑排序（≤3 层因果深度，超出的推迟或转趋势）、观察带与趋势转化（`p ≥ τ_watch` → 初始压力 `p × 0.5`）、导演评分与场景选择。
- 两者都只被单元测试覆盖，**还没有调用方**——接进回合流程是 `if-pipeline` 的活。

### 前端

- Tauri 启动时读取世界快照，顶部栏显示真实世界名和事件序号。
- Tauri 模式发送 IF 时走 `submit_if_model`，在聊天区显示事件序号和解析草案。
- 世界库二级页（`WorldLibrary`）+ 新建会话世界选择器（`WorldPicker`）。
- **导入预览（`WorldImportPreview`）**：读文件 → `import_world_file` → 展示角色 / 设定条目 / 警告 / 来源字段 → 用户确认后才进世界库。
- 非 Tauri 的 Vite 预览仍使用演示数据和模拟回合；导入在无 Tauri 时**明确提示**，不用假数据顶替。

### 已验证命令

```powershell
cd E:\IF
cargo test --workspace              # 当前 238 个测试通过

cd E:\IF\app
npx tsc --noEmit
npm run build                       # vite 生产构建
npm run tauri dev                   # 需要桌面验收时
```

> ⚠️ 2026-09-26 实测更正：`cargo` **可以在 Git Bash 里直接跑**（`cargo check -p if-domain` 13s 通过），旧笔记里「cargo 会静默死掉」的说法不再成立。
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

导入验收可用：把 `crates/if-app/tests/fixtures/peiyu.v2.json`（或任意酒馆 JSON / PNG 卡）拖进世界库的「导入」，应看到导入预览而不是直接落库。

## 4. 当前明确边界

- `parse_if` 与 `import_world_*` 都是**启发式 / 确定性**的，不调用真实结构模型；`parse_if` 也不做 Jev 忠实度判定。
- 导入的**语义抽取**（主体 / 事实 / 规则 / 故事线）尚未实现——导入结果只是忠实的规范化记录 + warnings。
- `LoreSection::Style` 已存在但没有**任何**自动映射：真实卡的 `position` 只区分角色定义前后，归段靠 T-parse 或用户指定。
- 导入结果**尚未持久化**：没有独立的世界资产表 / 文件格式，世界库仍是前端内存态，刷新即丢。
- 尚无裁定卡 **UI**（Rust 侧命令与 domain 类型已具备）。
- `if-views` / `if-policy` 已落地且单测全绿，但**没有任何调用方**。也就是说：
  「谁有权看到哪些事实」和「概率怎么变成结果」两件事都已经能算，只是回合流程还没去用它们。
- 尚无候选生成、场景计划、节拍检查、正文回收和提交闭环——这些属于尚未创建的 `if-pipeline`。
- 前端目前仍以演示故事为主要视觉数据源，真实投影尚未映射成角色 / 世界 / 导图面板。
- `open_world` 已有命令，但前端尚未提供世界文件选择器。

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
- ⚠️ 都还没有调用方，也没接真实 Jev——它们只被单元测试覆盖。

### 下一步（按优先级）

1. **世界资产持久化**：`if-store` 增加独立的世界资产表 / 文件格式；世界资产不能复用会话 `world_created` 事件，也不能在新建会话时隐式创建世界。导入确认后写入，并在重启后仍可见。
2. **条目 `origin` 字段**：来源类型 + 来源卡 / 文件 + 条目 uid + 卡版本，用于「重新导入同一张卡时按来源整组替换」与多卡合并去重（[13 §0.2](13-酒馆兼容.md)）。
3. **真实文件补测（剩余）**：独立 `lorebook_v3`、酒馆运行时导出的 World Info JSON、含 V3 扩展字段的真实卡、带装饰器的卡——由用户提供（[13 §6.3](13-酒馆兼容.md)）。
4. **导入 → 世界模型的确定性映射**：把 `ImportedWorld` 转成 `if-domain` 的主体 / 设定条目 / 规则草案，再接 T-parse 与 `q.extract.faithful` 校验。
5. **裁定卡 UI**：把已有的 pending / confirm / cancel / reinterpret 命令接到前端。
6. **`if-pipeline`：一个可提交的 IF 回合**：按 [04](04-回合流程.md) 实现 T-impact → 分层裁决 → 场景候选 → 导演选择 → 场景计划 → 逐节拍检查 → Proposed / Observed / Committed 对账。
   - 裁决与视图两层已经就绪（`if-policy` / `if-views`），本阶段要写的是**编排**：任务定义、`IfTaskHost`、工具实现、失败兜底，以及把 Jev 的判定喂进 `Policy::run`。
   - 第一版用 Judge stub + scripted provider 全流程跑通，再接真实 LLM/Jev。
7. **真实投影前端化** → **视图隔离与 v1 验收**（[15](15-v1范围.md)）。

## 6. 接手时先读什么

建议顺序：

1. 本文件；
2. `TODO.md`；
3. [15 v1 范围](15-v1范围.md)；
4. [13 酒馆兼容](13-酒馆兼容.md)（酒馆适配的字段依据，§0.1 的双方言表是重点）；
5. [10 世界库与导入](10-世界创建与导入.md) §2 / §7；
6. [04 回合流程](04-回合流程.md)、[01 IF 规则](01-IF规则.md)、[03 事件与世界线](03-事件与世界线.md)、[06 裁决策略](06-裁决策略.md)、[08 视图与世界书](08-视图与世界书.md)；
7. 代码：`crates/if-app/src/importer/`、`world_worker.rs`、`app/src/App.tsx`；
8. 若要接回合流程，先读 `crates/if-views/src/`（视图与可见性）与 `crates/if-policy/src/`
   （阈值表、命运骰子、分层裁决、导演评分）——它们是纯计算层，读起来没有副作用，
   接的时候只需要「喂输入、取输出」。

不要从旧会话推断产品状态；以仓库文档、测试和当前工作区代码为准。真实账号、真实 Jev/LLM 和主观 UI/叙事验收仍由用户执行。
