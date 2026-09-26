# 导入回归测试样本

这些文件用于 `importer` 的回归测试，均为**公开许可**的第三方样本，仅作本地测试输入。

| 文件 | 覆盖 | 来源 | 许可 |
|---|---|---|---|
| `peiyu.v2.json` | 中文单角色 V2 卡；含 `character_book`（5 条条目，含 `constant` 与关键词条目） | [foreverse-app/character-card-skills](https://github.com/foreverse-app/character-card-skills) · `cards/zh/peiyu/peiyu.v2.json` | 代码 MIT；卡片与文档 CC BY 4.0 |
| `nie-xiaoqian.v2.json` | 中文角色 V2 卡；12 条条目、`alternate_greetings`、`system_prompt` / `post_history_instructions` | 同上 · `cards/zh/nie-xiaoqian/nie-xiaoqian.v2.json` | 同上 |

## 使用约定

- 这两个文件是 **V2 卡外壳 + CCv3 格式的 `character_book`**（条目用 `keys` / `insertion_order` /
  `enabled` / `position: "before_char"`），正是需要兼容的真实形态。
- 它们只覆盖 **JSON** 导入。PNG 内嵌 `chara` / `ccv3`、独立 `lorebook_v3`、酒馆运行时
  World Info 导出（`uid` / `key` / `selectiveLogic` / 数字 `position`）由**合成 fixture** 覆盖，
  见 `crates/if-app/src/importer/png.rs` 与 `lorebook.rs` 的单元测试。
- 不要在测试里直连远程地址：样本以本地文件为准，更新时重新核对来源与许可。
- 原始内容不得改写；测试只读。

## 完整性与待补

- [ ] 真实 PNG 角色卡（含 `chara` / `ccv3` 文本块）——需要用户提供或另行收集，
      当前用合成 PNG 保证解析逻辑，但**不能等同于真实卡的兼容性证明**。
- [ ] 含 V3 扩展字段（`assets` / `group_only_greetings` / 装饰器）的真实卡。
- [ ] 独立 `lorebook_v3` 导出的真实世界书。
