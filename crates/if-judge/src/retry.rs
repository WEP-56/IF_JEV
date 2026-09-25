use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::{Judge, JudgeBackend, JudgeError, JudgeRequest, JudgeResponse};

/// docs/06 §9：失败后重试 2 次，指数退避【初始值】。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub retries: u32,
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy { retries: 2, base_delay: Duration::from_millis(500) }
    }
}

/// 只对瞬时失败重试；400、鉴权、格式异常、取消直接返回。
/// 重试耗尽后返回最后一次的错误，由回合流程暂停并提示用户（D11）。
#[derive(Debug)]
pub struct Retrying<J> {
    inner: J,
    policy: RetryPolicy,
}

impl<J: Judge> Retrying<J> {
    pub fn new(inner: J, policy: RetryPolicy) -> Self {
        Retrying { inner, policy }
    }

    pub fn inner(&self) -> &J {
        &self.inner
    }
}

impl<J: Judge> Judge for Retrying<J> {
    fn judge(&self, request: &JudgeRequest, cancel: &AtomicBool) -> Result<JudgeResponse, JudgeError> {
        let mut attempt = 0;
        loop {
            match self.inner.judge(request, cancel) {
                Err(e) if e.is_transient() && attempt < self.policy.retries => {
                    let delay = self.policy.base_delay * 2u32.pow(attempt);
                    attempt += 1;
                    if sleep_unless_cancelled(delay, cancel) {
                        return Err(JudgeError::Cancelled);
                    }
                }
                other => return other,
            }
        }
    }

    fn backend(&self) -> JudgeBackend {
        self.inner.backend()
    }
}

/// 分片睡眠，便于及时响应取消。返回 true 表示已取消。
fn sleep_unless_cancelled(total: Duration, cancel: &AtomicBool) -> bool {
    let step = Duration::from_millis(50);
    let mut left = total;
    while !left.is_zero() {
        if cancel.load(Ordering::Relaxed) {
            return true;
        }
        let d = left.min(step);
        std::thread::sleep(d);
        left -= d;
    }
    cancel.load(Ordering::Relaxed)
}
