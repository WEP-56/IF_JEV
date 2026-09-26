//! 分层裁决的排序部分：把候选按 `depends_on` 排成因果层（docs/06 §4）。
//!
//! 为什么不能各自独立掷骰：同一个请求里的问题是独立评估的，所以可能掷出
//! 「保护 ✗、回避 ✗、察觉异常 ✓」这种不自洽的组合。分层的意义是让下游候选
//! 在**更新后的临时状态**上重新判定——前提不成立时 Jev 自然会给出很低的概率，
//! 引擎不需要去区分「必要条件」和「影响因素」。
//!
//! 这里只做排序与环检测，不接触概率。它唯一的输出是「先算谁、后算谁、谁这回合不算」。

use std::collections::{BTreeMap, BTreeSet};

use if_domain::{Candidate, CandidateId};

/// 因果深度上限【初始值 3】（docs/06 §4）。
///
/// 它是**层数**上限，不是候选数上限：每一层内部的判定可以并行发出，
/// 跨层必须串行，所以决定往返次数的是层数。
pub const MAX_CAUSAL_DEPTH: usize = 3;

/// 排序失败。两种都是输入本身的问题，不是策略问题——所以是错误而不是降级。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayerError {
    #[error("候选 {id} 出现了多次")]
    Duplicate { id: String },
    #[error("候选 {candidate} 依赖了不存在的候选 {missing}")]
    UnknownDependency { candidate: String, missing: String },
    #[error("候选依赖成环，无法确定因果顺序：{members:?}")]
    Cycle { members: Vec<String> },
}

/// 排好层的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layered {
    /// 每个元素是一个因果层，层内按候选 ID 排序。层数不超过 [`MAX_CAUSAL_DEPTH`]。
    pub layers: Vec<Vec<CandidateId>>,
    /// 因果深度超上限的候选，留到下一回合或转为趋势（docs/06 §4）。
    pub deferred: Vec<CandidateId>,
}

impl Layered {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty() && self.deferred.is_empty()
    }

    /// 全部参与本回合裁决的候选（不含推迟的）。
    pub fn scheduled(&self) -> impl Iterator<Item = &CandidateId> {
        self.layers.iter().flatten()
    }

    /// 某个候选落在哪一层。推迟的候选返回 `None`。
    pub fn depth_of(&self, id: &CandidateId) -> Option<usize> {
        self.layers
            .iter()
            .position(|layer| layer.contains(id))
    }

    /// 某个候选是否被推迟。
    pub fn is_deferred(&self, id: &CandidateId) -> bool {
        self.deferred.contains(id)
    }
}

/// 按依赖分层。
///
/// 用的是「最长路径」分层：一个候选的深度 = 它的所有依赖的深度的最大值 + 1。
/// 不是「按层剥离」——那样会把 `A ← B ← C` 压成三层深度不同的东西，
/// 而这里的层号要能表达「这层的判定能看到上几层的全部结果」。
pub fn layer(candidates: &[Candidate]) -> Result<Layered, LayerError> {
    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    for (position, candidate) in candidates.iter().enumerate() {
        if index.insert(candidate.id.as_str(), position).is_some() {
            return Err(LayerError::Duplicate {
                id: candidate.id.to_string(),
            });
        }
    }

    for candidate in candidates {
        for dependency in &candidate.depends_on {
            if !index.contains_key(dependency.as_str()) {
                return Err(LayerError::UnknownDependency {
                    candidate: candidate.id.to_string(),
                    missing: dependency.to_string(),
                });
            }
        }
    }

    // 入度与反向边。去重之后自环也退化成普通环，用同一套检测。
    let mut indegree = vec![0_usize; candidates.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); candidates.len()];
    for (position, candidate) in candidates.iter().enumerate() {
        let mut seen = BTreeSet::new();
        for dependency in &candidate.depends_on {
            let upstream = index[dependency.as_str()];
            if seen.insert(upstream) {
                indegree[position] += 1;
                dependents[upstream].push(position);
            }
        }
    }

    let mut depth = vec![0_usize; candidates.len()];
    // 就绪集合用 `BTreeSet` 而不是队列：同层内按候选 ID 出队，
    // 于是同一个输入永远得到同一个分层结果。
    let mut ready: BTreeSet<&str> = candidates
        .iter()
        .enumerate()
        .filter(|(position, _)| indegree[*position] == 0)
        .map(|(_, candidate)| candidate.id.as_str())
        .collect();

    let mut processed = 0_usize;
    while let Some(id) = ready.pop_first() {
        let position = index[id];
        processed += 1;
        for &downstream in &dependents[position] {
            if depth[position] + 1 > depth[downstream] {
                depth[downstream] = depth[position] + 1;
            }
            indegree[downstream] -= 1;
            if indegree[downstream] == 0 {
                ready.insert(candidates[downstream].id.as_str());
            }
        }
    }

    if processed != candidates.len() {
        // 入度还没归零的就是环上的人（以及环的下游）。
        let members = candidates
            .iter()
            .enumerate()
            .filter(|(position, _)| indegree[*position] > 0)
            .map(|(_, candidate)| candidate.id.to_string())
            .collect();
        return Err(LayerError::Cycle { members });
    }

    let mut layered = Layered::default();
    for (position, candidate) in candidates.iter().enumerate() {
        if depth[position] >= MAX_CAUSAL_DEPTH {
            layered.deferred.push(candidate.id.clone());
        } else {
            while layered.layers.len() <= depth[position] {
                layered.layers.push(Vec::new());
            }
            layered.layers[depth[position]].push(candidate.id.clone());
        }
    }
    for layer in &mut layered.layers {
        layer.sort();
    }
    layered.deferred.sort();
    Ok(layered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_domain::{CandidateShape, SubjectId};

    fn candidate(id: &str, depends_on: &[&str]) -> Candidate {
        Candidate {
            id: CandidateId::new(id),
            key: None,
            subject: Some(SubjectId::new("c_gu")),
            content: format!("候选 {id}"),
            internal: false,
            shape: CandidateShape::Occurs,
            depends_on: depends_on.iter().map(|d| CandidateId::new(*d)).collect(),
            based_on: vec![],
            affects: vec![],
        }
    }

    fn ids(layer: &[CandidateId]) -> Vec<&str> {
        layer.iter().map(|id| id.as_str()).collect()
    }

    #[test]
    fn independent_candidates_share_one_layer() {
        let input = vec![candidate("cand_1", &[]), candidate("cand_2", &[])];
        let layered = layer(&input).unwrap();
        assert_eq!(layered.layers.len(), 1);
        assert_eq!(ids(&layered.layers[0]), ["cand_1", "cand_2"]);
        assert!(layered.deferred.is_empty());
    }

    #[test]
    fn a_chain_becomes_one_candidate_per_layer() {
        let input = vec![
            candidate("cand_3", &["cand_2"]),
            candidate("cand_1", &[]),
            candidate("cand_2", &["cand_1"]),
        ];
        let layered = layer(&input).unwrap();
        assert_eq!(layered.layers.len(), 3);
        assert_eq!(ids(&layered.layers[0]), ["cand_1"]);
        assert_eq!(ids(&layered.layers[1]), ["cand_2"]);
        assert_eq!(ids(&layered.layers[2]), ["cand_3"]);
        assert_eq!(layered.depth_of(&CandidateId::new("cand_3")), Some(2));
        assert_eq!(
            layered.scheduled().count(),
            3,
            "所有候选都排上了"
        );
    }

    #[test]
    fn depth_is_the_longest_path_not_the_first_found() {
        // cand_4 依赖 cand_1 和 cand_3；cand_3 又在 cand_2 之上。
        // 最长路径是 cand_1 → cand_2 → cand_3 → cand_4，所以 cand_4 在深度 3。
        let input = vec![
            candidate("cand_1", &[]),
            candidate("cand_2", &["cand_1"]),
            candidate("cand_3", &["cand_2"]),
            candidate("cand_4", &["cand_1", "cand_3"]),
        ];
        let layered = layer(&input).unwrap();
        // 深度 3 超上限 → 推迟
        assert!(layered.is_deferred(&CandidateId::new("cand_4")));
        assert_eq!(layered.layers.len(), MAX_CAUSAL_DEPTH);
        assert_eq!(ids(&layered.deferred), ["cand_4"]);
    }

    #[test]
    fn downstream_beyond_the_cap_is_deferred_not_dropped() {
        let input = vec![
            candidate("cand_1", &[]),
            candidate("cand_2", &["cand_1"]),
            candidate("cand_3", &["cand_2"]),
            candidate("cand_4", &["cand_3"]),
            candidate("cand_5", &["cand_4"]),
        ];
        let layered = layer(&input).unwrap();
        assert_eq!(layered.layers.len(), MAX_CAUSAL_DEPTH);
        assert_eq!(ids(&layered.deferred), ["cand_4", "cand_5"]);
        assert!(!layered.is_empty());
    }

    #[test]
    fn order_is_deterministic_regardless_of_input_order() {
        let forward = vec![
            candidate("cand_1", &[]),
            candidate("cand_2", &[]),
            candidate("cand_3", &["cand_1", "cand_2"]),
            candidate("cand_4", &["cand_2"]),
        ];
        let mut backward = forward.clone();
        backward.reverse();
        assert_eq!(layer(&forward).unwrap(), layer(&backward).unwrap());
    }

    #[test]
    fn duplicate_dependencies_count_once() {
        let input = vec![
            candidate("cand_1", &[]),
            candidate("cand_2", &["cand_1", "cand_1"]),
        ];
        let layered = layer(&input).unwrap();
        assert_eq!(layered.depth_of(&CandidateId::new("cand_2")), Some(1));
    }

    #[test]
    fn unknown_dependency_is_an_error() {
        let input = vec![candidate("cand_1", &["cand_missing"])];
        assert_eq!(
            layer(&input),
            Err(LayerError::UnknownDependency {
                candidate: "cand_1".into(),
                missing: "cand_missing".into(),
            })
        );
    }

    #[test]
    fn duplicate_candidate_id_is_an_error() {
        let input = vec![candidate("cand_1", &[]), candidate("cand_1", &[])];
        assert_eq!(
            layer(&input),
            Err(LayerError::Duplicate {
                id: "cand_1".into()
            })
        );
    }

    #[test]
    fn cycles_are_detected_and_named() {
        let input = vec![
            candidate("cand_1", &["cand_2"]),
            candidate("cand_2", &["cand_1"]),
            candidate("cand_3", &[]),
        ];
        match layer(&input) {
            Err(LayerError::Cycle { members }) => assert_eq!(members, ["cand_1", "cand_2"]),
            other => panic!("应当报环，得到 {other:?}"),
        }
    }

    #[test]
    fn self_dependency_is_a_cycle() {
        let input = vec![candidate("cand_1", &["cand_1"])];
        assert!(matches!(layer(&input), Err(LayerError::Cycle { .. })));
    }

    #[test]
    fn empty_input_layers_to_nothing() {
        let layered = layer(&[]).unwrap();
        assert!(layered.is_empty());
        assert!(layered.layers.is_empty());
    }
}
