//! # if-judge
//!
//! 统一的判定接口（P14，docs/12 §3）。一个请求 = 一个视图 + 多个问题（docs/07 §2 R1）。
//!
//! - [`jev::JevJudge`]：`POST /api/alpha/decisions`（docs/14）。
//! - [`llm::LlmJudge`]：Jev 不可用时的降级后端（D11），输出与 Jev 同形。
//! - [`stub::StubJudge`]：按脚本返回固定判定，用于回合流程测试。
//! - [`retry::Retrying`]：只对瞬时失败做指数退避重试（docs/06 §9）。

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod error;
pub mod jev;
pub mod llm;
pub mod question;
pub mod request;
pub mod retry;
pub mod stub;

use std::sync::atomic::AtomicBool;

pub use error::JudgeError;
pub use jev::{JevConfig, JevJudge};
pub use llm::LlmJudge;
pub use question::{ChoiceCriteria, Question, QuestionSpec, ScoreCriteria};
pub use request::{CompiledView, JudgeBackend, JudgeRequest, JudgeResponse};
pub use retry::{RetryPolicy, Retrying};
pub use stub::StubJudge;

/// 判定后端。同步接口，与 Onemore 的线程模型一致；多个视图用 scoped threads 并行。
pub trait Judge: Send + Sync {
    fn judge(&self, request: &JudgeRequest, cancel: &AtomicBool) -> Result<JudgeResponse, JudgeError>;
    fn backend(&self) -> JudgeBackend;
}

impl<J: Judge + ?Sized> Judge for Box<J> {
    fn judge(&self, request: &JudgeRequest, cancel: &AtomicBool) -> Result<JudgeResponse, JudgeError> {
        (**self).judge(request, cancel)
    }
    fn backend(&self) -> JudgeBackend {
        (**self).backend()
    }
}

impl<J: Judge + ?Sized> Judge for std::sync::Arc<J> {
    fn judge(&self, request: &JudgeRequest, cancel: &AtomicBool) -> Result<JudgeResponse, JudgeError> {
        (**self).judge(request, cancel)
    }
    fn backend(&self) -> JudgeBackend {
        (**self).backend()
    }
}
