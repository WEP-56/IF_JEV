//! 命运骰子（docs/03 §8）。
//!
//! ```text
//! u = H(world_seed ‖ decision_key ‖ salt) → [0, 1)
//! ```
//!
//! 这套设计只有一个目的：**同一个决定在两条世界线上用同一颗骰子**。
//! 于是「这个 IF 改变了什么」可以被准确展示——
//! `骰子 0.55，旧世界线 p = 0.41 → 未发生；新世界线 p = 0.72 → 发生`。
//!
//! 成立的前提全在 `decision_key` 上：它必须是**语义**标识，不能是回合内分配的
//! 候选 ID 或事件序号。键不稳定的候选，其骰子值也就不可复现。
//!
//! 哈希原语来自 [`if_views::digest_unit`]——骰子与视图指纹必须共用同一套
//! 确定性实现，否则「同一输入同一输出」这条性质会被拆成两份去维护。

use std::collections::BTreeMap;

use if_domain::{LoreId, MechanicStep, NumericRange, WorldTime};
use if_views::digest_unit;

/// 默认盐值。重掷时给该场景换一个盐值（docs/03 §8）。
pub const NO_SALT: &str = "";

/// 一颗属于某个世界的骰子。
///
/// 种子来自世界（`world_created` 写入），不属于全局设置——同一个世界在
/// 两条世界线上必须掷出同一颗骰子。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dice {
    seed: u64,
}

impl Dice {
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub const fn seed(self) -> u64 {
        self.seed
    }

    /// 取一个 `u ∈ [0, 1)`，不加盐。
    pub fn roll(&self, decision_key: &str) -> f64 {
        self.roll_salted(decision_key, NO_SALT)
    }

    /// 取一个 `u ∈ [0, 1)`，带盐。
    pub fn roll_salted(&self, decision_key: &str, salt: &str) -> f64 {
        digest_unit(&[
            &self.seed.to_be_bytes(),
            decision_key.as_bytes(),
            salt.as_bytes(),
        ])
    }

    /// 发生类：`u < p` 即发生（docs/06 §3）。
    pub fn sample(&self, decision_key: &str, probability: f64, salt: &str) -> bool {
        self.roll_salted(decision_key, salt) < probability
    }

    /// 互斥类：选项按 ID 排序后累加分布，看 `u` 落在哪个选项上（docs/06 §3）。
    ///
    /// `probabilities` 是 `BTreeMap`，键序天然就是选项 ID 序，不需要另外排序。
    /// 分布的和不一定恰好是 1（Jev 的浮点输出），所以按总和归一化再定位——
    /// 不归一化的话，剩下那点残差会让结果偏向最后一项。
    ///
    /// 返回 `None` 只在这种情况下：分布为空，或者所有概率都不是正数。
    /// 除此之外一定选出一个选项——互斥组的语义是「必然发生其中一个」。
    pub fn categorical(
        &self,
        decision_key: &str,
        probabilities: &BTreeMap<String, f64>,
        salt: &str,
    ) -> Option<String> {
        let total: f64 = probabilities
            .values()
            .filter(|p| p.is_finite() && **p > 0.0)
            .sum();
        if total <= 0.0 {
            return None;
        }
        let u = self.roll_salted(decision_key, salt);
        let mut accumulated = 0.0;
        let mut last_positive: Option<&String> = None;
        for (option, probability) in probabilities {
            if !probability.is_finite() || *probability < 0.0 {
                continue;
            }
            accumulated += probability;
            if accumulated > 0.0 {
                last_positive = Some(option);
            }
            if u < accumulated / total {
                return Some(option.clone());
            }
        }
        // 浮点误差导致 `u` 落不到任何一段时，取最后一个正概率选项，
        // 而不是返回 None——那会把「抽到了尾项」误报成「抽不出结果」。
        last_positive.cloned()
    }

    /// 数值机制：`value = min + u × (max − min)`（docs/06 §8）。
    pub fn magnitude(&self, decision_key: &str, range: NumericRange, salt: &str) -> f64 {
        range.sample(self.roll_salted(decision_key, salt))
    }
}

// ---------------------------------------------------------------- 决策键

/// 状态候选：`<命题键>@<世界时间段>`，如 `c_gu.notices.c_lin_abnormal@D7`。
pub fn candidate_key(proposition_key: &str, step: MechanicStep, at: WorldTime) -> String {
    bucketed(proposition_key, step, at)
}

/// 互斥选择点：`<选择点键>@<世界时间段>`，如 `c_lin.reaction_to_feeling@D7`。
pub fn exclusive_key(choice_point: &str, step: MechanicStep, at: WorldTime) -> String {
    bucketed(choice_point, step, at)
}

/// 数值机制：`<机制 ID>@<机制步>`，如 `m_river_rise@D8`。
pub fn mechanic_key(mechanic_id: &str, step: MechanicStep, at: WorldTime) -> String {
    bucketed(mechanic_id, step, at)
}

/// 场景选择：`scene_select@<叙述序号>`，如 `scene_select@48`。
///
/// 用的是**叙述序号**而不是世界时间——场景选择发生在时间被推进之前。
pub fn scene_select_key(narrative_order: u64) -> String {
    format!("scene_select@{narrative_order}")
}

/// 设定条目概率激活：`lore:<条目 ID>@<场景序号>`，如 `lore:e_12@48`。
///
/// 条目 ID 带着 `lore_` 前缀，键里再写一遍就成了 `lore:lore_e_12@48`，
/// 所以这里把前缀剥掉，与 docs/03 §8 的写法对齐。
pub fn lore_probe_key(lore: &LoreId, scene_index: u64) -> String {
    let body = lore
        .as_str()
        .strip_prefix(LoreId::PREFIX)
        .unwrap_or_else(|| lore.as_str());
    format!("lore:{body}@{scene_index}")
}

fn bucketed(id: &str, step: MechanicStep, at: WorldTime) -> String {
    format!("{id}@{}", step.bucket_label(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn same_key_same_seed_same_die() {
        let dice = Dice::new(20260926);
        assert_eq!(dice.roll("c_gu.notices@D7"), dice.roll("c_gu.notices@D7"));
        // 不同世界（不同种子）不该共用骰子
        assert_ne!(
            dice.roll("c_gu.notices@D7"),
            Dice::new(7).roll("c_gu.notices@D7")
        );
    }

    #[test]
    fn die_is_in_half_open_range() {
        let dice = Dice::new(1);
        for i in 0..500 {
            let u = dice.roll(&format!("k{i}"));
            assert!((0.0..1.0).contains(&u), "{u}");
        }
    }

    #[test]
    fn salt_changes_the_die() {
        let dice = Dice::new(42);
        let plain = dice.roll_salted("scene_select@48", NO_SALT);
        let salted = dice.roll_salted("scene_select@48", "reroll-1");
        assert_ne!(plain, salted);
        // 同一个盐值仍然稳定
        assert_eq!(salted, dice.roll_salted("scene_select@48", "reroll-1"));
    }

    #[test]
    fn key_parts_do_not_bleed_into_each_other() {
        // 种子与键的字节拼接必须无歧义，否则 (seed=1, key="2x") 与 (seed=12, key="x")
        // 会撞到同一颗骰子。
        let a = Dice::new(1).roll("2x");
        let b = Dice::new(12).roll("x");
        assert_ne!(a, b);
    }

    #[test]
    fn sampling_flips_exactly_when_probability_crosses_the_die() {
        let dice = Dice::new(20260926);
        let u = dice.roll("c_gu.notices@D7");
        assert!(dice.sample("c_gu.notices@D7", u + 1e-9, NO_SALT));
        assert!(!dice.sample("c_gu.notices@D7", u - 1e-9, NO_SALT));
        // 边界：`u < p` 是严格小于，p 恰好等于 u 时不发生
        assert!(!dice.sample("c_gu.notices@D7", u, NO_SALT));
    }

    #[test]
    fn categorical_respects_option_id_order() {
        let dice = Dice::new(20260926);
        let probabilities = dist(&[("leave", 0.5), ("stay", 0.5)]);
        // 手算同一个 u 该落在哪一段
        let u = dice.roll("c_lin.reaction@D7");
        let expected = if u < 0.5 { "leave" } else { "stay" };
        assert_eq!(
            dice.categorical("c_lin.reaction@D7", &probabilities, NO_SALT),
            Some(expected.to_string())
        );
    }

    #[test]
    fn categorical_skips_zero_options_and_normalizes() {
        let dice = Dice::new(20260926);
        // 和只有 0.8：归一化后仍然一定选中一个，且不选中零概率项
        let probabilities = dist(&[("a", 0.0), ("b", 0.8), ("c", 0.0)]);
        for i in 0..200 {
            let key = format!("choice@{i}");
            assert_eq!(
                dice.categorical(&key, &probabilities, NO_SALT),
                Some("b".to_string())
            );
        }
    }

    #[test]
    fn categorical_returns_none_only_without_positive_mass() {
        let dice = Dice::new(1);
        assert_eq!(dice.categorical("k", &BTreeMap::new(), NO_SALT), None);
        assert_eq!(
            dice.categorical("k", &dist(&[("a", 0.0)]), NO_SALT),
            None
        );
        assert_eq!(
            dice.categorical("k", &dist(&[("a", f64::NAN)]), NO_SALT),
            None
        );
    }

    #[test]
    fn categorical_always_picks_something_for_a_valid_distribution() {
        let dice = Dice::new(7);
        let probabilities = dist(&[("a", 0.25), ("b", 0.25), ("c", 0.25), ("d", 0.25)]);
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..400 {
            let picked = dice
                .categorical(&format!("cp@{i}"), &probabilities, NO_SALT)
                .expect("一定选出一个");
            seen.insert(picked);
        }
        // 四个选项都该被抽到过，否则不是分类抽样
        assert_eq!(seen.len(), 4, "{seen:?}");
    }

    #[test]
    fn magnitude_hits_the_range() {
        let dice = Dice::new(20260926);
        let range = NumericRange::new(0.4, 1.2);
        let value = dice.magnitude("m_river_rise@D8", range, NO_SALT);
        assert!((0.4..1.2).contains(&value), "{value}");
        assert_eq!(
            value,
            dice.magnitude("m_river_rise@D8", range, NO_SALT),
            "同一机制步必须取到同一个增量"
        );
    }

    #[test]
    fn decision_keys_match_docs_03_s8_examples() {
        let at = WorldTime::from_days(7);
        assert_eq!(
            candidate_key("c_gu.notices.c_lin_abnormal", MechanicStep::Day, at),
            "c_gu.notices.c_lin_abnormal@D7"
        );
        assert_eq!(
            exclusive_key("c_lin.reaction_to_feeling", MechanicStep::Day, at),
            "c_lin.reaction_to_feeling@D7"
        );
        // 世界时间段的粒度等于机制步长
        assert_eq!(
            candidate_key("c_gu.notices", MechanicStep::Day, WorldTime::from_days(7).plus_minutes(1439)),
            "c_gu.notices@D7"
        );
        assert_eq!(
            candidate_key("c_gu.notices", MechanicStep::Hour, WorldTime::from_days(7).plus_minutes(60)),
            "c_gu.notices@H169"
        );
        assert_eq!(mechanic_key("m_river_rise", MechanicStep::Day, at), "m_river_rise@D7");
        assert_eq!(scene_select_key(48), "scene_select@48");
        // 条目 ID 的 `lore_` 前缀不该在键里出现两次
        assert_eq!(lore_probe_key(&LoreId::new("lore_e_12"), 48), "lore:e_12@48");
    }
}
