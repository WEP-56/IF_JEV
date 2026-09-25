use thiserror::Error;

/// 失败分类决定是否重试（docs/14 §7.7）：只有 [`JudgeError::is_transient`] 为真的才退避重试。
#[derive(Debug, Error)]
pub enum JudgeError {
    /// 请求不合规：本地预校验失败或后端返回 400。是代码缺陷，不重试。
    /// `path` 形如 `questions.<id>.criteria`，可以对齐到问题模板。
    #[error("判定请求不合规（{path}）：{message}")]
    Invalid { path: String, message: String },
    /// 鉴权失败（401 / 403）。需要用户处理密钥，不重试。
    #[error("判定后端鉴权失败（HTTP {status}）：{message}")]
    Auth { status: u16, message: String },
    /// 5xx、429、超时、连接失败。
    #[error("判定后端暂时不可用：{0}")]
    Transient(String),
    /// 响应无法解析。
    #[error("判定响应格式异常：{0}")]
    Malformed(String),
    /// 后端返回了不可重试的其他错误（例如 LLM 裁判的模型配置有误）。
    #[error("判定后端错误：{0}")]
    Backend(String),
    #[error("判定已取消")]
    Cancelled,
}

impl JudgeError {
    pub fn is_transient(&self) -> bool {
        matches!(self, JudgeError::Transient(_))
    }

    pub(crate) fn invalid(path: impl Into<String>, message: impl Into<String>) -> Self {
        JudgeError::Invalid { path: path.into(), message: message.into() }
    }
}
