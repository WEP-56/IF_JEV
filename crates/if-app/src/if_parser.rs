//! Deterministic first-pass IF parser.
//! It intentionally does not claim semantic understanding; the next stage is T-parse via LLM/Jev.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParsedIfKind {
    State,
    Belief,
    Rule,
    Occurrence,
    Truth,
    Retcon,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParsedTimeAnchor {
    Now,
    Past,
    Always,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IfDraft {
    pub input: String,
    pub is_directive: bool,
    pub normalized: String,
    pub kind: ParsedIfKind,
    pub time_anchor: ParsedTimeAnchor,
    pub scope: String,
    pub suggested_lock: String,
    pub core: String,
    pub non_commitments: Vec<String>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub rewrite_candidates: Vec<String>,
}

pub fn parse(input: &str) -> Result<IfDraft, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("IF 输入不能为空".into());
    }
    let is_directive = is_directive(input);
    let normalized = strip_if_prefix(input).to_owned();
    let kind = classify(&normalized, is_directive);
    let time_anchor = time_anchor(&normalized);
    let scope = if ["所有人", "整个", "全世界", "全城", "世界"].iter().any(|word| normalized.contains(word)) {
        "global"
    } else {
        "individual_or_local"
    };
    let suggested_lock = match kind {
        ParsedIfKind::Rule => "L2",
        ParsedIfKind::Occurrence | ParsedIfKind::Truth | ParsedIfKind::Retcon => "L3",
        ParsedIfKind::State | ParsedIfKind::Belief => "L1",
        ParsedIfKind::Unknown => "L0",
    };
    let mut warnings = Vec::new();
    if is_directive {
        warnings.push("这句话可能包含导演意图；当前先按候选 IF 进入裁定卡，由结构模型和 Jev 继续判断。".into());
    }
    if matches!(kind, ParsedIfKind::Unknown) {
        warnings.push("暂时无法从措辞确定 IF 类型，需要 T-parse 复核。".into());
    }
    if !input.starts_with("IF ") && !input.starts_with("IF：") && !input.starts_with("IF:") {
        warnings.push("输入未使用 IF 前缀；当前仅做启发式解析。".into());
    }
    let rewrite_candidates = if is_directive { rewrite_candidates(&normalized) } else { Vec::new() };
    Ok(IfDraft {
        input: input.to_owned(),
        is_directive,
        normalized: format!("IF {normalized}"),
        kind,
        time_anchor,
        scope: scope.into(),
        suggested_lock: suggested_lock.into(),
        core: normalized,
        non_commitments: vec![
            "角色是否意识到该事实".into(),
            "后续行动与结果".into(),
            "其他主体是否知情".into(),
        ],
        warnings,
        rewrite_candidates,
    })
}

fn rewrite_candidates(text: &str) -> Vec<String> {
    let subject = text
        .trim_start_matches("请")
        .trim_start_matches("让")
        .trim_start_matches("使")
        .trim();
    let (who, action) = subject.split_once(' ')
        .or_else(|| subject.split_once('　'))
        .unwrap_or((subject, "产生变化"));
    vec![
        format!("IF {who}已经决定{action}"),
        format!("IF {who}产生了{action}的意图"),
        format!("IF {who}最近开始考虑{action}"),
    ]
}

fn is_directive(input: &str) -> bool {
    ["让", "请让", "时间快进", "快进", "推进", "继续", "跳过", "安排", "使"].iter().any(|prefix| input.starts_with(prefix))
}

fn strip_if_prefix(input: &str) -> &str {
    input.strip_prefix("IF ").or_else(|| input.strip_prefix("IF：")).or_else(|| input.strip_prefix("IF:")).unwrap_or(input)
}

fn classify(text: &str, directive: bool) -> ParsedIfKind {
    if directive {
        return ParsedIfKind::Unknown;
    }
    if text.contains("无法说谎") || text.contains("不得") || text.contains("规则") || text.contains("持续") {
        return ParsedIfKind::Rule;
    }
    if text.contains("相信") || text.contains("认为") || text.contains("以为") {
        return ParsedIfKind::Belief;
    }
    if text.contains("从未") || text.contains("其实一直") || text.contains("早已") {
        return ParsedIfKind::Retcon;
    }
    if text.contains("一直") || text.contains("真相") || text.contains("原来") {
        return ParsedIfKind::Truth;
    }
    if text.contains("发生") || text.contains("失踪") || text.contains("停止") || text.contains("已经") || text.contains("刚刚") {
        return ParsedIfKind::Occurrence;
    }
    if text.contains("爱") || text.contains("喜欢") || text.contains("关系") || text.contains("状态") || text.contains("决定") {
        return ParsedIfKind::State;
    }
    ParsedIfKind::Unknown
}

fn time_anchor(text: &str) -> ParsedTimeAnchor {
    if text.contains("一直") || text.contains("从未") || text.contains("始终") {
        ParsedTimeAnchor::Always
    } else if text.contains("已经") || text.contains("刚刚") || text.contains("早已") || text.contains("曾经") {
        ParsedTimeAnchor::Past
    } else {
        ParsedTimeAnchor::Now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rule_with_global_scope() {
        let draft = parse("IF 所有人从此无法说谎").unwrap();
        assert_eq!(draft.kind, ParsedIfKind::Rule);
        assert_eq!(draft.time_anchor, ParsedTimeAnchor::Now);
        assert_eq!(draft.scope, "global");
        assert_eq!(draft.suggested_lock, "L2");
    }

    #[test]
    fn detects_directive_and_does_not_confirm_it() {
        let draft = parse("让林夏表白").unwrap();
        assert!(draft.is_directive);
        assert_eq!(draft.kind, ParsedIfKind::Unknown);
        assert!(!draft.warnings.is_empty());
        assert_eq!(draft.rewrite_candidates.len(), 3);
    }

    #[test]
    fn distinguishes_past_truth_from_current_state() {
        let past = parse("IF 王宫刚刚起火了").unwrap();
        assert_eq!(past.kind, ParsedIfKind::Occurrence);
        assert_eq!(past.time_anchor, ParsedTimeAnchor::Past);
        let truth = parse("IF 骑士一直是失踪的王储").unwrap();
        assert_eq!(truth.kind, ParsedIfKind::Truth);
        assert_eq!(truth.time_anchor, ParsedTimeAnchor::Always);
    }
}
