//! 世界书激活（docs/08 §5）。
//!
//! 设定条目是「关键词 → 散文」的上下文注入机制（docs/02 §9.1）。每次编译视图前，
//! 由这里决定「这一轮该把哪些条目放进去」。
//!
//! 机制与酒馆世界书一致（这样社区的世界书可以直接导入），加两条 IF 自己的约束：
//!
//! 1. **结构化激活**：主体处于焦点时，关联它的条目直接激活。比关键词可靠得多，
//!    因为关键词会误命中，而「这条是写给林夏的」不会。
//! 2. **概率激活走命运骰子**：键是 `lore:<条目 ID>@<场景序号>`，所以同一场景
//!    重放必然得到同一个结果（docs/03 §8）。用 `rand` 就做不到这一点。
//!
//! 这里**不做可见性过滤**。secret 条目该不该进叙事视图是视图编译器的事
//! （[`if_views`] 的 `lore_lines` 按持有者过滤），激活层只回答「这一轮谁的条目在场」。
//! 混在一起做会让「这条没进正文」的原因变得说不清。
//!
//! 递归带深度上限（初始值 2）：已激活条目的内容可以触发别的条目，但深度封顶，
//! 否则一条写得很泛的条目会把整个世界书拖进来。

use std::collections::BTreeSet;

use if_domain::id::SubjectId;
use if_domain::narrative::{LoreEntry, LoreStatus, SecondaryLogic};
use if_domain::projection::Projection;
use if_domain::rule::{CompareOp, Condition};
use if_domain::value::Value;
use if_policy::{lore_probe_key, Dice};

/// 递归深度的初始值（docs/08 §5）。
pub const DEFAULT_MAX_DEPTH: u32 = 2;

/// 单次编译最多激活多少条【初始值】。
///
/// 真实卡的条目规模在 148 条 / 10 万字量级（docs/13 §6.4），整卡注入不可行，
/// 所以预算必须真做。超预算时按 order 从低到高丢弃。
pub const DEFAULT_BUDGET: usize = 24;

/// 激活所需的上下文。
#[derive(Clone, Debug, Default)]
pub struct LoreSignals {
    /// 当前场景序号。概率激活的键与 L1 保护期都按它算。
    pub scene_index: u64,
    /// 焦点主体。关联它们的条目直接激活。
    pub focus: Vec<SubjectId>,
    /// 关键词扫描范围：场景计划、最近 N 个节拍、当前任务的输入（docs/08 §5）。
    pub scan: String,
    pub budget: usize,
    pub max_depth: u32,
}

impl LoreSignals {
    pub fn new(scene_index: u64) -> Self {
        Self {
            scene_index,
            focus: Vec::new(),
            scan: String::new(),
            budget: DEFAULT_BUDGET,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }

    pub fn focus(mut self, subjects: impl IntoIterator<Item = SubjectId>) -> Self {
        self.focus = subjects.into_iter().collect();
        self
    }

    pub fn scan(mut self, text: impl Into<String>) -> Self {
        self.scan = text.into();
        self
    }

    pub fn budget(mut self, budget: usize) -> Self {
        self.budget = budget;
        self
    }

    pub fn max_depth(mut self, depth: u32) -> Self {
        self.max_depth = depth;
        self
    }
}

/// 激活设定条目，返回按优先级排好序的结果。
///
/// 同一份投影 + 同一组信号 + 同一颗骰子，必然返回同一个 `Vec`（含顺序）。
pub fn activate_lore(
    projection: &Projection,
    signals: &LoreSignals,
    dice: &Dice,
    salt: &str,
) -> Vec<LoreEntry> {
    let scan = signals.scan.to_lowercase();
    let mut active: BTreeSet<String> = BTreeSet::new();
    let mut picked: Vec<LoreEntry> = Vec::new();

    // 候选按 ID 排序后遍历，保证「先到先得」的那部分也是确定的。
    let entries: Vec<&LoreEntry> = projection
        .lore
        .values()
        .filter(|entry| entry.status == LoreStatus::Active)
        .collect();

    // ---- 第一轮：常驻 + 结构化 + 关键词 + 条件
    for entry in &entries {
        let seed = entry.constant
            || structured_hit(entry, &signals.focus)
            || keyword_hit(entry, &scan)
            || condition_holds(projection, entry);
        if seed && admits(entry, signals, dice, salt) {
            active.insert(entry.id.as_str().to_owned());
            picked.push((*entry).clone());
        }
    }

    // ---- 递归：已激活条目的内容可以触发其他条目（docs/08 §5）
    let mut frontier: Vec<String> = picked.iter().map(|entry| entry.content.to_lowercase()).collect();
    for _ in 0..signals.max_depth {
        if frontier.is_empty() {
            break;
        }
        let haystack = frontier.join("\n");
        let mut next: Vec<String> = Vec::new();
        for entry in &entries {
            if active.contains(entry.id.as_str()) {
                continue;
            }
            let triggered = entry
                .keys
                .iter()
                .any(|key| !key.trim().is_empty() && haystack.contains(&key.to_lowercase()));
            if triggered && admits(entry, signals, dice, salt) {
                active.insert(entry.id.as_str().to_owned());
                next.push(entry.content.to_lowercase());
                picked.push((*entry).clone());
            }
        }
        frontier = next;
    }

    // ---- 排序与预算裁剪：常驻优先，其次按 order 从高到低，最后按 ID 稳定
    picked.sort_by(|a, b| {
        b.constant
            .cmp(&a.constant)
            .then(b.order.cmp(&a.order))
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });
    let budget = if signals.budget == 0 {
        DEFAULT_BUDGET
    } else {
        signals.budget
    };
    picked.truncate(budget);
    // 输出仍按 ID 排序，便于快照测试与指纹稳定——truncate 的取舍已经发生过了。
    picked.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    picked
}

/// 门禁：概率条目要走命运骰子。没有 `probability` 的条目直接通过。
fn admits(entry: &LoreEntry, signals: &LoreSignals, dice: &Dice, salt: &str) -> bool {
    let Some(probability) = entry.probability else {
        return true;
    };
    if !probability.is_finite() {
        // 非有限值当作「这条配置坏了」，激活它比静默丢掉更容易被发现。
        return true;
    }
    let key = lore_probe_key(&entry.id, signals.scene_index);
    dice.sample(&key, probability.clamp(0.0, 1.0), salt)
}

/// 结构化激活：条目关联的主体里有任何一个处于焦点。
fn structured_hit(entry: &LoreEntry, focus: &[SubjectId]) -> bool {
    !entry.subjects.is_empty() && entry.subjects.iter().any(|subject| focus.contains(subject))
}

/// 关键词激活。
///
/// 主关键词按「任一命中」；次关键词按作者声明的四种逻辑处理（docs/08 §5）。
/// 未声明次关键词时视为通过——否则一条只写了主关键词的条目会永远激活不了。
fn keyword_hit(entry: &LoreEntry, scan_lower: &str) -> bool {
    let primary = entry
        .keys
        .iter()
        .any(|key| !key.trim().is_empty() && scan_lower.contains(&key.to_lowercase()));
    if !primary {
        return false;
    }
    let secondaries: Vec<bool> = entry
        .secondary_keys
        .iter()
        .map(|key| !key.trim().is_empty() && scan_lower.contains(&key.to_lowercase()))
        .collect();
    if secondaries.is_empty() {
        return true;
    }
    match entry.logic {
        SecondaryLogic::AndAny => secondaries.iter().any(|hit| *hit),
        SecondaryLogic::AndAll => secondaries.iter().all(|hit| *hit),
        SecondaryLogic::NotAny => secondaries.iter().all(|hit| !*hit),
        SecondaryLogic::NotAll => secondaries.iter().any(|hit| !*hit),
    }
}

/// 条件激活（IF 新增）：`when` 条件引用事实。条件写法不清或引用不到事实时**不激活**——
/// 「拿不准就别塞进上下文」比「猜一个」安全。
fn condition_holds(projection: &Projection, entry: &LoreEntry) -> bool {
    entry
        .when
        .as_ref()
        .is_some_and(|condition| eval(projection, condition))
}

fn eval(projection: &Projection, condition: &Condition) -> bool {
    match condition {
        Condition::Compare { prop, cmp, value } => {
            let Some(fact) = projection.facts.get(prop) else {
                return false;
            };
            numerical(&fact.value).is_some_and(|lhs| compare(*cmp, lhs, *value))
        }
        Condition::All { all } => all.iter().all(|c| eval(projection, c)),
        Condition::Any { any } => any.iter().any(|c| eval(projection, c)),
        Condition::Not { not } => !eval(projection, not),
    }
}

fn numerical(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        // 文本没法和数字比大小。硬把 "是" 当成 1 只会让条件悄悄命中。
        Value::Text(_) => None,
    }
}

fn compare(op: CompareOp, lhs: f64, rhs: f64) -> bool {
    if !lhs.is_finite() || !rhs.is_finite() {
        return false;
    }
    op.test(lhs, rhs)
}

#[cfg(test)]
mod tests;
