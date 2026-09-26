//! # if-pipeline
//!
//! IF 的**回合编排**。文档意义上的 runner：它把已经就位的两层——
//! 视图编译（[`if_views`]）与裁决策略（[`if_policy`]）——接进回合流程（docs/04），
//! 并产出可以写进事件日志的补丁。
//!
//! 这个 crate 刻意**不认识 LLM，也不认识存储**：
//!
//! - 需要模型的地方，只向 [`if_judge::Judge`] 发问（视图 + 问题）；
//!   真实 Jev、LLM 裁判、测试桩在它眼里是同一个东西。
//! - 需要叙事文本的地方，接收**提议**（候选、场景、节拍），而不是自己生成；
//!   从模型那里拿到提议是 agent 任务的事（docs/05），本 crate 只负责
//!   「提议进来 → 判定出去 → 补丁出去」。
//! - 产出的是一批 [`if_domain::EventDraft`]，写不写、写去哪由调用方（`if-app` 的
//!   世界工作线程）决定。
//!
//! 于是整个回合可以在**没有网络、没有数据库**的情况下被端到端测试：
//! 测试桩给出固定概率，提议直接构造，断言落在事件序列与投影上。
//! 这正是 docs/12 §9「回合流程测试：Judge 测试桩 + 模拟的 provider」要的东西。
//!
//! ## 各模块在回合里的位置
//!
//! ```text
//! 提议（来自 agent 任务）          本 crate                    产物
//! ────────────────────────────────────────────────────────────────────
//! T-impact 的候选      →  candidates  约束门 + 分层裁决      accepted / rejected
//! T-scenes 的场景      →  scenes     导演评分 + 骰子抽取     选中的场景
//! T-plan 的场景计划     →  （类型来自 if-domain）            ScenePlan
//! 激活的设定条目        →  lore       世界书激活             Vec<LoreEntry>
//! T-render 的正文      →  beats      逐节拍检查放行          放行的节拍
//! 以上全部             →  commit     裁决结果 → 事件补丁      Vec<EventDraft>
//! ```
//!
//! [`turn`] 把上面这些串成一个回合；其余模块各自只做一件事，可以单独测。
//!
//! **当前进度（别把它当成已完成）**：只有 [`lore`] 与 [`question`] 两块落地了，
//! 其余四个模块（`candidates` / `scenes` / `beats` / `commit` / `turn`）**尚未创建**。
//! 也就是说这个 crate 现在还**跑不出一个回合**，它提供的是「世界书激活」与
//! 「问题构造」两个零件。上面那张表是目标形状，不是现状。
//!
//! ## 两条不可退让的性质
//!
//! 1. **约束类永不掷骰**（docs/06 §1）。合规检查与「是否符合人设」只比阈值，
//!    骰子只在发生类与互斥类上出现。
//! 2. **同一输入同一结果**（docs/12 §7）。所有集合用 `BTreeMap` / 有序 `Vec`，
//!    骰子由世界种子驱动，不读时钟、不读随机数。

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod lore;
pub mod question;

pub use lore::{activate_lore, LoreSignals};
pub use question::{template_id, TEMPLATES};

/// 回合编排中可能出现的失败。
///
/// 大部分失败不该终止回合，而是降级并记进 `warnings`（docs/06 §9）：
/// 判定缺失按「约束类不通过、发生类不发生」处理，场景选不出来就跳过本回合。
/// 这里只承载那些**编不下去了**的情况。
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("判定请求失败：{0}")]
    Judge(#[from] if_judge::JudgeError),
    #[error("候选分层失败：{0}")]
    Layer(#[from] if_policy::LayerError),
    #[error("场景计划非法：{0}")]
    ScenePlan(#[from] if_domain::ScenePlanError),
    #[error("回合编排失败：{0}")]
    Invalid(String),
}

impl PipelineError {
    pub fn invalid(message: impl Into<String>) -> Self {
        PipelineError::Invalid(message.into())
    }
}

/// 本 crate 假设的模板版本后缀。
///
/// docs/07 §3 要求「措辞或标准一有改动就升版本」，判定记录记的是 `模板 ID@版本`。
/// 版本常量集中在这里，改动时一处可见。
pub const TEMPLATE_VERSION: &str = "1";
