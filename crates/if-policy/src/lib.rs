//! # if-policy
//!
//! IF 的**确定性裁决策略**层。Jev 只输出分布，这个 crate 决定分布怎么变成结果。
//!
//! 四件事在这里落地，都不碰网络、不碰存储，因此可以完全单元测试：
//!
//! - [`threshold`]：阈值表（docs/06 §2）。约束类与合规检查只比阈值，**永不掷骰**。
//! - [`dice`]：命运骰子（docs/03 §8）。`u = H(seed ‖ decision_key ‖ salt)`，
//!   决策键跨世界线稳定，所以同一个决定在两条世界线上用同一颗骰子。
//! - [`layers`] + [`adjudicate`]：分层裁决（docs/06 §4）。同一请求里的问题虽然独立
//!   评估，但不能各自独立掷骰——那会掷出「保护 ✗、回避 ✗、察觉异常 ✓」这种不自洽的
//!   组合。所以按依赖分层，逐层在更新后的临时状态上判定；被否决但落在观察带内的
//!   候选转成趋势，而不是消失。
//! - [`director`]：导演评分与场景选择（docs/06 §6）。风格 = 一组权重，
//!   选择按 `P(i) ∝ exp(scoreᵢ / T)`，所以既不会永远取最高分，也不是纯随机。
//!
//! 依赖 `if-views` 只为一件事：命运骰子的哈希原语（[`if_views::digest_unit`]）。
//! 骰子与视图指纹必须共用同一套确定性实现，否则「同一输入同一输出」这条性质
//! 会被拆成两份去维护，迟早对不上。
//!
//! 上面一层（组装回合流程的 `if-pipeline`）才是把这里接进 agent 循环的地方；
//! 本 crate 不认识 LLM，也不认识事件日志。

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod adjudicate;
pub mod dice;
pub mod director;
pub mod layers;
pub mod threshold;

pub use adjudicate::{Adjudication, AdjudicationInput, LayerOutcome, Policy};
pub use dice::{
    candidate_key, exclusive_key, lore_probe_key, mechanic_key, scene_select_key, Dice, NO_SALT,
};
pub use director::{
    character_balance, probabilities, select_index, thread_pressure, DirectorWeights, SceneSignals,
    DIRECTOR_TEMPERATURE,
};
pub use layers::{layer, LayerError, Layered, MAX_CAUSAL_DEPTH};
pub use threshold::{Strictness, ThresholdSense, ThresholdSpec, Thresholds, STRICTNESS_MARGIN};
