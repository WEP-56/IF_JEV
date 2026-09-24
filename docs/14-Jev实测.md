# 14 Jev 实测

> v0.1 · 2026-09-24 · **实测记录，非设计文档**
> 被测后端：`typesafe/jev-1.13`（实测解析为 `typesafe/jev-1.13-20260917`），经 OpenRouter
> 与 [07 Jev 问题库](07-Jev问题库.md) 冲突之处，以本文为准。

本文回答 07 §1 里"整理自 `拍板.md`，以官方文档为准"那些未经核实的接口描述。所有结论来自真实请求，探针脚本见 `tools/jev_probe.py`。

## 1. 调用方式（07 缺失的部分）

**端点不是 chat/completions。** 用 OpenAI 兼容格式调用会被明确拒绝：

```
POST /api/v1/chat/completions
→ 400 {"error":{"message":"typesafe/jev-1.13 is a decisions model and cannot be used
   with the chat/completions endpoint. Use the /api/alpha/decisions endpoint instead."}}
```

正确调用：

| 项 | 值 |
|---|---|
| 端点 | `POST https://openrouter.ai/api/alpha/decisions` |
| 认证 | `Authorization: Bearer <OpenRouter key>` |
| 请求头 | `Content-Type: application/json` |
| 请求体 | `{ "model", "state", "questions" }` |
| 响应体 | `{ "model", "answers", "usage", "id", "provider" }` |

两点工程提醒：

- **该模型不出现在 `/api/v1/models` 列表里**（实测 458 个模型中无 `jev`），直查 `/api/v1/models/typesafe/jev-1.13` 也返回 404。因此**不能用模型列表做可用性检查**，只能直接探测 decisions 端点。
- 响应中的 `model` 是**解析后的固定快照** `typesafe/jev-1.13-20260917`。07 §1 "固定版本号、不用 latest"的结论成立，但真实版本 ID 是这种带日期的形态，不是 `jev-1.13.0`。

## 2. 请求 schema（实测）

`questions` 是一个 `id → 问题` 的 map，问题用 `type` 做判别式，取值只有三种：`noul` / `choice` / `score`。

```json
{
  "model": "typesafe/jev-1.13",
  "state": "蒸汽时代王国。林夏想让顾言留下。顾言今晚将乘火车离开。",
  "questions": {
    "q_noul":   { "type": "noul",   "instructions": "顾言今晚会离开吗？",
                  "true_means": "会离开", "false_means": "不会离开" },
    "q_choice": { "type": "choice", "instructions": "接下来林夏最可能做什么？",
                  "criteria": { "go": "直接去找顾言", "nothing": "什么都不做" } },
    "q_score":  { "type": "score",  "instructions": "紧张程度会如何变化？",
                  "criteria": ["明显缓和", "略微缓和", "持平", "略微升高", "明显升高"] }
  }
}
```

**最容易踩的坑：`criteria` 的形态在两种原语下是相反的。**

| 原语 | criteria 形态 | 说明 |
|---|---|---|
| `choice` | **record（对象）** `{"key": "标签"}` | key 就是返回的概率分布的键 |
| `score` | **array（有序数组）** `["等级0", "等级1", …]` | 顺序即量表顺序，索引 0 基 |

写成另一种会直接 400（`expected record, received array` / 反向同理）。07 §3 的模板规范把两者统一写成数组 `criteria: []`，需要拆成两个字段。

`state` 三种形态均可：字符串、JSON 对象、数组（均已实测 200）。内容须为文本。

## 3. 响应 schema（实测）

三种原语的返回结构：

```json
{
  "model": "typesafe/jev-1.13-20260917",
  "answers": {
    "q_noul":   { "type": "noul", "noul": 0.62 },
    "q_choice": { "type": "choice", "choice": "go",
                  "probabilities": { "go": 0.97, "nothing": 0, "…": 0 },
                  "confidence": 0.96 },
    "q_score":  { "type": "score", "score": 3.73,
                  "legend": { "0": "明显缓和", "…": "…", "4": "明显升高" },
                  "probabilities": { "0": 0, "1": 0.01, "2": 0.02, "3": 0.2, "4": 0.77 },
                  "confidence": 0.78 }
  },
  "usage": { "input_tokens": 394, "output_tokens": 57, "cost": 0.000016548 },
  "id": "gen-dec-1790258900-J5hdOTz8JOaNXwAczMri",
  "provider": "TypeSafe"
}
```

要点：

- **Noul 的概率在 `answers.<id>.noul`**，不是 07 §5 写的 `output.probability`。
- Choice 返回选中键、完整分布、置信度 —— 与 07 §1 描述一致。
- Score 返回**连续分数**（0 基，本例 5 级时区间为 0–4）、`legend`（索引→标签）、各级概率、置信度。06 §6 的"五级映射 0/0.25/0.5/0.75/1"需要在引擎侧按 `score / (n − 1)` 归一化，别硬编码 5 级。
- **`usage` 直接给出 `cost`**，不必自己按 token 估算。07 §5 的判定记录应加一列成本。

校验错误一览（都是 400，带 zod 风格的 `path`，可直接对齐到问题模板 ID）：

| 情形 | 信息 |
|---|---|
| `choice` 缺 / 错写 criteria | `expected record, received undefined/array`，path `questions.<id>.criteria` |
| `score` 缺 / 错写 criteria | `expected array, received undefined/object` |
| `type` 非法 | `invalid_union`，`discriminator:"type"`，`options:["noul","choice","score"]` |
| 缺 `instructions` | `invalid_type, expected string` |
| `questions` 为空 | `At least one question is required` |
| 模型名不存在 | `Model typesafe/jev-9.99 does not exist` |
| **额外未知字段** | **200，静默忽略**（非严格模式） |

## 4. 与 07 的差异清单（需修订文档）

| # | 位置 | 文档写法 | 实测 |
|---|---|---|---|
| 1 | 07 §1 | 未记录端点 | `POST /api/alpha/decisions` |
| 2 | 07 §3 | `criteria: []` 统一为数组 | `choice` 用 record，`score` 用 array |
| 3 | 07 §5 | `output: { probability: 0.58 }` | `answers.<id>.noul`；choice/score 另有 `choice`/`score`/`probabilities`/`confidence` |
| 4 | 07 §5 | 只记 token | `usage.cost` 可直接使用 |
| 5 | 07 §1 | 固定 `jev-1.13.0` | 别名 `typesafe/jev-1.13` → 快照 `typesafe/jev-1.13-20260917` |
| 6 | 06 §6 / §7 | Score 五级映射写死 | 需按 `n − 1` 归一化，并读取 `legend` 对齐标签 |
| 7 | 07 §3 | `true_means`/`false_means` 可选 | 实测对输出几乎无影响（见 §6.3） |

## 5. 工程参数

### 5.1 批量打包：延迟几乎不随问题数增长

| 同批问题数 | 延迟 | input tokens | cost |
|---|---|---|---|
| 1 | 645 ms | 330 | $0.0000139 |
| 5 | 946 ms | 488 | $0.0000205 |
| 20 | 743 ms | 1 168 | $0.0000491 |
| 60 | 682 ms | 3 026 | $0.000127 |
| **120** | **1 052 ms** | 5 746 | $0.000241 |

- 120 个问题全部按 ID 正确对齐返回，**未触到数量上限**。
- 1 问 → 120 问，延迟只从 645 ms 涨到 1 052 ms。**07 §2 的 R1（一个视图一个请求）实测成立且廉价**，可以放心把同一视图的所有检查打包。
- state 在一个请求里只计一次（1 问 330 → 120 问 5 746，非线性增长），与 07 §1 一致。

### 5.2 大 state

27 435 字符 + 20 问 → input 28 244 tokens，987 ms，$0.00119，20/20 正常。06 §2 / 08 §6 的"state ≤ 24k token"目标可行。

### 5.3 稳定性与偏差

| 测试 | 结果 |
|---|---|
| 同批内同一问题重复 5 次 | 0.57 / 0.57 / 0.57 / 0.57 / 0.55，极差 0.02 |
| 跨请求重复 5 次 | 0.57 / 0.57 / 0.57 / 0.58 / 0.57，极差 0.01 |
| Choice 选项顺序打乱 3 次 | 选中项不变，概率波动 ≤ 0.04（0.96 / 0.97 / 0.93） |

**判定几乎是确定性的**，这对阈值策略是好消息：p 的小数位不是噪声，可以用。

### 5.4 成本口径

- 330 input → `$0.00001386` = 330 × 0.042 / 1e6，**输出 token 未计费**。`拍板.md` 的"输入计价、输出免费"确认。
- 一个 IF 回合按 100 个判定估 ≈ **$0.0002**。5 美元约合 **2.5 万回合**。**成本不构成约束，不必为省钱拆请求。**

### 5.5 并发

20 并发全部 200，wall 1.21 s，单请求 p50 685 ms / max 1 208 ms。未触到文档所述的速率限制。

## 6. 三项影响设计的实证结论

### 6.1 视角隔离（P9）确实是必要的，但 Jev 本身不是全知盲

对照实验：同一组关于**林夏**的问题，state 里加/不加一句林夏不知道的秘密（"顾言其实已决定留下"）。

| 问题 | 含秘密 | 无秘密 | 差 |
|---|---|---|---|
| 林夏今晚会不会去火车站挽留顾言 | 0.53 | 0.61 | **−0.08** |
| 林夏此刻是否已知道顾言准备留下 | 0.10 | 0.10 | 0.00 |
| 林夏今晚心情会比较平静吗 | 0.15 | 0.13 | +0.02 |

- 去掉"只根据林夏知道的信息判断"这句护栏后，差值同样复现（0.53 / 0.61），`realize` 由 0.00 变为 +0.04。
- **结论一：泄漏客观存在**（行为类判定被污染约 0.08），且**提示词护栏不足以消除** → 视图隔离必须由引擎强制，不能指望在指令里写一句免责。P9 从"设计洁癖"升级为"可测量的正确性要求"。
- **结论二：Jev 有一定视角意识**。把秘密放进 state 并不会让它直接认定"角色知道"（`realize` 稳定在 0.10）。所以隔离的目标是**消除 0.08 量级的偏移**，而不是防止灾难性全知。

### 6.2 06 §1 的两条硬规则成立

同一情境，三种措辞：

| 措辞 | p |
|---|---|
| "顾言今晚**会离开**吗？"（发生类，符合硬规则） | 0.85 |
| "顾言今晚离开这件事，**符合他的性格与当前处境**吗？" | **0.65** |
| "……**发生的可能性有多大**？" | 0.92 |

同一情境下 0.65 / 0.85 / 0.92。**混用会系统性偏离**：拿"符合人物"的 0.65 去掷骰，会显著低估事件发生率。06 §1 "发生类必须用 Noul 的 true/false 写成'会发生 / 不会发生'"是对的，应保留为强制规则。

（注意第三条 0.92 与第一条 0.85 也不等，说明连"可能性有多大"这种看似等价的问法都不可互换 —— 措辞必须版本化。）

### 6.3 措辞是唯一有效杠杆；`true_means`/`false_means` 近乎无效

同一问题，加与不加 `true_means`/`false_means`，三轮实测：

| 轮次 | 无定义 | 有定义 | 差 |
|---|---|---|---|
| 1 | 0.87 | 0.87 | 0.00 |
| 2 | 0.87 | 0.87 | 0.00 |
| 3 | 0.87 | 0.86 | −0.01 |

对比 §6.2：改 `instructions` 措辞能把同一情境从 0.65 拉到 0.92，而 `true_means` 的效应 ≤ 0.01。

- **结论：07 §6"措辞就是行为，改措辞必须升版本"是本项目最重要的一条工程纪律。**
- `true_means`/`false_means` 可以继续写（无害、可读性更好），但**不要把它当作语义开关依赖**，更不要因为加了它就不改措辞。

## 7. 对 `if-judge` 实现的建议

1. **用类型系统挡住 criteria 的形态差异**：Rust 侧给 `choice` 和 `score` 两个不同的 criteria 类型，不要用一个 enum 字段装两种形态。
2. **本地预校验**：先校验 type 枚举、criteria 形态、至少 1 问，避免 400 白跑往返。错误信息里的 `path` 可以对齐到模板 ID 后回填判定记录。
3. **打包策略放心激进**：同一视图的所有问题打包成一到两个请求（§5.1）。
4. **预算优先多打包、少拆请求**：state 只计一次，拆请求等于重复付 state 的钱。
5. **版本记录用响应回显值**：请求写别名 `typesafe/jev-1.13`，判定记录里存 `typesafe/jev-1.13-20260917`。
6. **成本直接用 `usage.cost`**。
7. **区分失败类型**：400 = 请求不合规（代码 bug，不该重试）；5xx / 超时才走 06 §9 的退避重试。当前 `if-judge` 的错误分类需要按这个边界设计。
8. **鉴权探测**：不要依赖 `/api/v1/models`，直接对 decisions 端点发一个最小 noul 探活。

## 8. 待补验证（本次未覆盖）

| 项 | 说明 |
|---|---|
| 阈值标注集 | 07 §6 要求的每模板约 40 条人工标注，是阈值能否落地的唯一依据 |
| `q.beat.violates_fact` 阈值余量 | 实测违规 0.88 / 合规 0.21，阈值 0.3 方向正确，但合规侧只有 0.09 余量，**最可能误杀**，优先校准 |
| 速率限制 | 文档称 250 k tok/s、1 200 req/min，本次 20 并发未触边界 |
| Choice 位置偏差 | 仅 3 次采样，需要更大样本 |
| 长 instructions | 接近 2 k token 的问题表现 |
| 隐式会话 | decisions 接口表面无状态，需确认无跨请求记忆 |

## 9. 密钥

探针从仓库根 `jevkey`（已 gitignore）读取，或读环境变量 `JEV_KEY`，不回显、不落盘。

> 注：本次使用的仍是 `jevkey` 里的 key（`usage` 显示此前未被使用过）。仓库中不存在 `jevkey.txt`。若之后换用另一个临时 key，请放到 `jevkey` 或设置 `JEV_KEY`。
