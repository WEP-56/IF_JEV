//! T-render：正文与节拍切分（docs/04 第 11 步、docs/05 §7）。
//!
//! **正文走普通文本输出，不走工具参数**（docs/05 §7 已确认）。长段文学文本塞进
//! JSON 参数里质量不可控，所以模型每写完一个节拍输出一行分隔标记，host 这边切。
//!
//! 这一段与其余四个任务不同的地方：
//!
//! - 没有工具，所以不走 agent loop，直接 [`Provider::stream_turn`]；
//! - 切出来的节拍只是**提议**，能不能上屏要过 [`crate::beats::run`]；
//! - 背压（docs/04 §4.7「未裁决的节拍超过上限就暂停读取」）**不在这里**——
//!   那是流式读取侧的编排，而本模块的输入就是一只已经收完的手。
//!   这里做的是「收完再切」，与 [`crate::beats`] 只看已切好的片段的边界一致。

use std::sync::atomic::AtomicBool;

use if_agent::provider::{Provider, StreamTerminal};
use if_agent::{ChatMessage, PromptContext};
use if_domain::narrative::{ScenePlan, MAX_BEATS_PER_SCENE};
use if_domain::projection::Projection;
use if_domain::rule::{NarrativePov, WorldSettings};
use if_domain::turn::ViewKind;

use crate::beats::BeatProposal;
use crate::context::TurnContext;

use super::{view_block, view_json, TaskError};

/// 节拍之间的分隔标记（docs/05 §7）。
pub const BEAT_MARKER: &str = "<<<BEAT>>>";

/// 任务代号。T-render 没有工具，所以它不在 [`super::TaskKind`] 里——
/// 那个枚举是「有提议工具的任务」的目录。
pub const TASK: &str = "T-render";

/// 一次渲染的产物。
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub beats: Vec<BeatProposal>,
    pub warnings: Vec<String>,
}

/// 把模型输出的整段正文切成节拍。
///
/// 位置映射是**确定性**的：第 i 个节拍对应 `plan.required_beats[i-1]`，
/// 因此前 `required_beats.len()` 个节拍是「必需的」。这一条不完美——
/// 模型完全可以多写一个铺垫节拍把位置错开——但它是可复现的、不需要再问一次模型，
/// 而且失手的后果只是「某个非必需节拍被当成必需」（最多让场景早一点收束），
/// 不会让正文违规上屏。把判据交给模型反而会多一类不稳定。
pub fn beats_from_text(text: &str, plan: &ScenePlan, max_beats: u32) -> Rendered {
    let mut warnings = Vec::new();
    let mut beats: Vec<BeatProposal> = Vec::new();
    let mut dropped = 0usize;

    for part in text.split(BEAT_MARKER) {
        let body = part.trim();
        if body.is_empty() {
            continue;
        }
        if beats.len() as u32 >= max_beats {
            dropped += 1;
            continue;
        }
        let index = beats.len() as u32 + 1;
        let mut beat = BeatProposal::new(index, body);
        if index as usize <= plan.required_beats.len() {
            beat = beat.plan_beat(index as usize - 1).required();
        }
        beats.push(beat);
    }

    if dropped > 0 {
        warnings.push(format!(
            "正文超出每场 {max_beats} 个节拍的上限，多出的 {dropped} 个片段已丢弃"
        ));
    }
    if beats.is_empty() {
        warnings.push("模型没有写出任何正文".to_owned());
    }
    Rendered { beats, warnings }
}

/// 跑一次 T-render：发一次问、收完整段正文、切成节拍。
///
/// `emit` 拿到的是**原文增量**（`AgentEvent::AssistantDelta`），供宿主做进度展示；
/// 它推出去的东西在过完 [`crate::beats::run`] 之前都不算数
/// （docs/05 §7：先不展示，检查通过才放行）。
pub fn run(
    provider: &dyn Provider,
    projection: &Projection,
    settings: &WorldSettings,
    ctx: &TurnContext,
    plan: &ScenePlan,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(if_agent::AgentEvent),
) -> Result<Rendered, TaskError> {
    let mut on_delta = |delta: &str| emit(if_agent::AgentEvent::AssistantDelta(delta.to_owned()));
    render_beats(
        provider,
        projection,
        settings,
        ctx,
        plan,
        cancel,
        &mut on_delta,
    )
}

/// 真正干活的那一层：只认 `Provider` 与一个增量回调，便于测试直接喂一段脚本输出。
pub fn render_beats(
    provider: &dyn Provider,
    projection: &Projection,
    settings: &WorldSettings,
    ctx: &TurnContext,
    plan: &ScenePlan,
    cancel: &AtomicBool,
    on_delta: &mut dyn FnMut(&str),
) -> Result<Rendered, TaskError> {
    let (system, user) = prompt(projection, settings, ctx, plan);
    let prompt = PromptContext {
        system_sections: vec![system],
        messages: vec![ChatMessage::user_text(user)],
    };

    let mut text = String::new();
    let mut collect = |event: if_agent::ProviderEvent| {
        if let if_agent::ProviderEvent::TextDelta(delta) = &event {
            text.push_str(delta);
            on_delta(delta);
        }
    };
    let output = match provider.stream_turn(&prompt, &[], &mut collect, cancel) {
        StreamTerminal::Done(output) => output,
        StreamTerminal::Aborted(_) => return Err(TaskError::Cancelled { task: TASK }),
        StreamTerminal::Error(failed) => {
            return Err(TaskError::Provider {
                task: TASK,
                reason: failed.error.message,
            })
        }
    };
    // 流式增量与最终消息理论上一致；不一致时以最终消息为准——
    // 那是模型真正说的话，增量的拼接只是网络分的片。
    let final_text = output.message.text();
    if !final_text.is_empty() {
        text = final_text;
    }

    Ok(beats_from_text(&text, plan, MAX_BEATS_PER_SCENE))
}

/// 组装 T-render 的提示词：叙事视图 + 世界设置里的文风与视角 + 分隔标记约定。
pub fn prompt(
    projection: &Projection,
    settings: &WorldSettings,
    ctx: &TurnContext,
    plan: &ScenePlan,
) -> (String, String) {
    let pov = match settings.narrative_pov {
        NarrativePov::ThirdLimited => "第三人称有限视角：只写视角人物能看到、能想到的，不要越过他的认知",
        NarrativePov::FirstPerson => "第一人称：由视角人物自述",
        NarrativePov::Omniscient => "全知视角：可以叙述视角人物之外的事",
    };
    let style = if settings.narrative_style.trim().is_empty() {
        "文学叙事".to_owned()
    } else {
        settings.narrative_style.trim().to_owned()
    };

    let system = format!(
        "你是这个世界的叙述者。\n\
         文风：{style}。\n\
         视角：{pov}。\n\
         \n\
         规则：\n\
         - 直接写正文。不要解释、不要总结、不要询问。\n\
         - 每写完一个节拍，单独输出一行 {BEAT_MARKER}，再继续写下一个。\n\
         - 节拍之间是同一场戏的连续推进，不是互不相关的片段。\n\
         - 一个场景最多 {MAX_BEATS_PER_SCENE} 个节拍。\n\
         - 只写视角人物有权知道的事；不许暗示尚未揭示的秘密。\n\
         - 到了停止条件就收笔，不要硬拖。"
    );

    let mut head = format!("这一场戏的目标：{}\n", plan.goal);
    if !plan.required_beats.is_empty() {
        head.push_str("必须写到的节拍（按顺序）：\n");
        for (ordinal, beat) in plan.required_beats.iter().enumerate() {
            head.push_str(&format!("{}. {beat}\n", ordinal + 1));
        }
    }
    head.push_str(&format!("停止条件：{}\n", plan.stop_condition));
    if !plan.forbidden_resolutions.is_empty() {
        head.push_str("**不许出现的结果**：\n");
        for forbidden in &plan.forbidden_resolutions {
            head.push_str(&format!("- {forbidden}\n"));
        }
    }

    let json = view_json(
        projection,
        &ctx.view(ViewKind::Narration).task(TASK).scene_plan(plan.clone()),
    );
    (system, format!("{head}\n{}", view_block("叙事视图", &json)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> ScenePlan {
        crate::testsupport::plan("顾言说出他知道的事")
    }

    #[test]
    fn the_marker_splits_beats_and_the_plan_beats_land_on_the_first_ones() {
        let text = "第一拍。\n<<<BEAT>>>\n第二拍。\n<<<BEAT>>>\n第三拍。";
        let rendered = beats_from_text(text, &plan(), MAX_BEATS_PER_SCENE);
        assert_eq!(rendered.beats.len(), 3);
        assert_eq!(rendered.beats[0].text, "第一拍。");
        assert_eq!(rendered.beats[2].text, "第三拍。");
        // 夹具的计划有两条 required_beats：前两拍是必需的，第三拍不是。
        assert!(rendered.beats[0].required && rendered.beats[0].plan_beat == Some(0));
        assert!(rendered.beats[1].required && rendered.beats[1].plan_beat == Some(1));
        assert!(!rendered.beats[2].required && rendered.beats[2].plan_beat.is_none());
        assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);
    }

    /// 模型漏写标记时，正文只能算一个节拍——总比整段丢掉好，
    /// 而且它会照常走节拍检查（docs/05 §7：标记不合规按「这个节拍不合规」处理）。
    #[test]
    fn text_without_any_marker_is_one_beat() {
        let rendered = beats_from_text("整段话，没有标记。", &plan(), MAX_BEATS_PER_SCENE);
        assert_eq!(rendered.beats.len(), 1);
        assert_eq!(rendered.beats[0].index, 1);
    }

    #[test]
    fn extra_beats_beyond_the_cap_are_dropped_with_a_warning() {
        let text = (1..=9).map(|i| format!("第{i}拍。")).collect::<Vec<_>>().join(BEAT_MARKER);
        let rendered = beats_from_text(&text, &plan(), MAX_BEATS_PER_SCENE);
        assert_eq!(rendered.beats.len(), MAX_BEATS_PER_SCENE as usize);
        assert!(rendered.warnings.join("\n").contains("上限"), "{:?}", rendered.warnings);
    }

    #[test]
    fn an_empty_scene_says_so_instead_of_pretending_it_wrote_something() {
        let rendered = beats_from_text("   \n<<<BEAT>>>\n  ", &plan(), MAX_BEATS_PER_SCENE);
        assert!(rendered.beats.is_empty());
        assert!(rendered.warnings.join("\n").contains("没有写出任何正文"));
    }
}
