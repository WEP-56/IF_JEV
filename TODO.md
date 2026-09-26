# IF TODO

## 已完成

- [x] 阅读现有正式设计文档
- [x] 创建 TODO 清单
- [x] 创建根目录 AGENTS.md

## 规范与决策

- [x] 确认 docs 中的 D1–D15 待定项
- [x] 固定 v1 范围与暂缓功能（见 docs/15）
- [x] 确认 Jev API（端点、请求 / 响应 schema、计价、稳定性，见 docs/14）
- [ ] 固定问题模板版本与阈值（需先做标注集校准）

## 工程骨架

- [x] 创建 Rust workspace 与 IF crate 划分（先落 `if-domain` / `if-store`，其余按需再建）
- [x] 接入 Onemore agent loop（`if-agent`：loop、三种 provider 含 Chat Completions、工具与 schema 校验；`IfTaskHost` 随 `if-pipeline` 实现）
- [x] 建立领域类型与 serde schema
- [x] 建立 SQLite 事件日志、投影和世界线存储
- [x] 建立 Judge trait、Jev 后端与测试桩（`if-judge`：Jev、LLM 裁判、测试桩、重试）
- [x] 接通桌面端真实 LLM 工具调用与 Jev 判定诊断入口（真实账号验收仍待用户）
- [ ] 建立 Tauri 命令、事件和单世界工作线程
  - 已接入 `create_world` / `open_world` / `close_world` / `get_world_snapshot`，世界 SQLite 连接由专属线程持有；聊天回合命令和前端状态接入仍待完成。

## 核心回合

- [ ] 实现 IF 解析、导演指令改写和裁定卡数据
  - 已有 `submit_if` / `submit_if_model` 落盘入口：先落一张 `pending` 裁定卡，不直接冒充事实。
  - 已有 `confirm_if` / `reinterpret_if` / `cancel_if`，以及本地确定性冲突预检（`IfConflict` / `IfConflictResolution::Reinterpret`）。
  - 已增加确定性启发式 `parse_if` 草案（指令检测、类型初判、时间锚点、锁定建议）。
  - 真实 T-parse、语义级冲突预检与裁定卡 UI 仍待完成。
- [ ] 实现 IF 锁定、冲突处理和最小承诺（回溯型 v1 只做重释）
  - 已有确定性冲突预检的雏形（相反断言识别 + 重释）；锁定等级与保护期尚未接入。
- [ ] 实现候选生成、Jev 判定、命运骰子和裁决策略
- [ ] 实现场景计划、节拍检查和展示边界
- [ ] 实现正文回收与 Proposed / Observed / Committed 对账
- [ ] 实现继续回合、观测和基础后台结算

## 视图与导入

- [ ] 实现上帝、导演、角色、叙事、检查、解析和玩家视图
- [ ] 实现世界书激活与预算裁剪
- [ ] 实现世界创建、世界卡和角色定型
- [ ] 实现酒馆角色卡与世界书导入
  - [x] 确定性解析：V1 / V2 / V3 卡、PNG `chara` / `ccv3` 文本块、CCv3 与酒馆运行时两套世界书方言（`crates/if-app/src/importer/`）
  - [x] 联网核实 `docs/13 §6` 待核实清单 1–3，并补公开 CC BY 样本回归（`crates/if-app/tests/fixtures/`）
  - [x] 导入预览（`WorldImportPreview`）与 IPC 接线；用户确认后才进世界库
  - [x] 真实 PNG 卡体检并按其行为修正导入器（`docs/13 §6.4`；体检入口 `cargo run -p if-app --example inspect_card`）
  - [ ] 导入结果持久化为独立世界资产（不能复用会话 `world_created` 事件）
  - [ ] 导入 → `if-domain` 的确定性映射（主体 / 设定条目 / 规则草案）
  - [ ] 条目加 `origin` 字段（来源类型 + 来源卡 / 文件 + 条目 uid + 卡版本），支持按来源整组替换
  - [ ] 独立 `lorebook_v3`、酒馆运行时 World Info JSON、多世界书合并去重补测

## 前端

- [ ] 将 frontend-example 对齐为 IF 三种动作：IF、观测、继续
- [ ] 实现裁定卡、推演卡和按节拍展示
- [ ] 实现角色、世界、趋势和 IF 导图面板
- [ ] 实现重写、重掷、分支、回滚和未选之路（旧世界残影推迟，见 docs/15）

## 验证与交付

- [ ] 添加事件重放、投影、视图隔离和工具 schema 测试
- [ ] 用真实 Jev / LLM 配置完成一轮端到端演练
- [ ] 由用户完成真实界面流程、文案和叙事质量验收
- [ ] 记录已知限制并整理 v1 发布清单
