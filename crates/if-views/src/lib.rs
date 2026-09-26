//! # if-views
//!
//! 把世界投影编译成**某个消费方有资格看到的形式**，并给出稳定指纹。
//!
//! 这是 P9（视角隔离）的落点。判断角色行为时，不能让判定者看到角色不知道的事实；
//! 写正文时，不能让叙述者说出玩家无权知道的内容。两条约束都靠这里保证：
//!
//! - [`visibility`]：谁有权看到哪条事实。三档可见性（public / private / secret）
//!   加上 L1 保护期，规则集中在一处。
//! - [`compile`]：按视图种类装配状态，再按预算裁剪（docs/08 §3）。
//! - [`hash`]：视图指纹与命运骰子共用的确定性原语。判定记录靠指纹追溯
//!   「这个结论是在什么样的信息下得出的」（docs/07 §5）。
//!
//! 依赖 `if-judge` 只为一件事：`CompiledView` 的容器类型定义在那里。
//! 评测请求把「视图 + 问题」装在一起，所以视图编译的产物必须先满足它的形状。

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod compile;
pub mod hash;
pub mod state;
pub mod visibility;

pub use compile::{compile, now, ViewRequest};
pub use hash::{digest_hex, digest_unit, view_hash, VIEW_HASH_PREFIX};
pub use state::{
    BeliefLine, BeatLine, DirectorLine, FactLine, KnowledgeBoundary, LoreLine, RuleLine,
    ScenePlanLine, SubjectLine, TendencyLine, ThreadLine, ViewState, DEFAULT_VIEW_BUDGET,
};
pub use visibility::{
    fact_is_visible, knowledge_of, player_knowledge, secret_facts, unjustified_basis,
};
