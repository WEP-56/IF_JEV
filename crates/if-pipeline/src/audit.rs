//! 判定记录的序号游标（docs/03 §3、docs/07 §5）。
//!
//! 判定记录**不进事件日志**——它进 [`if_domain::turn::TurnRecord`]。这让审计信息
//! 不必污染事件流，但也带来一个不显眼的要求：**同一个回合里的判定 ID 必须唯一**。
//!
//! 一个回合要经过三段判定：影响候选的约束门与发生类（T-impact）、场景选择（T-scenes）、
//! 逐节拍放行（T-render）。如果每段都从 `jdg_0001` 重新开始，那么
//! `Beat::judgments` 里存的 `jdg_0001` 在 `TurnRecord::judgments` 里会**同时命中**
//! 影响裁决的第一条与节拍检查的第一条——「这条节拍是凭什么被放行的」就会查到别的记录。
//!
//! 所以游标由回合持有、三段共用：
//!
//! ```text
//! open()    ← 新建游标：影响裁决 → 场景选择        → Opening.audit
//! resolve() ← 接着那条游标：逐节拍检查             → SceneOutcome
//! ```
//!
//! [`crate::turn::resolve`] 会自己从 [`crate::turn::Opening`] 上取走游标，
//! 调用方不需要（也不应该）自己拼一个——拼错就是又一轮 ID 重复。
//!
//! 它只管**发号**，记录由各段自己收（`ImpactOutcome.judgments` /
//! `SceneChoice.judgments` / `BeatRound.judgments`），这样每段的产物仍然是自足的。

use if_domain::id::JudgmentId;

/// 回合内唯一的判定序号游标。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Audit {
    next: u64,
}

impl Audit {
    /// 一个新回合的游标。
    pub fn new() -> Self {
        Self::default()
    }

    /// 取下一条判定记录的 ID。
    pub fn take(&mut self) -> JudgmentId {
        self.next += 1;
        JudgmentId::numbered(self.next)
    }

    /// 已经发出多少条。第二段接第一段时用它自查。
    pub fn issued(&self) -> u64 {
        self.next
    }
}
