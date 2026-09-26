//! 问题构造：把领域对象翻成 [`if_judge::Question`]（docs/07 §3–§4）。
//!
//! **措辞就是行为。** docs/07 §3 记录了三组实测：同一个情境下「…会离开吗」得 0.85、
//! 「…符合他的性格吗」得 0.65、「…发生的可能性有多大」得 0.92。所以模板文本是
//! 契约的一部分，不是可以随手改的提示词——改了就升版本。
//!
//! 这里只做两件事：**填模板**，和**给出稳定的键**。原语、类、视图、策略的选择
//! 也一并写死在这里，因为那是模板的属性；调用方（`candidates` / `scenes` / `beats`）
//! 只负责决定「问哪些、问几个」。
//!
//! 两条 docs/06 §1 的硬规则在构造层就落地：
//!
//! - 发生类一律用 `Noul` 且措辞是「会 / 不会」，禁止写成「是否合理」「是否符合人物」；
//! - 约束类一律走 `Noul`/`Choice` 供阈值裁决，调用方不会拿到骰子。
//!
//! 问题键（[`Question::key`]）在同一请求内必须唯一，它们同时是响应 `answers` 的键。
//! 形如 `cand_004.occurs`：`target` 部分让人一眼看出问的是谁。

use if_domain::id::LoreId;
use if_judge::{ChoiceCriteria, Question, QuestionSpec, ScoreCriteria};

use crate::TEMPLATE_VERSION;

// ---------------------------------------------------------------- 模板目录

pub const Q_BEHAVIOR_OCCURS: &str = "q.behavior.occurs@1";
pub const Q_WORLD_OCCURS: &str = "q.world.occurs@1";
pub const Q_PERCEPTION_NOTICES: &str = "q.perception.notices@1";
pub const Q_OUTCOME_CHOICE: &str = "q.outcome.choice@1";
pub const Q_CAND_IN_CHARACTER: &str = "q.cand.in_character@1";
pub const Q_CAND_KNOWLEDGE_GAP: &str = "q.cand.knowledge_gap@1";

pub const Q_SCENE_FIT: &str = "q.scene.fit@1";
pub const Q_SCENE_TENSION: &str = "q.scene.tension@1";
pub const Q_SCENE_ADVANCES_THREAD: &str = "q.scene.advances_thread@1";
pub const Q_SCENE_REPETITIVE: &str = "q.scene.repetitive@1";
pub const Q_SCENE_RESOLVES_THREAD: &str = "q.scene.resolves_thread@1";

pub const Q_BEAT_VIOLATES_FACT: &str = "q.beat.violates_fact@1";
pub const Q_BEAT_VIOLATES_RULE: &str = "q.beat.violates_rule@1";
pub const Q_BEAT_KNOWLEDGE_LEAK: &str = "q.beat.knowledge_leak@1";
pub const Q_BEAT_FORBIDDEN_RESOLUTION: &str = "q.beat.forbidden_resolution@1";
pub const Q_BEAT_REVEALS_SECRET: &str = "q.beat.reveals_secret@1";
pub const Q_BEAT_STOP_REACHED: &str = "q.beat.stop_reached@1";
pub const Q_BEAT_GOAL_DONE: &str = "q.beat.goal_done@1";

pub const Q_TENDENCY_PUSH: &str = "q.tendency.push@1";
pub const Q_THREAD_STAGE: &str = "q.thread.stage@1";

/// 全部模板 ID，供文档与回归工具遍历。顺序与 docs/07 §4 一致。
pub const TEMPLATES: &[&str] = &[
    Q_BEHAVIOR_OCCURS,
    Q_WORLD_OCCURS,
    Q_PERCEPTION_NOTICES,
    Q_OUTCOME_CHOICE,
    Q_CAND_IN_CHARACTER,
    Q_CAND_KNOWLEDGE_GAP,
    Q_SCENE_FIT,
    Q_SCENE_TENSION,
    Q_SCENE_ADVANCES_THREAD,
    Q_SCENE_REPETITIVE,
    Q_SCENE_RESOLVES_THREAD,
    Q_BEAT_VIOLATES_FACT,
    Q_BEAT_VIOLATES_RULE,
    Q_BEAT_KNOWLEDGE_LEAK,
    Q_BEAT_FORBIDDEN_RESOLUTION,
    Q_BEAT_REVEALS_SECRET,
    Q_BEAT_STOP_REACHED,
    Q_BEAT_GOAL_DONE,
    Q_TENDENCY_PUSH,
    Q_THREAD_STAGE,
];

/// `q.behavior.occurs` → `q.behavior.occurs@1`。已经是带版本的写法就原样返回。
pub fn template_id(base: &str) -> String {
    if base.contains('@') {
        base.to_owned()
    } else {
        format!("{base}@{TEMPLATE_VERSION}")
    }
}

/// `q.scene.tension` 的等级（docs/06 §7）：0 明显缓和 → 1 明显升高。
pub const TENSION_LEVELS: &[&str] = &["明显缓和", "略微缓和", "持平", "略微升高", "明显升高"];

/// `q.tendency.push` 的等级（docs/06 §5）。索引与 [`if_domain::tendency_push_delta`] 对齐。
pub const PUSH_LEVELS: &[&str] = &["削弱", "无影响", "轻微推动", "明显推动", "强烈推动"];

// ---------------------------------------------------------------- 候选

/// 行为候选：某主体会不会做某件事（发生类 O，角色视图）。docs/06 §1 硬规则一。
pub fn behavior_occurs(target: &str, subject: &str, candidate: &str) -> Question {
    Question {
        key: format!("{target}.occurs"),
        template: Q_BEHAVIOR_OCCURS.to_owned(),
        target: target.to_owned(),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "在当前情境下，{subject}会「{candidate}」吗？只根据{subject}已知的信息、目标和性格判断。\
                 true = 会这样做。"
            ),
            true_means: Some("会这样做".into()),
            false_means: Some("不会这样做".into()),
        },
    }
}

/// 世界候选：没有主体的事件（发生类 O，上帝视图）。
pub fn world_occurs(target: &str, candidate: &str) -> Question {
    Question {
        key: format!("{target}.occurs"),
        template: Q_WORLD_OCCURS.to_owned(),
        target: target.to_owned(),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "在当前的世界状态下，「{candidate}」会发生吗？不要因为「这样写更有意思」而调高概率，\
                 只判断它在这个世界里自然发生的可能性。true = 会发生。"
            ),
            true_means: Some("会发生".into()),
            false_means: Some("不会发生".into()),
        },
    }
}

/// 感知候选：某观察者会不会注意到某个线索（发生类 O，角色视图）。
pub fn perception_notices(target: &str, observer: &str, cue: &str, of: &str) -> Question {
    Question {
        key: format!("{target}.notices"),
        template: Q_PERCEPTION_NOTICES.to_owned(),
        target: target.to_owned(),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "{observer}是否会注意到：{cue}？只考虑{observer}当时的注意力、处境，\
                 以及对{of}的熟悉程度。true = 会注意到。"
            ),
            true_means: Some("会注意到".into()),
            false_means: Some("不会注意到".into()),
        },
    }
}

/// 互斥组：几种结果中哪一种发生（互斥类 A）。选项按 ID 排序后累加分布（docs/06 §3）。
pub fn outcome_choice(target: &str, situation: &str, options: &[(String, String)]) -> Question {
    Question {
        key: format!("{target}.choice"),
        template: Q_OUTCOME_CHOICE.to_owned(),
        target: target.to_owned(),
        spec: QuestionSpec::Choice {
            instructions: format!("{situation}，接下来会是哪种情况？"),
            criteria: ChoiceCriteria(options.iter().cloned().collect()),
        },
    }
}

/// 约束门一：候选是否符合人物（约束类 C，角色视图，概率越高越可靠）。
pub fn cand_in_character(target: &str, subject: &str, candidate: &str) -> Question {
    Question {
        key: format!("{target}.in_character"),
        template: Q_CAND_IN_CHARACTER.to_owned(),
        target: target.to_owned(),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "以{subject}的性格、目标和他目前知道的信息来看，「{candidate}」是否在人物的合理范围之内？\
                 true = 在合理范围之内。"
            ),
            true_means: Some("在合理范围之内".into()),
            false_means: Some("超出合理范围".into()),
        },
    }
}

/// 约束门二：候选是否需要用到该角色不知道的信息（约束类 C，概率越高越可疑）。
///
/// state 里列出的就是他知道的全部（docs/07 R5），所以问题本身不能再给出任何
/// 「他不知道什么」的清单。
pub fn cand_knowledge_gap(target: &str, subject: &str, candidate: &str) -> Question {
    Question {
        key: format!("{target}.knowledge_gap"),
        template: Q_CAND_KNOWLEDGE_GAP.to_owned(),
        target: target.to_owned(),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "「{candidate}」这个行动或想法，是否需要用到{subject}目前并不知道的信息？\
                 state 中列出的就是他知道的全部。true = 需要用到。"
            ),
            true_means: Some("需要用到他不知道的信息".into()),
            false_means: Some("不需要".into()),
        },
    }
}

// ---------------------------------------------------------------- 场景

pub fn scene_fit(index: usize, summary: &str) -> Question {
    Question {
        key: format!("scene_{index}.fit"),
        template: Q_SCENE_FIT.to_owned(),
        target: format!("scene_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!("作为接下来的一个场景，「{summary}」是否能自然承接当前的局面？true = 能自然承接。"),
            true_means: Some("能自然承接".into()),
            false_means: Some("接不上".into()),
        },
    }
}

pub fn scene_tension(index: usize, summary: &str) -> Question {
    Question {
        key: format!("scene_{index}.tension"),
        template: Q_SCENE_TENSION.to_owned(),
        target: format!("scene_{index}"),
        spec: QuestionSpec::Score {
            instructions: format!("如果接下来发生「{summary}」，故事的紧张程度会如何变化？"),
            criteria: ScoreCriteria(TENSION_LEVELS.iter().map(|s| (*s).to_owned()).collect()),
        },
    }
}

pub fn scene_advances_thread(index: usize, summary: &str, title: &str, question: &str) -> Question {
    Question {
        key: format!("scene_{index}.advances.{title}"),
        template: Q_SCENE_ADVANCES_THREAD.to_owned(),
        target: format!("scene_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "「{summary}」是否会实质推进故事线「{title}」（{question}），\
                 也就是让它的核心问题更接近答案，或者让代价更高？true = 会推进。"
            ),
            true_means: Some("会推进".into()),
            false_means: Some("不会推进".into()),
        },
    }
}

pub fn scene_repetitive(index: usize, summary: &str, recent: &str) -> Question {
    Question {
        key: format!("scene_{index}.repetitive"),
        template: Q_SCENE_REPETITIVE.to_owned(),
        target: format!("scene_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "与最近几个场景（{recent}）相比，「{summary}」是否在重复同一种冲突模式或情绪？\
                 true = 在重复。"
            ),
            true_means: Some("在重复".into()),
            false_means: Some("不重复".into()),
        },
    }
}

/// 受保护的故事线走引擎硬否决（docs/04 §2.1），这个模板只用在**未受保护**的线上，
/// 作为导演评分里的「过早了结」惩罚项（docs/06 §6）。
pub fn scene_resolves_thread(index: usize, summary: &str, title: &str, question: &str) -> Question {
    Question {
        key: format!("scene_{index}.resolves.{title}"),
        template: Q_SCENE_RESOLVES_THREAD.to_owned(),
        target: format!("scene_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "「{summary}」是否会让故事线「{title}」（{question}）的核心问题得到最终的回答？\
                 true = 会让它得到最终回答。"
            ),
            true_means: Some("会让它得到最终回答".into()),
            false_means: Some("不会".into()),
        },
    }
}

// ---------------------------------------------------------------- 节拍

pub fn beat_violates_fact(index: u32, fact: &str) -> Question {
    Question {
        key: format!("beat_{index}.fact"),
        template: Q_BEAT_VIOLATES_FACT.to_owned(),
        target: format!("beat_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!("这段正文是否与以下事实相矛盾？事实：{fact}。true = 相矛盾。"),
            true_means: Some("相矛盾".into()),
            false_means: Some("不矛盾".into()),
        },
    }
}

pub fn beat_violates_rule(index: u32, rule: &str, boundaries: &str) -> Question {
    Question {
        key: format!("beat_{index}.rule"),
        template: Q_BEAT_VIOLATES_RULE.to_owned(),
        target: format!("beat_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "这段正文是否违反了世界规则「{rule}」？边界解释：{boundaries}。true = 违反了。"
            ),
            true_means: Some("违反了".into()),
            false_means: Some("没有违反".into()),
        },
    }
}

pub fn beat_knowledge_leak(index: u32, character: &str, knowledge: &str) -> Question {
    Question {
        key: format!("beat_{index}.leak.{character}"),
        template: Q_BEAT_KNOWLEDGE_LEAK.to_owned(),
        target: format!("beat_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "在这段正文里，{character}的言行是否表现出了他不可能知道的信息？\
                 他知道的信息：{knowledge}。true = 表现出他不可能知道的信息。"
            ),
            true_means: Some("泄露了他不知道的信息".into()),
            false_means: Some("没有泄露".into()),
        },
    }
}

pub fn beat_forbidden_resolution(index: u32, forbidden: &str) -> Question {
    Question {
        key: format!("beat_{index}.forbidden"),
        template: Q_BEAT_FORBIDDEN_RESOLUTION.to_owned(),
        target: format!("beat_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!("这段正文是否出现了被禁止的结果：{forbidden}？true = 出现了。"),
            true_means: Some("出现了被禁止的结果".into()),
            false_means: Some("没有出现".into()),
        },
    }
}

/// 未批准揭示的秘密。**问题里可以说秘密**——检查视图本来就是为了查泄露
/// （docs/08 §1：检查视图含未批准揭示的秘密）；受约束的是叙事视图与否决理由，
/// 后者不能把秘密说出来（docs/05 §2.5）。
pub fn beat_reveals_secret(index: u32, secret: &str) -> Question {
    Question {
        key: format!("beat_{index}.secret"),
        template: Q_BEAT_REVEALS_SECRET.to_owned(),
        target: format!("beat_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "这段正文是否向读者透露或明显暗示了：{secret}？true = 透露或明显暗示了。"
            ),
            true_means: Some("透露了".into()),
            false_means: Some("没有透露".into()),
        },
    }
}

/// 停止条件是否达成。`prior` 是此前已放行的节拍——它决定「累计到哪一步」。
pub fn beat_stop_reached(index: u32, stop: &str, prior: &[String]) -> Question {
    let context = if prior.is_empty() {
        "（这是本场景第一个节拍）".to_owned()
    } else {
        format!("此前的节拍：{}", prior.join(" / "))
    };
    Question {
        key: format!("beat_{index}.stop"),
        template: Q_BEAT_STOP_REACHED.to_owned(),
        target: format!("beat_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!(
                "到这一节拍为止，场景的停止条件「{stop}」是否已经达成？{context}。true = 已经达成。"
            ),
            true_means: Some("已经达成".into()),
            false_means: Some("尚未达成".into()),
        },
    }
}

pub fn beat_goal_done(index: u32, goal: &str) -> Question {
    Question {
        key: format!("beat_{index}.goal"),
        template: Q_BEAT_GOAL_DONE.to_owned(),
        target: format!("beat_{index}"),
        spec: QuestionSpec::Noul {
            instructions: format!("这段正文是否完成了节拍目标「{goal}」？true = 完成了。"),
            true_means: Some("完成了".into()),
            false_means: Some("没有完成".into()),
        },
    }
}

// ---------------------------------------------------------------- 结构

pub fn tendency_push(tendency: &str, text: &str) -> Question {
    Question {
        key: format!("{tendency}.push"),
        template: Q_TENDENCY_PUSH.to_owned(),
        target: tendency.to_owned(),
        spec: QuestionSpec::Score {
            instructions: format!("本回合发生的事，对趋势「{text}」的推动程度如何？"),
            criteria: ScoreCriteria(PUSH_LEVELS.iter().map(|s| (*s).to_owned()).collect()),
        },
    }
}

/// 故事线阶段。选项按阶段链顺序给出，`Choice` 的 criteria 是 record，
/// 键就是返回分布的键（docs/14 §2）。
pub fn thread_stage(thread: &str, title: &str, question: &str) -> Question {
    Question {
        key: format!("{thread}.stage"),
        template: Q_THREAD_STAGE.to_owned(),
        target: thread.to_owned(),
        spec: QuestionSpec::Choice {
            instructions: format!("故事线「{title}」（{question}）现在处于哪个阶段？"),
            criteria: ChoiceCriteria(
                [
                    ("seeded", "埋下：刚被提出，还没有真正展开"),
                    ("developing", "发展：正在推进，代价开始累积"),
                    ("escalating", "升级：冲突加剧，牵涉更多人或更大的代价"),
                    ("climax", "高潮：核心问题即将得到回答"),
                    ("resolved", "已解决：核心问题已经得到回答"),
                    ("abandoned", "已搁置：不再是故事关注的对象"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
            ),
        },
    }
}

/// 设定条目在关键词扫描里的呈现形式。`search_lore` 之类的读类工具会用它。
pub fn lore_summary(id: &LoreId, title: &str, section: &str) -> String {
    format!("[{}] {}（{}）", section, title, id.as_str())
}

#[cfg(test)]
mod tests;
