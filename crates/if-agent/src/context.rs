use crate::message::ChatMessage;

/// 一次模型调用的输入。IF 中由视图编译产出：稳定前缀放在 `system_sections` 前部，
/// 便于 prompt cache 命中（docs/08）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PromptContext {
    pub system_sections: Vec<String>,
    pub messages: Vec<ChatMessage>,
}

impl PromptContext {
    pub fn system_text(&self) -> String {
        self.system_sections.join("\n\n")
    }
}
