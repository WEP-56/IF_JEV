//! 阈值表与一致性严格度（docs/06 §2）。
//!
//! 两条硬规则（docs/06 §1）在这里落地：
//!
//! 1. 约束类与合规检查只比阈值，概率**永远不拿去掷骰**。所以这个模块里没有骰子。
//! 2. 「通过」和「可疑」的方向由模板决定。`q.cand.in_character` 概率越高越可靠，
//!    `q.beat.violates_fact` 概率越高越可疑——同一张表里两个方向。
//!
//! 第 2 条是这张表存在的真正理由。方向写错不会报错、不会 panic，只会静默地
//! 放过违规正文；所以方向必须和数据一样被显式声明，而不是留给调用方去记。

use serde::{Deserialize, Serialize};

/// 全局「一致性严格度」对单条阈值的最大调整幅度【初始值 0.4】。
///
/// docs/06 §2 要求「按比例线性收紧或放宽所有约束类阈值，并夹在各模板的上下限之间」。
/// 各模板的真实上下限要等阈值校准（每模板约 40 条标注集）才能定，现在统一用这个
/// 比例作为临时上下限；校准完成后应把它换成逐模板的界限表。
pub const STRICTNESS_MARGIN: f64 = 0.4;

/// L1 保护期内改动的阈值（docs/06 §4）。
///
/// 它不是 Jev 问题，而是引擎侧的规则——保护期内的改动根本不该送去抽样，
/// 所以没有 `q.` 前缀。
pub const L1_TEMPLATE: &str = "l1_protected_change";

/// 阈值方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdSense {
    /// 概率越高越**通过**：保留、采用、结束、允许、兼容、忠实、等价。
    HigherPasses,
    /// 概率越高越**触发**：违反、越界、泄露、泄密。
    HigherFlags,
}

/// 一条阈值的规则：值 + 方向。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThresholdSpec {
    pub value: f64,
    pub sense: ThresholdSense,
}

impl ThresholdSpec {
    pub const fn new(value: f64, sense: ThresholdSense) -> Self {
        Self { value, sense }
    }

    /// 放行与否。「放行」= 保留、采用、允许、没有违规。
    pub fn passes(&self, probability: f64) -> bool {
        match self.sense {
            ThresholdSense::HigherPasses => probability >= self.value,
            ThresholdSense::HigherFlags => probability < self.value,
        }
    }

    /// 触发与否。恒等于 `!passes`——方向只影响「触发」这个词怎么读。
    pub fn triggers(&self, probability: f64) -> bool {
        !self.passes(probability)
    }
}

/// 一致性严格度。`1.0` 表示用表里的初始值。
///
/// 它属于设置、不属于世界：改它不会让已提交的事件变得不可读，只会让后续判定
/// 更严或更松。所以它不写进 `WorldSettings`。
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Strictness(f64);

impl Strictness {
    pub const MIN: f64 = 0.5;
    pub const MAX: f64 = 2.0;

    /// 越界夹紧，非有限值回落到默认。
    pub fn new(factor: f64) -> Self {
        if factor.is_finite() {
            Self(factor.clamp(Self::MIN, Self::MAX))
        } else {
            Self(1.0)
        }
    }

    pub fn factor(self) -> f64 {
        self.0
    }

    pub fn is_default(self) -> bool {
        (self.0 - 1.0).abs() < f64::EPSILON
    }
}

impl Default for Strictness {
    fn default() -> Self {
        Self(1.0)
    }
}

impl From<f64> for Strictness {
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

/// 阈值表（docs/06 §2）。字段名 = 模板 ID 去掉 `q.` 前缀、点换下划线。
///
/// 每个世界可以单独覆盖（校准的产物就写在这里），默认值来自文档的【初始值】。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    pub input_is_directive: f64,
    pub if_parse_faithful: f64,
    pub if_compatible_with: f64,
    pub if_reconcile_plausible: f64,
    pub cand_in_character: f64,
    pub cand_knowledge_gap: f64,
    pub scene_resolves_thread: f64,
    pub beat_violates_fact: f64,
    pub beat_violates_rule: f64,
    pub beat_knowledge_leak: f64,
    pub beat_forbidden_resolution: f64,
    pub beat_reveals_secret: f64,
    pub beat_stop_reached: f64,
    pub extract_faithful: f64,
    pub extract_missing: f64,
    pub key_equivalent: f64,
    pub observe_leaks_secret: f64,
    pub l1_protected_change: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            input_is_directive: 0.5,
            if_parse_faithful: 0.75,
            if_compatible_with: 0.4,
            if_reconcile_plausible: 0.6,
            cand_in_character: 0.35,
            cand_knowledge_gap: 0.4,
            scene_resolves_thread: 0.4,
            beat_violates_fact: 0.3,
            beat_violates_rule: 0.35,
            beat_knowledge_leak: 0.35,
            beat_forbidden_resolution: 0.3,
            beat_reveals_secret: 0.3,
            beat_stop_reached: 0.6,
            extract_faithful: 0.6,
            extract_missing: 0.5,
            key_equivalent: 0.8,
            observe_leaks_secret: 0.3,
            l1_protected_change: 0.85,
        }
    }
}

/// 模板 ID（不含 `@版本`）、方向、以及该模板在 [`Thresholds`] 上的取值。
///
/// 方向与取值都只在这里声明一次：`spec()` 与 [`templates()`] 都从这张表读，
/// 所以不存在「加了模板忘了加方向」的中间态——加了行就两样都有。
type Getter = fn(&Thresholds) -> f64;

const TABLE: &[(&str, ThresholdSense, Getter)] = &[
    ("q.input.is_directive", ThresholdSense::HigherPasses, |t| {
        t.input_is_directive
    }),
    ("q.if.parse_faithful", ThresholdSense::HigherPasses, |t| {
        t.if_parse_faithful
    }),
    // true = 能同时成立，所以概率高是好事，冲突是「低」触发。
    ("q.if.compatible_with", ThresholdSense::HigherPasses, |t| {
        t.if_compatible_with
    }),
    (
        "q.if.reconcile_plausible",
        ThresholdSense::HigherPasses,
        |t| t.if_reconcile_plausible,
    ),
    ("q.cand.in_character", ThresholdSense::HigherPasses, |t| {
        t.cand_in_character
    }),
    ("q.cand.knowledge_gap", ThresholdSense::HigherFlags, |t| {
        t.cand_knowledge_gap
    }),
    (
        "q.scene.resolves_thread",
        ThresholdSense::HigherFlags,
        |t| t.scene_resolves_thread,
    ),
    ("q.beat.violates_fact", ThresholdSense::HigherFlags, |t| {
        t.beat_violates_fact
    }),
    ("q.beat.violates_rule", ThresholdSense::HigherFlags, |t| {
        t.beat_violates_rule
    }),
    ("q.beat.knowledge_leak", ThresholdSense::HigherFlags, |t| {
        t.beat_knowledge_leak
    }),
    (
        "q.beat.forbidden_resolution",
        ThresholdSense::HigherFlags,
        |t| t.beat_forbidden_resolution,
    ),
    ("q.beat.reveals_secret", ThresholdSense::HigherFlags, |t| {
        t.beat_reveals_secret
    }),
    ("q.beat.stop_reached", ThresholdSense::HigherPasses, |t| {
        t.beat_stop_reached
    }),
    ("q.extract.faithful", ThresholdSense::HigherPasses, |t| {
        t.extract_faithful
    }),
    ("q.extract.missing", ThresholdSense::HigherPasses, |t| {
        t.extract_missing
    }),
    ("q.key.equivalent", ThresholdSense::HigherPasses, |t| {
        t.key_equivalent
    }),
    (
        "q.observe.leaks_secret",
        ThresholdSense::HigherFlags,
        |t| t.observe_leaks_secret,
    ),
    (L1_TEMPLATE, ThresholdSense::HigherPasses, |t| {
        t.l1_protected_change
    }),
];

/// 模板 ID → 规范名：去掉 `@版本`（`Judgment.template` 记的是 `q.x@1`）。
pub fn normalize(template: &str) -> &str {
    template.split('@').next().unwrap_or(template)
}

/// 全部模板 ID，按表序。
pub fn templates() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|(id, _, _)| *id)
}

impl Thresholds {
    /// 按模板 ID 取规则，接受带版本的写法（`q.beat.violates_fact@1`）。
    ///
    /// 返回 `None` 表示这个模板没有阈值——那它就不该走阈值裁决。
    pub fn spec(&self, template: &str) -> Option<ThresholdSpec> {
        let key = normalize(template);
        let (_, sense, get) = TABLE.iter().find(|(id, _, _)| *id == key)?;
        Some(ThresholdSpec::new(get(self), *sense))
    }

    /// 施加一致性严格度之后的规则（docs/06 §2）。
    pub fn spec_with(&self, template: &str, strictness: Strictness) -> Option<ThresholdSpec> {
        let spec = self.spec(template)?;
        Some(ThresholdSpec::new(
            tighten(spec.value, spec.sense, strictness),
            spec.sense,
        ))
    }

    /// 放行与否。模板不存在时返回 `None`——调用方得自己决定这意味着什么，
    /// 悄悄当作「通过」会让拼错的模板 ID 永远不被发现。
    pub fn passes(&self, template: &str, probability: f64, strictness: Strictness) -> Option<bool> {
        self.spec_with(template, strictness)
            .map(|spec| spec.passes(probability))
    }
}

/// 一致性严格度的线性收紧 / 放宽，夹在 `[τ·(1−margin), τ·(1+margin)]` 之间。
///
/// - `HigherFlags` 的模板：越严格 → 阈值越低 → 越容易判为违反。
/// - `HigherPasses` 的模板：越严格 → 阈值越高 → 越难通过。
pub fn tighten(value: f64, sense: ThresholdSense, strictness: Strictness) -> f64 {
    if strictness.is_default() {
        return value;
    }
    let factor = strictness.factor();
    let raw = match sense {
        ThresholdSense::HigherFlags => value / factor,
        ThresholdSense::HigherPasses => value * factor,
    };
    raw.clamp(value * (1.0 - STRICTNESS_MARGIN), value * (1.0 + STRICTNESS_MARGIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_table_matches_docs_06_s2() {
        let t = Thresholds::default();
        for (template, expected) in [
            ("q.input.is_directive", 0.5),
            ("q.if.parse_faithful", 0.75),
            ("q.if.compatible_with", 0.4),
            ("q.if.reconcile_plausible", 0.6),
            ("q.cand.in_character", 0.35),
            ("q.cand.knowledge_gap", 0.4),
            ("q.scene.resolves_thread", 0.4),
            ("q.beat.violates_fact", 0.3),
            ("q.beat.violates_rule", 0.35),
            ("q.beat.knowledge_leak", 0.35),
            ("q.beat.forbidden_resolution", 0.3),
            ("q.beat.reveals_secret", 0.3),
            ("q.beat.stop_reached", 0.6),
            ("q.extract.faithful", 0.6),
            ("q.extract.missing", 0.5),
            ("q.key.equivalent", 0.8),
            ("q.observe.leaks_secret", 0.3),
            (L1_TEMPLATE, 0.85),
        ] {
            let spec = t.spec(template).unwrap_or_else(|| panic!("缺 {template}"));
            assert!((spec.value - expected).abs() < 1e-12, "{template}");
        }
    }

    #[test]
    fn every_table_row_is_reachable() {
        let t = Thresholds::default();
        // 表里每行都要能取到值，且值落在合法概率区间里
        for template in templates() {
            let spec = t.spec(template).expect(template);
            assert!((0.0..=1.0).contains(&spec.value), "{template}");
        }
        // 反向：`q.` 前缀的模板都在表里
        assert_eq!(templates().filter(|id| id.starts_with("q.")).count(), 17);
        assert!(t.spec("q.nonexistent").is_none());
        assert!(t.passes("q.nonexistent", 0.9, Strictness::default()).is_none());
    }

    #[test]
    fn versioned_template_ids_resolve() {
        let t = Thresholds::default();
        assert_eq!(t.spec("q.beat.violates_fact@1"), t.spec("q.beat.violates_fact"));
        assert_eq!(normalize("q.beat.violates_fact@3"), "q.beat.violates_fact");
        assert_eq!(normalize(L1_TEMPLATE), L1_TEMPLATE);
    }

    #[test]
    fn flag_direction_is_not_reversed() {
        let t = Thresholds::default();
        // 「违反」类：低概率=清白，高概率=违规
        let fact = t.spec("q.beat.violates_fact").unwrap();
        assert_eq!(fact.sense, ThresholdSense::HigherFlags);
        assert!(fact.passes(0.29));
        assert!(!fact.passes(0.3));
        assert!(fact.triggers(0.88));

        // 「符合人设」类：低概率=丢弃，高概率=保留
        let character = t.spec("q.cand.in_character").unwrap();
        assert_eq!(character.sense, ThresholdSense::HigherPasses);
        assert!(!character.passes(0.34));
        assert!(character.passes(0.35));
    }

    #[test]
    fn compatible_with_flags_on_low_probability() {
        // docs/06 §2：`p < τ 判为冲突`。冲突是触发，所以方向必须是 HigherPasses。
        let t = Thresholds::default();
        let spec = t.spec("q.if.compatible_with").unwrap();
        assert!(spec.passes(0.4)); // 能同时成立 → 不冲突
        assert!(spec.triggers(0.39)); // 判为冲突
    }

    #[test]
    fn l1_change_needs_a_high_probability() {
        let t = Thresholds::default();
        assert!(!t.passes(L1_TEMPLATE, 0.84, Strictness::default()).unwrap());
        assert!(t.passes(L1_TEMPLATE, 0.85, Strictness::default()).unwrap());
    }

    #[test]
    fn strictness_moves_both_directions_and_clamps() {
        let t = Thresholds::default();
        let strict = Strictness::new(2.0);

        // 违规类：阈值被压低 → 更容易触发
        let relaxed = t.spec("q.beat.violates_fact").unwrap();
        let tightened = t.spec_with("q.beat.violates_fact", strict).unwrap();
        assert!(tightened.value < relaxed.value);
        assert!((tightened.value - 0.3 * (1.0 - STRICTNESS_MARGIN)).abs() < 1e-12);
        // 0.25 在默认下清白，在严格下违规
        assert!(relaxed.passes(0.25));
        assert!(tightened.triggers(0.25));

        // 通过类：阈值被抬高 → 更难通过
        let relaxed = t.spec("q.cand.in_character").unwrap();
        let tightened = t.spec_with("q.cand.in_character", strict).unwrap();
        assert!(tightened.value > relaxed.value);
        assert!((tightened.value - 0.35 * (1.0 + STRICTNESS_MARGIN)).abs() < 1e-12);
        assert!(relaxed.passes(0.4));
        assert!(!tightened.passes(0.4));

        // 夹紧：无论多严格都不越过 ±margin
        let absurd = Strictness::new(99.0);
        assert_eq!(absurd.factor(), Strictness::MAX);
        let edge = t.spec_with("q.beat.violates_fact", absurd).unwrap();
        assert!(edge.value >= 0.3 * (1.0 - STRICTNESS_MARGIN) - 1e-12);
    }

    #[test]
    fn relaxed_strictness_loosens_constraints() {
        let t = Thresholds::default();
        let loose = Strictness::new(0.5);
        let spec = t.spec_with("q.beat.violates_fact", loose).unwrap();
        assert!(spec.value > 0.3);
        // 松开之后 0.35 不再算违规
        assert!(t.spec("q.beat.violates_fact").unwrap().triggers(0.35));
        assert!(spec.passes(0.35));
    }

    #[test]
    fn strictness_is_clamped_and_finite_guarded() {
        assert_eq!(Strictness::new(0.0).factor(), Strictness::MIN);
        assert_eq!(Strictness::new(-3.0).factor(), Strictness::MIN);
        assert_eq!(Strictness::new(f64::NAN).factor(), 1.0);
        assert_eq!(Strictness::new(f64::INFINITY).factor(), 1.0);
        assert!(Strictness::default().is_default());
        assert!(!Strictness::new(1.5).is_default());
        // 默认严格度下不做任何换算，避免浮点往返
        let t = Thresholds::default();
        assert_eq!(
            t.spec_with("q.beat.violates_fact", Strictness::default()).unwrap(),
            t.spec("q.beat.violates_fact").unwrap()
        );
    }

    #[test]
    fn thresholds_round_trip_through_json() {
        let t = Thresholds::default();
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Thresholds>(&json).unwrap(), t);
    }
}
