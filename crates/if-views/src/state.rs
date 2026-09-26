//! 视图状态的形态与预算裁剪。
//!
//! 视图状态的唯一消费者是判定者（Jev）与 LLM，所以它必须是**文本可读**的：
//! 全部字段在序列化后都是字符串或字符串数组，不塞结构体让模型去猜。
//!
//! 裁剪按 docs/08 §3 的优先级：规则 > 焦点状态 > 故事线 > 设定条目 > 历史。
//! 优先级高的先占预算，低的后拿剩下的；拿不到就整段丢弃，而不是截断成半句话。

use if_domain::id::SubjectId;
use serde::{Deserialize, Serialize};

/// 默认的视图字符预算。Onemore 单个工具结果是 24,000 字符，
/// 视图作为 prompt 前缀要留出对话与工具结果的空间，所以取一半。
pub const DEFAULT_VIEW_BUDGET: usize = 12_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectLine {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub tier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactLine {
    pub prop: String,
    pub key: String,
    pub text: String,
    pub value: String,
    pub lock: String,
    /// `Lock` 的数值等级（L0=0 … L3=3）。裁剪时靠它决定先丢哪条。
    pub lock_rank: u8,
    /// 是否在内层（感情、意图）。对账规则按它决定能否只靠预演提交（docs/02 §11）。
    pub internal: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeliefLine {
    pub holder: String,
    pub prop: String,
    pub text: String,
    pub value: String,
    pub belief: f64,
    pub certainty: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleLine {
    pub id: String,
    pub text: String,
    pub lock: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadLine {
    pub id: String,
    pub title: String,
    pub question: String,
    pub stage: String,
    pub pressure: f64,
    /// 受保护的故事线：它的解决必须被否决（docs/04 §2 第 8 步）。
    pub protected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TendencyLine {
    pub id: String,
    pub text: String,
    pub pressure: f64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoreLine {
    pub id: String,
    pub title: String,
    pub section: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeatLine {
    pub narrative_order: u64,
    pub scene: String,
    pub text: String,
}

/// 某个在场角色知道哪些命题——`q.beat.knowledge_leak` 的检查范围（docs/04 §4）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeBoundary {
    pub holder: String,
    pub name: String,
    /// 该角色知道的命题键。
    pub knows: Vec<String>,
    /// 该角色**不知道**、但本场景的相关命题里存在的键。这是泄露检查的重点。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unaware_of: Vec<String>,
}

/// 编译后的视图状态。序列化成 JSON 后交给判定者或 LLM。
///
/// 字段全部 `skip_serializing_if` 空的，这样不同视图的指纹只反映它真正包含的内容——
/// 否则一个空的故事线数组会让两个语义不同的视图算出同一个 hash。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    pub view: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    pub world_time: String,
    pub narrative_order: u64,
    pub scene_index: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub task: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<SubjectLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<FactLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beliefs: Vec<BeliefLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<ThreadLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tendencies: Vec<TendencyLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lore: Vec<LoreLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_beats: Vec<BeatLine>,

    // ---- 检查视图 / 叙事视图专用 ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_plan: Option<ScenePlanLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_boundaries: Vec<KnowledgeBoundary>,
    /// 本场景允许揭示的事实。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reveal_allowed: Vec<String>,
    /// 未批准揭示的秘密。**只进检查视图，不进叙事视图**（docs/04 §2.1）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unapproved_secrets: Vec<String>,
    /// 待检查的节拍原文。只有检查视图有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat_text: Option<String>,

    // ---- 解析视图 / 导演视图专用 ----
    /// 用户输入原文。解析视图要拿它判 `q.input.is_directive`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_input: Option<String>,
    /// 引擎注入的导演信号（docs/08 §1 导演视图）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub director: Option<DirectorLine>,
}

/// 引擎给导演的调度信号。这些不由 LLM 提供，是引擎算出来的。
///
/// 字段**不做空值省略**：`overdue_threads: []`（没有逾期）与「根本没算」是两回事，
/// 导演要能分辨。这个块很小，无条件输出不影响指纹的判别力。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectorLine {
    /// 已逾期、该推进的故事线（`id: 标题`）。
    pub overdue_threads: Vec<String>,
    /// 离开焦点太久的主体。
    pub absent_subjects: Vec<String>,
    /// 达到爆发阈值的趋势。
    pub erupting_tendencies: Vec<String>,
    /// 已有场景数。
    pub scene_count: u64,
}

/// 场景计划在视图里的呈现（docs/02 §10）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePlanLine {
    pub goal: String,
    pub pov: String,
    pub focus: Vec<String>,
    pub present: Vec<String>,
    pub time_span: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_beats: Vec<String>,
    pub stop_condition: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_resolutions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reveal_allowed: Vec<String>,
}

impl ViewState {
    pub fn new(view: impl Into<String>, world_time: impl Into<String>, scene_index: u64) -> Self {
        Self {
            view: view.into(),
            holder: None,
            world_time: world_time.into(),
            narrative_order: 0,
            scene_index,
            task: String::new(),
            subjects: Vec::new(),
            facts: Vec::new(),
            beliefs: Vec::new(),
            rules: Vec::new(),
            threads: Vec::new(),
            tendencies: Vec::new(),
            lore: Vec::new(),
            recent_beats: Vec::new(),
            scene_plan: None,
            knowledge_boundaries: Vec::new(),
            reveal_allowed: Vec::new(),
            unapproved_secrets: Vec::new(),
            beat_text: None,
            user_input: None,
            director: None,
        }
    }

    pub fn holder(mut self, holder: Option<&SubjectId>) -> Self {
        self.holder = holder.map(|h| h.as_str().to_owned());
        self
    }

    pub fn task(mut self, task: impl Into<String>) -> Self {
        self.task = task.into();
        self
    }

    pub fn narrative_order(mut self, order: u64) -> Self {
        self.narrative_order = order;
        self
    }

    /// 按优先级裁剪到预算内（docs/08 §3.6）：规则 > 焦点状态 > 故事线 > 设定条目 > 历史。
    ///
    /// 实现方式是**从优先级最低的一段开始整段丢弃**，而不是截断单项——半句话的设定条目
    /// 比没有更糟，模型会把它当成一个残缺的事实。
    ///
    /// 规则与当前状态（facts）永不整段丢弃：丢掉它们等于让判定者在没有约束的情况下判断。
    /// 真的还超预算时，事实按锁定等级从低到高丢。
    pub fn trim_to_budget(&mut self, budget: usize) {
        let droppers: [fn(&mut Self); 6] = [
            |s| s.recent_beats.clear(),           // 5. 历史
            |s| s.lore.clear(),                   // 4. 设定条目
            |s| s.tendencies.clear(),             // 3. 趋势（故事线的一半）
            |s| s.threads.clear(),                // 3. 故事线
            |s| s.beliefs.clear(),                // 2. 认知摘要
            |s| s.knowledge_boundaries.clear(),   // 2. 认知边界
        ];
        for drop in droppers {
            if self.json_len() <= budget {
                return;
            }
            drop(self);
        }

        // 只剩主体、事实、规则，还是超预算：按锁定等级丢掉最弱的事实。
        if self.json_len() > budget {
            self.subjects.clear();
            self.facts.sort_by_key(|f| std::cmp::Reverse(f.lock_rank));
            while self.json_len() > budget && !self.facts.is_empty() {
                self.facts.pop();
            }
        }
    }

    fn json_len(&self) -> usize {
        serde_json::to_string(self).map(|s| s.len()).unwrap_or_default()
    }
}

/// 视图里出现过的命题键集合。策略层用它做「候选影响了哪些命题」的校验。
pub fn prop_keys(state: &ViewState) -> Vec<&str> {
    state.facts.iter().map(|f| f.key.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sections_are_omitted_from_json() {
        let state = ViewState::new("god", "第 1 天", 0);
        let json = serde_json::to_string(&state).unwrap();
        assert!(!json.contains("threads"), "{json}");
        assert!(!json.contains("unapproved_secrets"), "{json}");
        assert!(json.contains("\"view\":\"god\""));
    }

    #[test]
    fn identical_states_fingerprint_identically() {
        let mut a = ViewState::new("pov", "第 1 天", 2);
        a.facts.push(FactLine {
            prop: "p_1".into(),
            key: "c_gu.state".into(),
            text: "顾言在门外".into(),
            value: "true".into(),
            lock: "L2".into(),
            lock_rank: 2,
            internal: false,
            subjects: vec![],
        });
        let b = a.clone();
        assert_eq!(crate::hash::view_hash(&a), crate::hash::view_hash(&b));
    }

    #[test]
    fn trimming_drops_history_before_rules() {
        let mut state = ViewState::new("god", "第 1 天", 0);
        state.rules.push(RuleLine {
            id: "r_1".into(),
            text: "夜禁".into(),
            lock: "L1".into(),
            constraints: vec![],
            boundaries: vec![],
        });
        for i in 0..500 {
            state.recent_beats.push(BeatLine {
                narrative_order: i,
                scene: "sc_1".into(),
                text: "正文".repeat(20),
            });
        }
        state.trim_to_budget(2_000);
        assert_eq!(state.rules.len(), 1, "规则不该被裁掉");
        assert!(state.recent_beats.len() < 500, "历史应该先被裁");
    }

    #[test]
    fn identifiers_are_kept_as_strings() {
        // 视图层把所有 ID 统一降级成字符串，前端与判定者都不需要再解析一次。
        use if_domain::id::{LoreId, PropositionId, RuleId, SceneId, TendencyId, ThreadId};
        let ids = [
            PropositionId::new("p_1").as_str().to_owned(),
            RuleId::new("r_1").as_str().to_owned(),
            ThreadId::new("t_1").as_str().to_owned(),
            TendencyId::new("td_1").as_str().to_owned(),
            LoreId::new("l_1").as_str().to_owned(),
            SceneId::new("sc_1").as_str().to_owned(),
        ];
        assert!(ids.iter().all(|id| !id.is_empty()));
    }
}
