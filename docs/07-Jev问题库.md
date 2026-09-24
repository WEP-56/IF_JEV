# 07 Jev 问题库

> 本文的接口部分已按实测结果修订；实测原始记录与依据见 [14 Jev 实测](14-Jev实测.md)。

## 1. Jev 接口摘要

**端点**（实测）：`POST https://openrouter.ai/api/alpha/decisions`。Jev 是 decisions 模型，**不能用 chat/completions 调用**（会返回 400）。它也不出现在 `/api/v1/models` 列表里，探活只能直接打这个端点。

- 请求包含三部分：`model`、`state`（字符串、JSON 对象或数组，内容须为文本）、`questions`（`id → 问题` 的 map，**至少一个问题**）。
- 三种原语，用 `type` 字段判别，取值只有这三个：
  - **Noul**：`{"type":"noul","instructions":"…"}`，可选 `true_means` / `false_means`。返回命题为真的概率（0–1）。**实测 `true_means`/`false_means` 对输出几乎无影响（≤0.01），措辞才是有效杠杆。**
  - **Choice**：`{"type":"choice","instructions":"…","criteria":{…}}`。`criteria` 是 **record（对象）** `{"选项ID":"选项说明"}`，选项 ID 就是返回分布的键。
  - **Score**：`{"type":"score","instructions":"…","criteria":[… ]}`。`criteria` 是 **有序 array**，最多 10 级。
- **注意两种原语的 `criteria` 形态相反**，写错会直接 400。
- 响应：`{ "model", "answers", "usage", "id", "provider" }`；`answers.<问题ID>` 按原语给出 `noul` / `choice + probabilities + confidence` / `score + legend + probabilities + confidence`。
- 同一请求中的所有问题看到同一个 state，并行、独立地评估，结果按问题 ID 对齐。**实测问题数量对延迟影响很小**：1 问 645 ms → 120 问 1 052 ms，120/120 全部正确返回。
- 预算：每个请求总计 64k token；state 加上最长的单个问题不超过 32k token。实测 27.4k 字符的 state 正常工作。
- 计费：只计输入 token，state 在一个请求里只计一次；**输出免费**（实测输出 token 未计入 `cost`）。`usage.cost` 直接给出本次成本，不必自行按 token 估算。
- 固定版本：请求写别名 `typesafe/jev-1.13`，**响应回显的是固定快照**（如 `typesafe/jev-1.13-20260917`），判定记录要存这个回显值。不要用会漂移的别名当版本号。
- 错误：不合规的请求返回 400，带 zod 风格的 `path`（如 `questions.<id>.criteria`），可对齐到问题模板 ID。**400 属于"请求不合规"，不该进入重试流程。**
- 官方建议：一个问题只判断一件具体、聚焦的事；多因素的决策拆成多个问题，再由代码组合。

## 2. 请求打包规则

| 编号 | 规则 |
|---|---|
| R1 | **一个视图一个请求。** 同一回合中需要同一视图的问题合并成一个请求；需要不同视图的分开发送，可以并行。 |
| R2 | **一个问题只判断一件事。** |
| R3 | **state 只放上下文，候选放在问题里。** 否则判断候选 A 的问题会看到候选 B，互相干扰。 |
| R4 | **控制预算。** state 目标不超过 24k token，单个问题不超过 2k，总量不超过 60k【初始值】；超出时由引擎拆分请求。 |
| R5 | **角色视图中不得出现该角色不知道的信息。** 连"他不知道的事"这种清单也不能放。 |
| R6 | **state 确定性序列化。** 键的顺序固定，便于复现和缓存。 |

示例（字段名已按实测校正）：

```json
{
  "model": "typesafe/jev-1.13",
  "state": {
    "视角人物": "顾言",
    "设定": "……",
    "目标": ["今晚乘火车离开"],
    "已知的事": ["林夏最近总是避开他", "……"],
    "当前场景": "……"
  },
  "questions": {
    "cand_004.notices": { "type": "noul", "instructions": "顾言是否会注意到：林夏在他靠近时刻意换了座位？……" },
    "cand_006.occurs":  { "type": "noul", "instructions": "在当前情境下，顾言会推迟今晚的行程吗？……" },
    "cand_007.action": {
      "type": "choice",
      "instructions": "顾言接下来最可能做什么？",
      "criteria": { "leave": "按原计划离开", "stay": "留下", "delay": "推迟行程", "ask": "先问清楚" }
    }
  }
}
```

返回（实测形态）：

```json
{
  "model": "typesafe/jev-1.13-20260917",
  "answers": {
    "cand_004.notices": { "type": "noul", "noul": 0.58 },
    "cand_006.occurs":  { "type": "noul", "noul": 0.41 },
    "cand_007.action":  {
      "type": "choice",
      "choice": "delay",
      "probabilities": { "leave": 0.42, "delay": 0.44, "ask": 0.14, "stay": 0.0 },
      "confidence": 0.63
    }
  },
  "usage": { "input_tokens": 1168, "output_tokens": 544, "cost": 0.00004906 },
  "id": "gen-dec-…",
  "provider": "TypeSafe"
}
```

## 3. 问题模板规范

```yaml
id: q.behavior.occurs          # 稳定的 ID
version: 1                     # 措辞或标准一有改动就升版本
primitive: noul                # noul | choice | score
class: O                       # C | O | A | M | D | V（见 06 §1）
view: pov                      # parse | god | director | pov | narration | check | player
instructions: "在当前情境下，{subject}会{candidate}吗？只根据{subject}已知的信息、目标和性格判断。"
true_means: "会这样做"          # Noul 可选；实测对输出几乎无影响，不要当作语义开关
false_means: "不会这样做"
criteria: null                 # 仅 choice / score 使用，形态见下
policy: seeded_sample          # threshold | seeded_sample | numeric | director_term
threshold: null
```

`criteria` 的形态由 `primitive` 决定，**两者相反，容易写错**：

| primitive | criteria 形态 | 示例 |
|---|---|---|
| `choice` | **record（对象）**，键即返回分布的键 | `{ "leave": "按原计划离开", "stay": "留下" }` |
| `score` | **有序 array（最多 10 级）**，索引 0 基 | `["明显缓和", "略微缓和", "持平", "略微升高", "明显升高"]` |

- 模板中的 `{…}` 由引擎填充。
- 措辞就是行为：只要改了措辞就必须升版本，事件日志记录的是"模板 ID@版本"。**实测支持这条**：同一情境下"…会离开吗"得 0.85、"…符合他的性格吗"得 0.65、"…发生的可能性有多大"得 0.92，三种措辞不可互换。
- 发生类问题必须用 Noul 的 `true_means`/`false_means` 写成"会发生 / 不会发生"，不能写成"是否合理""是否符合人物"（06 §1）。上一条的实测数据就是这条规则的依据。

## 4. 核心问题目录（v0.1）

### 4.1 输入与解析

| ID | 原语 | 类 | 视图 | 措辞草案 |
|---|---|---|---|---|
| `q.input.is_directive` | Noul | C | parse | 这段输入是否包含导演式指令：要求某个角色在未来采取特定行动、要求推进或跳过时间、指定故事接下来聚焦什么，或指定某个冲突的结局？只陈述世界现在或过去状态的输入不算。true = 包含指令 |
| `q.input.rewrite_preserves` | Noul | C | parse | 改写后的 IF「{rewrite}」是否保留了原输入想改变世界的意图，同时不再包含导演式指令？ |
| `q.if.parse_faithful` | Noul | C | parse | 解析结果是否忠实表达了原文：既没有遗漏原文断言的内容，也没有加入原文没有断言的内容（例如擅自假定角色已经意识到、会采取行动，或者他人已经知道）？ |
| `q.if.type` | Choice | C | parse | 这条 IF 属于哪一类？选项：状态型 / 认知型 / 规则型 / 事件型 / 真相型 / 回溯型（每项附一句定义，见 [01 §5](01-IF规则.md)） |
| `q.if.contradicts_displayed` | Noul | C | player | 这条断言是否与读者已经读到的内容相矛盾？ |
| `q.if.compatible_with` | Noul | C | god | 以下两条陈述能否在同一个世界中同时成立？A：{core}　B：{fact}。true = 能同时成立 |
| `q.if.reconcile_plausible` | Noul | C | god | 为了让 A 和 B 同时成立，提出了这样的解释：{reconcile}。这个解释是否自然、不牵强，并且确实能让两者同时成立？ |
| `q.if.prior_plausibility` | Noul | M | god | 在当前这个世界里，没有任何外力干预的情况下，「{core}」自然成立的可能性有多大？true = 会自然成立 |

### 4.2 候选

| ID | 原语 | 类 | 视图 | 措辞草案 |
|---|---|---|---|---|
| `q.cand.relevance` | Score | M | god | 事件「{event}」会对{subject}产生多大影响？等级：无关 / 间接 / 明显 / 直接 |
| `q.cand.in_character` | Noul | C | pov | 以{subject}的性格、目标和他目前知道的信息来看，「{candidate}」是否在人物的合理范围之内？ |
| `q.cand.knowledge_gap` | Noul | C | pov | 「{candidate}」这个行动或想法，是否需要用到{subject}目前并不知道的信息？（state 中列出的就是他知道的全部）true = 需要 |
| `q.behavior.occurs` | Noul | O | pov | 在当前情境下，{subject}会{candidate}吗？只根据{subject}已知的信息、目标和性格判断。true = 会 |
| `q.perception.notices` | Noul | O | pov | {observer}是否会注意到：{cue}？只考虑{observer}当时的注意力、处境，以及对{target}的熟悉程度。true = 会注意到 |
| `q.world.occurs` | Noul | O | god | 在当前的世界状态下，「{candidate}」会发生吗？true = 会发生 |
| `q.outcome.choice` | Choice | A | pov 或 god | {situation}，接下来会是哪种情况？选项：{options}（按选项 ID 排序） |
| `q.cand.coherent` | Noul | C | god | 以下变化同时成立时，是否自相矛盾？{accepted}。true = 矛盾【可选】 |

### 4.3 导演

| ID | 原语 | 类 | 视图 | 措辞草案 |
|---|---|---|---|---|
| `q.scene.fit` | Noul | D | director | 作为接下来的一个场景，「{scene}」是否能自然承接当前的局面？ |
| `q.scene.tension` | Score | D | director | 如果接下来发生「{scene}」，故事的紧张程度会如何变化？等级：明显缓和 / 略微缓和 / 持平 / 略微升高 / 明显升高 |
| `q.scene.advances_thread` | Noul | D | director | 「{scene}」是否会实质推进故事线「{thread}」，也就是让它的核心问题更接近答案，或者让代价更高？ |
| `q.scene.repetitive` | Noul | D | director | 与最近几个场景（{recent}）相比，「{scene}」是否在重复同一种冲突模式或情绪？ |
| `q.scene.resolves_thread` | Noul | C | director | 「{scene}」是否会让故事线「{thread}」的核心问题得到最终的回答？ |

### 4.4 节拍（场景计划检查也复用这些模板，检查对象换成计划文本）

| ID | 原语 | 类 | 视图 | 措辞草案 |
|---|---|---|---|---|
| `q.beat.violates_fact` | Noul | C | check | 这段正文是否与以下事实相矛盾？事实：{fact} |
| `q.beat.violates_rule` | Noul | C | check | 这段正文是否违反了世界规则「{rule}」？边界解释：{boundaries} |
| `q.beat.knowledge_leak` | Noul | C | check | 在这段正文里，{character}的言行是否表现出了他不可能知道的信息？他知道的信息：{knowledge} |
| `q.beat.forbidden_resolution` | Noul | C | check | 这段正文是否出现了被禁止的结果：{forbidden}？ |
| `q.beat.reveals_secret` | Noul | C | check | 这段正文是否向读者透露或明显暗示了：{secret}？ |
| `q.beat.stop_reached` | Noul | C | check | 到这一节拍为止，场景的停止条件「{stop}」是否已经达成？（附此前各节拍） |
| `q.beat.goal_done` | Noul | C | check | 这段正文是否完成了节拍目标「{beat_goal}」？ |

### 4.5 回收与认知

| ID | 原语 | 类 | 视图 | 措辞草案 |
|---|---|---|---|---|
| `q.extract.faithful` | Noul | V | check | 这段正文是否确立了以下内容：{item}？只看正文写了什么或明确暗示了什么，不做延伸推测 |
| `q.extract.kind` | Choice | V | check | 这条内容属于哪一类？选项：客观事实 / 某人的声称 / 某人的想法或信念 / 某人的意图或计划 / 仅是氛围描写 |
| `q.extract.load_bearing` | Noul | V | check | 这个细节将来是否可能影响剧情，例如成为线索、承诺、伤势、道具或关系变化？ |
| `q.extract.missing` | Noul | V | check | 除了已经列出的内容，这段正文是否还确立了其他可能影响剧情的新事实？已列出：{items} |
| `q.key.equivalent` | Noul | C | god | 「{new}」和「{existing}」说的是同一件事吗？ |
| `q.claim.credibility` | Score | M | pov（听者） | {listener}听到{speaker}说「{claim}」之后，会在多大程度上相信？等级见 [06 §7](06-裁决策略.md) |
| `q.claim.misleading` | Noul | C | god | {speaker}说「{claim}」时，是否意在让对方相信一件不真实的事（字面上可以为真）？ |

### 4.6 结构、后台、观测、创建

| ID | 原语 | 类 | 视图 | 措辞草案 |
|---|---|---|---|---|
| `q.thread.stage` | Choice | C | god | 故事线「{thread}」现在处于哪个阶段？选项：埋下 / 发展 / 升级 / 高潮 / 已解决 / 已搁置（每项附定义） |
| `q.tendency.push` | Score | M | god | 本回合发生的事，对趋势「{tendency}」的推动程度如何？等级：削弱 / 无影响 / 轻微推动 / 明显推动 / 强烈推动 |
| `q.window.fate_node` | Noul | C | director | 此刻是否处在关键转折点，外部的干预会显著改变故事走向？（仅用于界面提示） |
| `q.bg.occurs` | Noul | O | god | 在过去的{elapsed}里，「{candidate}」发生了吗？ |
| `q.observe.leaks_secret` | Noul | C | check | 以下观测结果是否直接透露了这些秘密：{secrets}？ |
| `q.world.consistent` | Noul | C | god | 以下世界设定之间是否存在矛盾？ |
| `q.world.goal_conflict` | Noul | C | god | 这些主要角色的目标之间是否存在实质的冲突？ |

## 5. 判定记录

```json
{
  "judgment_id": "jdg_01923",
  "turn": "turn_0088",
  "template": "q.perception.notices@1",
  "target": "cand_004",
  "view": { "kind": "pov", "holder": "c_gu", "hash": "b3:…" },
  "model": "typesafe/jev-1.13-20260917",
  "primitive": "noul",
  "output": { "noul": 0.58 },
  "usage": { "input_tokens": 3216, "cost": 0.00013507 },
  "latency_ms": 140
}
```

- `model` 记响应回显的**固定快照版本**，不是请求用的别名。
- `output` 按原语：Noul 为 `{ "noul": p }`；Choice 为 `{ "choice", "probabilities", "confidence" }`；Score 为 `{ "score", "legend", "probabilities", "confidence" }`。
- `usage.cost` 直接取自响应，不要自己按 token 估算。
- 本文档早期版本用的 `output.probability` 与 `jev-1.13.0` 均与实际不符，已按实测校正。

## 6. 版本、校准与回归

- **模型版本**：请求写别名 `typesafe/jev-1.13`，记录响应回显的快照版本；升级前必须跑回归。
- **模板版本**：措辞或标准一有改动就升版本。
- **标注集**：每个约束类、校验类模板准备 40 条左右的人工标注样本【初始值】，用来挑选阈值。违反类问题更看重召回：宁可多拦，不可漏过。优先校准 `q.beat.violates_fact`：实测违规 0.88 / 合规 0.21，阈值 0.3 的合规侧只剩 0.09 余量，最可能误杀。
- **回归**：升级 Jev 版本或修改模板之前，先用标注集比较判定结果；差异超出容忍范围就暂缓升级。工具入口：`python tools/jev_probe.py --file <batch>.json --repeat N`。
- **稳定性测试**：同一个问题换几种说法，看概率是否稳定；打乱 Choice 的选项顺序，看是否存在位置偏差。
- **已知基线**（实测，可作为回归参照）：跨请求重复 5 次极差 0.01；同批内重复 5 次极差 0.02；Choice 选项顺序打乱三次，选中项不变、概率波动 ≤0.04；20 并发无速率限制问题。
