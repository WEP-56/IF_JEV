# IF 项目交接说明

> 面向下一位接手者的快速恢复文档。设计细节以 `docs/00–15` 为准；本文件只记录当前工程状态、已验证入口和下一阶段顺序。
> 最后核对：**2026-09-26**（此前的版本落后于代码，已按工作区实际情况重写）。

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
crates/if-store/        SQLite 事件日志、投影、快照、世界线
crates/if-agent/        Agent loop、provider、工具 schema
crates/if-judge/        Jev、LLM 裁判、测试桩与重试
docs/                   正式设计与工程文档
```

crate 划分见 [12 §3](12-工程架构.md)。`if-views` / `if-policy` / `if-lore` / `if-pipeline` **尚未创建**。

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
- `import_world_json` / `import_world_file`：酒馆导入。自动识别 PNG（`chara` / `ccv3` 文本块，优先 `ccv3`）与 UTF-8 JSON；同时兼容 CCv3 规范与酒馆运行时两套字段方言；含装饰器剥离、`position` → 归段映射、宏检测（`{{user}}` 等）。
- 诊断：`test_llm` / `probe_jev` / `test_judge`。

### 前端

- Tauri 启动时读取世界快照，顶部栏显示真实世界名和事件序号。
- Tauri 模式发送 IF 时走 `submit_if_model`，在聊天区显示事件序号和解析草案。
- 世界库二级页（`WorldLibrary`）+ 新建会话世界选择器（`WorldPicker`）。
- **导入预览（`WorldImportPreview`）**：读文件 → `import_world_file` → 展示角色 / 设定条目 / 警告 / 来源字段 → 用户确认后才进世界库。
- 非 Tauri 的 Vite 预览仍使用演示数据和模拟回合；导入在无 Tauri 时**明确提示**，不用假数据顶替。

### 已验证命令

```powershell
cd E:\IF
cargo test --workspace              # 当前 141 个测试通过

cd E:\IF\app
npx tsc --noEmit
npm run build                       # vite 生产构建
npm run tauri dev                   # 需要桌面验收时
```

> ⚠️ 2026-09-26 实测更正：`cargo` **可以在 Git Bash 里直接跑**（`cargo check -p if-domain` 13s 通过），旧笔记里「cargo 会静默死掉」的说法不再成立。

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
- 导入结果**尚未持久化**：没有独立的世界资产表 / 文件格式，世界库仍是前端内存态，刷新即丢。
- 尚无裁定卡 **UI**（Rust 侧命令与 domain 类型已具备）。
- 尚无候选生成、分层裁决、命运骰子、场景计划、节拍检查、正文回收和提交闭环。
- 前端目前仍以演示故事为主要视觉数据源，真实投影尚未映射成角色 / 世界 / 导图面板。
- `open_world` 已有命令，但前端尚未提供世界文件选择器。

## 5. 下一阶段工作顺序

### 已完成：酒馆适配第一步（角色卡 / 世界书导入解析）

- ✅ 联网核实 CCv3 规范与酒馆运行时字段，关闭 [13 §6](13-酒馆兼容.md) 待核实清单 1–3。
- ✅ PNG 文本块（`chara` / `ccv3`，含 `tEXt` / `zTXt` / `iTXt`）解析。
- ✅ 双方言世界书条目归一化 + 装饰器剥离 + `position` → 归段。
- ✅ 导入预览 UI 与 IPC 接线。
- ✅ 公开 CC BY 样本纳入回归（`crates/if-app/tests/fixtures/`）。

### 下一步（按优先级）

1. **世界资产持久化**：`if-store` 增加独立的世界资产表 / 文件格式；世界资产不能复用会话 `world_created` 事件，也不能在新建会话时隐式创建世界。导入确认后写入，并在重启后仍可见。
2. **真实文件补测**：真实 PNG 卡、V3 扩展字段卡、独立 `lorebook_v3`、酒馆运行时导出的 World Info JSON——由用户提供（见 [13 §6.3](13-酒馆兼容.md)）。
3. **导入 → 世界模型的确定性映射**：把 `ImportedWorld` 转成 `if-domain` 的主体 / 设定条目 / 规则草案，再接 T-parse 与 `q.extract.faithful` 校验。
4. **裁定卡 UI**：把已有的 pending / confirm / cancel / reinterpret 命令接到前端。
5. **一个可提交的 IF 回合**：按 [04](04-回合流程.md) 实现 T-impact → 分层裁决 → 场景候选 → 导演选择 → 场景计划 → 逐节拍检查 → Proposed / Observed / Committed 对账。第一版可先用 Judge stub + scripted provider 全流程跑通，再接真实 LLM/Jev。
6. **真实投影前端化** → **视图隔离与 v1 验收**（[15](15-v1范围.md)）。

## 6. 接手时先读什么

建议顺序：

1. 本文件；
2. `TODO.md`；
3. [15 v1 范围](15-v1范围.md)；
4. [13 酒馆兼容](13-酒馆兼容.md)（酒馆适配的字段依据，§0.1 的双方言表是重点）；
5. [10 世界库与导入](10-世界创建与导入.md) §2 / §7；
6. [04 回合流程](04-回合流程.md)、[01 IF 规则](01-IF规则.md)、[03 事件与世界线](03-事件与世界线.md)；
7. 代码：`crates/if-app/src/importer/`、`world_worker.rs`、`app/src/App.tsx`。

不要从旧会话推断产品状态；以仓库文档、测试和当前工作区代码为准。真实账号、真实 Jev/LLM 和主观 UI/叙事验收仍由用户执行。
