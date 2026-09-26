//! 导演评分与场景选择（docs/06 §6）。
//!
//! 场景调度不是「选分数最高的那个」——那样会永远只写同一种场景。也不是纯随机。
//! 它是一组加权项求和，再按 `P(i) ∝ exp(scoreᵢ / T)` 抽取：
//! 分数高的场景大概率被选中，但不是必然。
//!
//! **导演风格就是一组权重。** 这里不做任何「哪个风格更好」的判断，
//! 风格只决定权重表，剩下的是算术。

use std::collections::BTreeMap;

use if_domain::{SubjectId, Thread};
use serde::{Deserialize, Serialize};

/// 叙事温度【初始值 0.15】（docs/06 §6）。越小越接近「只取最高分」。
pub const DIRECTOR_TEMPERATURE: f64 = 0.15;

/// 导演风格的权重表（docs/06 §6）。
///
/// `repetition` 同时作为两项惩罚的权重：`q.scene.repetitive` 的重复惩罚，
/// 以及未受保护故事线的「过早解决」惩罚。文档的表里没有单独一列，
/// 两者都是「这事不该再来一次」的同类信号。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectorWeights {
    pub fit: f64,
    pub tension: f64,
    pub thread: f64,
    pub overdue: f64,
    pub balance: f64,
    pub tendency: f64,
    pub repetition: f64,
}

impl DirectorWeights {
    /// 均衡（默认）。
    pub const BALANCED: Self = Self {
        fit: 1.0,
        tension: 0.8,
        thread: 0.6,
        overdue: 0.5,
        balance: 0.4,
        tendency: 0.6,
        repetition: 0.8,
    };

    /// 慢热言情：故事线与角色出场优先，张力与逾期都放低。
    pub const SLOW_BURN: Self = Self {
        fit: 1.0,
        tension: 0.4,
        thread: 0.8,
        overdue: 0.3,
        balance: 0.6,
        tendency: 0.8,
        repetition: 0.6,
    };

    /// 悬疑：张力、趋势、避免重复都给高权重。
    pub const MYSTERY: Self = Self {
        fit: 1.0,
        tension: 0.9,
        thread: 0.7,
        overdue: 0.6,
        balance: 0.3,
        tendency: 0.9,
        repetition: 0.9,
    };

    /// 爽文：张力与推进速度优先，角色平衡几乎不管。
    pub const POWER_FANTASY: Self = Self {
        fit: 0.8,
        tension: 1.0,
        thread: 0.5,
        overdue: 0.8,
        balance: 0.2,
        tendency: 0.5,
        repetition: 0.7,
    };

    /// 按 `WorldSettings::director_style` 取权重。未知风格回落到均衡——
    /// 风格名来自用户输入或世界书，拼错不该让整个回合失败。
    pub fn for_style(style: &str) -> Self {
        match style {
            "慢热言情" => Self::SLOW_BURN,
            "悬疑" => Self::MYSTERY,
            "爽文" => Self::POWER_FANTASY,
            _ => Self::BALANCED,
        }
    }

    /// `score = Σ wᵢ · termᵢ`（docs/06 §6）。两项惩罚取负号。
    pub fn score(&self, signals: &SceneSignals) -> f64 {
        let s = signals.sanitized();
        self.fit * s.semantic_fit
            + self.tension * s.tension_gain
            + self.thread * s.thread_pressure
            + self.overdue * s.overdue_bonus
            + self.balance * s.character_balance
            + self.tendency * s.tendency_bonus
            - self.repetition * s.repetition
            - self.repetition * s.premature_resolution
    }
}

/// 一个候选场景的评分项。
///
/// 前两项与后两项来自 Jev（比较类问题），中间四项由引擎算。
/// 每项都应落在 `[0, 1]`；不在区间里的值会被夹紧，NaN 当作 0
/// （docs/06 §6 要求「每一项先归一化到 [0, 1] 再加权求和」）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneSignals {
    /// `q.scene.fit` 的 p。
    pub semantic_fit: f64,
    /// `q.scene.tension`，按 `score / (等级数 − 1)` 归一化（docs/06 §7）。
    pub tension_gain: f64,
    /// 该场景推进的故事线的平均压力。
    pub thread_pressure: f64,
    /// 逾期程度，见 [`Thread::overdue_bonus`]。
    pub overdue_bonus: f64,
    /// 焦点角色的出场缺额：越久没出场越高。
    pub character_balance: f64,
    /// 该场景是否让高压趋势爆发。
    pub tendency_bonus: f64,
    /// `q.scene.repetitive` 的 p（惩罚项）。
    pub repetition: f64,
    /// `q.scene.resolves_thread` 的 p（惩罚项）。
    ///
    /// 受保护的故事线不在这里——它们在回合流程第 8 步就被否决了（docs/04 §2.1），
    /// 根本进不到导演评分。
    pub premature_resolution: f64,
}

impl SceneSignals {
    /// 夹到 `[0, 1]`，非有限值归零。
    fn sanitized(&self) -> Self {
        fn clean(value: f64) -> f64 {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        }
        Self {
            semantic_fit: clean(self.semantic_fit),
            tension_gain: clean(self.tension_gain),
            thread_pressure: clean(self.thread_pressure),
            overdue_bonus: clean(self.overdue_bonus),
            character_balance: clean(self.character_balance),
            tendency_bonus: clean(self.tendency_bonus),
            repetition: clean(self.repetition),
            premature_resolution: clean(self.premature_resolution),
        }
    }
}

// ---------------------------------------------------------------- 引擎侧的四项

/// 该场景推进的故事线的平均压力。
pub fn thread_pressure<'a>(threads: impl IntoIterator<Item = &'a Thread>) -> f64 {
    let mut count = 0_usize;
    let mut total = 0.0;
    for thread in threads {
        total += if thread.pressure.is_finite() {
            thread.pressure.clamp(0.0, 1.0)
        } else {
            0.0
        };
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

/// 焦点角色的出场缺额。
///
/// `recent_appearances` 给出每个主体在最近 `window` 个场景里出场了几次；
/// 缺额 = `1 − min(出场次数 / window, 1)`，再对焦点角色取平均。
/// 全部缺席 = 1，全都在场 = 0。
pub fn character_balance(
    focus: &[SubjectId],
    recent_appearances: &BTreeMap<SubjectId, u64>,
    window: u64,
) -> f64 {
    if focus.is_empty() || window == 0 {
        return 0.0;
    }
    let total: f64 = focus
        .iter()
        .map(|subject| {
            let seen = recent_appearances.get(subject).copied().unwrap_or(0);
            1.0 - (seen as f64 / window as f64).min(1.0)
        })
        .sum();
    total / focus.len() as f64
}

// ---------------------------------------------------------------- 选择

/// softmax 概率，`P(i) ∝ exp(scoreᵢ / T)`。
///
/// 先减去最大分数再取指数：`exp` 的自变量最多到 0，不会溢出。
/// 这不会改变分布——softmax 对整体平移不变。
pub fn probabilities(scores: &[f64], temperature: f64) -> Vec<f64> {
    if scores.is_empty() {
        return Vec::new();
    }
    if !temperature.is_finite() || temperature <= 0.0 {
        // 温度归零退化成「只取最高分」；文档的初始值是 0.15，不会走到这里，
        // 但让 0 有个明确含义比让 exp 除零好。
        let best = argmax(scores).unwrap_or(0);
        return (0..scores.len())
            .map(|i| if i == best { 1.0 } else { 0.0 })
            .collect();
    }
    let max = scores
        .iter()
        .copied()
        .filter(|s| s.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    let max = if max.is_finite() { max } else { 0.0 };
    let weights: Vec<f64> = scores
        .iter()
        .map(|score| {
            let score = if score.is_finite() { *score } else { max };
            ((score - max) / temperature).exp()
        })
        .collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return vec![1.0 / scores.len() as f64; scores.len()];
    }
    weights.into_iter().map(|w| w / total).collect()
}

/// 按 softmax 分布抽一个场景索引。
///
/// `die` 来自键为 `scene_select@<叙述序号>` 的命运骰子（docs/06 §6）——
/// 由调用方掷，这样本模块不依赖骰子类型。
pub fn select_index(scores: &[f64], temperature: f64, die: f64) -> Option<usize> {
    if scores.is_empty() {
        return None;
    }
    let probabilities = probabilities(scores, temperature);
    let mut accumulated = 0.0;
    let die = if die.is_finite() { die.clamp(0.0, 1.0) } else { 0.0 };
    for (index, probability) in probabilities.iter().enumerate() {
        accumulated += probability;
        if die < accumulated {
            return Some(index);
        }
    }
    // 浮点误差：落在最后一格之外时取最后一个，而不是报「选不出来」。
    Some(scores.len() - 1)
}

fn argmax(scores: &[f64]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, score) in scores.iter().enumerate() {
        if !score.is_finite() {
            continue;
        }
        match best {
            Some(current) if scores[current] >= *score => {}
            _ => best = Some(index),
        }
    }
    best.or(if scores.is_empty() { None } else { Some(0) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_domain::ThreadId;

    fn thread(pressure: f64) -> Thread {
        Thread {
            id: ThreadId::new("thr_1"),
            title: "顾言会不会离开".into(),
            question: "顾言最终会离开吗".into(),
            stakes: String::new(),
            subjects: vec![],
            stage: if_domain::ThreadStage::Developing,
            protected_until: None,
            pressure,
            last_advanced: 0,
            cadence: 3.0,
        }
    }

    #[test]
    fn weights_match_docs_06_s6() {
        for (style, expected) in [
            (
                "均衡",
                (1.0, 0.8, 0.6, 0.5, 0.4, 0.6, 0.8),
            ),
            ("慢热言情", (1.0, 0.4, 0.8, 0.3, 0.6, 0.8, 0.6)),
            ("悬疑", (1.0, 0.9, 0.7, 0.6, 0.3, 0.9, 0.9)),
            ("爽文", (0.8, 1.0, 0.5, 0.8, 0.2, 0.5, 0.7)),
        ] {
            let w = DirectorWeights::for_style(style);
            assert_eq!(
                (
                    w.fit, w.tension, w.thread, w.overdue, w.balance, w.tendency, w.repetition
                ),
                expected,
                "{style}"
            );
        }
        // 未知风格回落均衡，而不是 panic 或者零权重
        assert_eq!(DirectorWeights::for_style("随便写的"), DirectorWeights::BALANCED);
        assert_eq!(DirectorWeights::for_style(""), DirectorWeights::BALANCED);
    }

    #[test]
    fn score_is_a_weighted_sum_with_penalties_subtracted() {
        let w = DirectorWeights::BALANCED;
        let signals = SceneSignals {
            semantic_fit: 1.0,
            tension_gain: 0.5,
            thread_pressure: 0.5,
            overdue_bonus: 0.0,
            character_balance: 0.0,
            tendency_bonus: 0.0,
            repetition: 0.5,
            premature_resolution: 0.25,
        };
        let expected = 1.0 * 1.0 + 0.8 * 0.5 + 0.6 * 0.5 - 0.8 * 0.5 - 0.8 * 0.25;
        assert!((w.score(&signals) - expected).abs() < 1e-12);
    }

    #[test]
    fn every_term_is_normalized_before_weighting() {
        let w = DirectorWeights::BALANCED;
        // 越界与 NaN 都被夹紧，不能靠一个爆表的项压过其他项
        let wild = SceneSignals {
            semantic_fit: 99.0,
            tension_gain: f64::NAN,
            ..Default::default()
        };
        let clamped = SceneSignals {
            semantic_fit: 1.0,
            ..Default::default()
        };
        assert!((w.score(&wild) - w.score(&clamped)).abs() < 1e-12);
        // 负值也不能变成奖励
        let negative = SceneSignals {
            repetition: -5.0,
            ..Default::default()
        };
        assert_eq!(w.score(&negative), 0.0);
    }

    #[test]
    fn probabilities_sum_to_one() {
        for scores in [
            vec![1.0, 2.0, 3.0],
            vec![0.0, 0.0, 0.0],
            vec![-5.0, -5.0, -5.0],
            vec![1000.0, 1000.0, 1000.0],
        ] {
            let p = probabilities(&scores, DIRECTOR_TEMPERATURE);
            let total: f64 = p.iter().sum();
            assert!((total - 1.0).abs() < 1e-12, "{scores:?} → {p:?}");
            assert!(p.iter().all(|x| x.is_finite() && *x >= 0.0));
        }
        assert!(probabilities(&[], DIRECTOR_TEMPERATURE).is_empty());
    }

    #[test]
    fn large_scores_do_not_overflow() {
        let p = probabilities(&[1000.0, 999.0], DIRECTOR_TEMPERATURE);
        assert!(p[0] > p[1]);
        assert!(p[0].is_finite());
        // 1 分的差距在 T = 0.15 下几乎全给了第一项
        assert!(p[0] > 0.99);
    }

    #[test]
    fn low_temperature_approaches_argmax() {
        let scores = [0.0, 1.0, 0.5];
        let cold = probabilities(&scores, 0.001);
        assert!(cold[1] > 0.999, "{cold:?}");
        // 温度归零是可以表达的，含义是「只取最高分」
        assert_eq!(probabilities(&scores, 0.0), vec![0.0, 1.0, 0.0]);
        // 温度很高则接近均匀
        let hot = probabilities(&scores, 1000.0);
        assert!(hot.iter().all(|p| (p - 1.0 / 3.0).abs() < 1e-3), "{hot:?}");
    }

    #[test]
    fn selection_is_deterministic_and_covers_the_likely_band() {
        let scores = [0.0, 1.0, 0.5];
        let picked = |die: f64| select_index(&scores, DIRECTOR_TEMPERATURE, die).unwrap();
        assert_eq!(picked(0.3), picked(0.3));
        // 最高分占绝大部分概率质量，所以低位的 die 必然落在它身上
        assert_eq!(picked(0.5), 1);
        // 极端值的 die 也要有结果，不能返回 None
        assert!(select_index(&scores, DIRECTOR_TEMPERATURE, 1.0).is_some());
        assert!(select_index(&scores, DIRECTOR_TEMPERATURE, -3.0).is_some());
        assert!(select_index(&scores, DIRECTOR_TEMPERATURE, f64::NAN).is_some());
        assert_eq!(select_index(&[], DIRECTOR_TEMPERATURE, 0.5), None);
    }

    #[test]
    fn selection_spreads_across_scenes() {
        // 三个分数接近的场景：一个固定的 die 序列应当都能抽到
        let scores = [0.0, 0.01, 0.02];
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..300 {
            let die = i as f64 / 300.0;
            seen.insert(select_index(&scores, 0.15, die).unwrap());
        }
        assert_eq!(seen.len(), 3, "{seen:?}");
    }

    #[test]
    fn thread_pressure_averages_and_tolerates_empty() {
        let none: Vec<Thread> = Vec::new();
        assert_eq!(thread_pressure(none.iter()), 0.0);
        let threads = [thread(0.4), thread(0.8)];
        let value = thread_pressure(threads.iter());
        assert!((value - 0.6).abs() < 1e-9);
        // 非有限值按 0 计，不让 NaN 污染整项
        let broken = [thread(f64::NAN)];
        assert!((thread_pressure(broken.iter()) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn character_balance_measures_absence() {
        let gu = SubjectId::new("c_gu");
        let lin = SubjectId::new("c_lin");
        let none = BTreeMap::new();
        // 完全没出场 → 缺额 1
        assert!((character_balance(std::slice::from_ref(&gu), &none, 6) - 1.0).abs() < 1e-9);
        // 一直在场 → 缺额 0
        let recent = BTreeMap::from([(gu.clone(), 6_u64), (lin.clone(), 6_u64)]);
        assert!((character_balance(&[gu.clone(), lin.clone()], &recent, 6) - 0.0).abs() < 1e-9);
        // 顾言只出场一半（缺额 0.5）、林夏全程在场（缺额 0）→ 平均 0.25
        let half = BTreeMap::from([(gu.clone(), 3_u64), (lin.clone(), 6_u64)]);
        assert!((character_balance(&[gu.clone(), lin.clone()], &half, 6) - 0.25).abs() < 1e-9);
        assert!((character_balance(std::slice::from_ref(&gu), &half, 6) - 0.5).abs() < 1e-9);
        // 出场次数超过窗口也要夹到 1
        let over = BTreeMap::from([(gu.clone(), 99_u64)]);
        assert!((character_balance(&[gu], &over, 6) - 0.0).abs() < 1e-9);
        // 没有焦点角色 / 窗口为 0 → 0
        assert_eq!(character_balance(&[], &half, 6), 0.0);
        assert_eq!(character_balance(&[lin], &half, 0), 0.0);
    }
}
