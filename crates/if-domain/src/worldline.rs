//! 世界线：指向事件的一个「头指针」（docs/03 §6）。
//!
//! 所有世界线操作都只是创建或移动头指针，**不删除任何事件**。
//! 一条世界线的当前状态 = 它祖先链上事件并集按 `seq` 折叠得到的投影。

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::id::WorldLineId;

/// 世界线的类别（docs/03 §6）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldLineKind {
    Main,
    Branch,
    /// 重写 / 重掷：同一场景起点，沿用或更换骰子。
    Swipe,
    /// 执行回溯型 IF 前自动保存的备份分支。
    RetconBackup,
    /// 导演没有选中的高分场景候选。
    RoadNotTaken,
    /// 回滚后退回的事件留在这种分支上。
    Abandoned,
}

impl WorldLineKind {
    /// IF 导图中默认折叠的类别（docs/03 §6）。
    pub const fn collapsed_by_default(self) -> bool {
        matches!(
            self,
            WorldLineKind::Swipe | WorldLineKind::RoadNotTaken | WorldLineKind::Abandoned
        )
    }

    /// 是否是可以接受新事件的「活跃」世界线。备份与未选之路只是记录。
    pub const fn accepts_new_events(self) -> bool {
        !matches!(
            self,
            WorldLineKind::RetconBackup | WorldLineKind::RoadNotTaken
        )
    }
}

/// 分叉点。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRef {
    pub line: WorldLineId,
    /// 从父线的哪个 `seq` 分叉。该序号的事件**包含**在新分支的历史里。
    pub at_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldLine {
    pub id: WorldLineId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentRef>,
    pub head_seq: u64,
    pub label: String,
    pub kind: WorldLineKind,
}

impl WorldLine {
    /// 全新的主世界线，头指针在 0（还没有任何事件）。
    pub fn main(id: impl Into<WorldLineId>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            parent: None,
            head_seq: 0,
            label: label.into(),
            kind: WorldLineKind::Main,
        }
    }

    /// 从 `parent` 的 `at_seq` 处分叉。
    pub fn fork(
        id: impl Into<WorldLineId>,
        parent: &WorldLine,
        at_seq: u64,
        kind: WorldLineKind,
        label: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            parent: Some(ParentRef {
                line: parent.id.clone(),
                at_seq: at_seq.min(parent.head_seq),
            }),
            head_seq: at_seq.min(parent.head_seq),
            label: label.into(),
            kind,
        }
    }

    /// 把「回到此处」实现成分叉：不移动当前头指针，另起一条线。
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }
}

/// 祖先链上的一段：该世界线上截止到 `upto_seq`（含）的事件都算数。
///
/// `upto_seq` 对起点线是它的 `head_seq`，对祖先是分叉点 `at_seq`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub line: WorldLineId,
    pub upto_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldLineError {
    NotFound(WorldLineId),
    /// 父链成环。数据损坏时才会出现，但必须挡住——否则折叠会死循环。
    Cycle(WorldLineId),
}

impl fmt::Display for WorldLineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorldLineError::NotFound(id) => write!(f, "世界线不存在: {id}"),
            WorldLineError::Cycle(id) => write!(f, "世界线的父链成环: {id}"),
        }
    }
}

impl std::error::Error for WorldLineError {}

/// 沿父链走到根，得到折叠所需的全部段。顺序是**从子到父**。
///
/// 起点线取它的 `head_seq`；每个祖先取「子线指向它的分叉点」，并夹在该祖先自己的
/// `head_seq` 之内。因此继承过来的历史**恰好到分叉点为止**，不含分叉之后的事件。
pub fn lineage(
    lines: &BTreeMap<WorldLineId, WorldLine>,
    start: &WorldLineId,
) -> Result<Vec<Segment>, WorldLineError> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut seen: Vec<WorldLineId> = Vec::new();

    let first = lines
        .get(start)
        .ok_or_else(|| WorldLineError::NotFound(start.clone()))?;
    let mut cursor = start.clone();
    let mut upto = first.head_seq;

    loop {
        if seen.contains(&cursor) {
            return Err(WorldLineError::Cycle(cursor));
        }
        seen.push(cursor.clone());
        segments.push(Segment {
            line: cursor.clone(),
            upto_seq: upto,
        });

        let line = lines
            .get(&cursor)
            .ok_or_else(|| WorldLineError::NotFound(cursor.clone()))?;
        let parent_ref = match line.parent.as_ref() {
            Some(p) => p,
            None => break,
        };
        let parent = lines
            .get(&parent_ref.line)
            .ok_or_else(|| WorldLineError::NotFound(parent_ref.line.clone()))?;
        // 祖先只能看到分叉点为止，不能超过它自己的头指针，也不能超过子线已经截到的位置。
        upto = parent_ref.at_seq.min(parent.head_seq).min(upto);
        cursor = parent_ref.line.clone();
    }

    Ok(segments)
}

/// 判断某个 `(line, seq)` 是否落在祖先链的可见范围内。
pub fn covers(segments: &[Segment], line: &WorldLineId, seq: u64) -> bool {
    segments
        .iter()
        .any(|s| &s.line == line && seq <= s.upto_seq)
}

/// 从祖先链追溯共同祖先，用于展示「世界线偏移」和导图连线【后续会用到】。
pub fn common_ancestor(
    lines: &BTreeMap<WorldLineId, WorldLine>,
    a: &WorldLineId,
    b: &WorldLineId,
) -> Result<Option<WorldLineId>, WorldLineError> {
    let chain_a: Vec<WorldLineId> = lineage(lines, a)?.into_iter().map(|s| s.line).collect();
    let chain_b: Vec<WorldLineId> = lineage(lines, b)?.into_iter().map(|s| s.line).collect();
    Ok(chain_a.into_iter().find(|id| chain_b.contains(id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(lines: Vec<WorldLine>) -> BTreeMap<WorldLineId, WorldLine> {
        lines.into_iter().map(|l| (l.id.clone(), l)).collect()
    }

    #[test]
    fn main_line_has_single_segment() {
        let mut main = WorldLine::main("wl_main", "主世界线");
        main.head_seq = 10;
        let lines = map(vec![main.clone()]);
        let segs = lineage(&lines, &main.id).unwrap();
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].upto_seq, 10);
    }

    #[test]
    fn fork_inherits_history_up_to_the_fork_point() {
        let mut main = WorldLine::main("wl_main", "主世界线");
        main.head_seq = 100;
        let lines = map(vec![main.clone()]);
        let branch = WorldLine::fork("wl_b1", &main, 40, WorldLineKind::Branch, "分支");
        assert_eq!(branch.head_seq, 40);

        let mut with_branch = lines.clone();
        with_branch.insert(branch.id.clone(), branch.clone());
        let segs = lineage(&with_branch, &branch.id).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].line.as_str(), "wl_b1");
        assert_eq!(segs[0].upto_seq, 40);
        assert_eq!(segs[1].line.as_str(), "wl_main");
        // 关键：父线只继承到分叉点，不含分叉之后的事件
        assert_eq!(segs[1].upto_seq, 40);

        assert!(covers(&segs, &WorldLineId::new("wl_main"), 40));
        assert!(!covers(&segs, &WorldLineId::new("wl_main"), 41));
        assert!(covers(&segs, &WorldLineId::new("wl_b1"), 40));
        assert!(!covers(&segs, &WorldLineId::new("wl_b1"), 41));
    }

    #[test]
    fn fork_from_future_seq_is_clamped_to_head() {
        let mut main = WorldLine::main("wl_main", "主");
        main.head_seq = 10;
        let branch = WorldLine::fork("wl_b", &main, 999, WorldLineKind::Swipe, "重写");
        assert_eq!(branch.head_seq, 10);
        assert_eq!(branch.parent.unwrap().at_seq, 10);
    }

    #[test]
    fn nested_forks_keep_each_fork_point() {
        let mut main = WorldLine::main("wl_main", "主");
        main.head_seq = 100;
        let b1 = WorldLine::fork("wl_b1", &main, 60, WorldLineKind::Branch, "一");
        let b2 = WorldLine::fork("wl_b2", &b1, 30, WorldLineKind::Branch, "二");
        let lines = map(vec![main, b1, b2.clone()]);
        let segs = lineage(&lines, &b2.id).unwrap();
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].upto_seq, 30); // b2 head
        assert_eq!(segs[1].upto_seq, 30); // b1 到分叉点
        assert_eq!(segs[2].upto_seq, 30); // main 到 b1 的分叉点
    }

    #[test]
    fn missing_or_cyclic_parents_are_errors() {
        let lines = map(vec![WorldLine::main("wl_main", "主")]);
        assert_eq!(
            lineage(&lines, &WorldLineId::new("wl_nope")),
            Err(WorldLineError::NotFound(WorldLineId::new("wl_nope")))
        );

        // 人为造一个环
        let mut a = WorldLine::main("wl_a", "a");
        a.parent = Some(ParentRef {
            line: WorldLineId::new("wl_b"),
            at_seq: 1,
        });
        let mut b = WorldLine::main("wl_b", "b");
        b.parent = Some(ParentRef {
            line: WorldLineId::new("wl_a"),
            at_seq: 1,
        });
        let cyclic = map(vec![a, b]);
        assert!(matches!(
            lineage(&cyclic, &WorldLineId::new("wl_a")),
            Err(WorldLineError::Cycle(_))
        ));
    }

    #[test]
    fn common_ancestor_finds_the_fork_root() {
        let mut main = WorldLine::main("wl_main", "主");
        main.head_seq = 100;
        let b1 = WorldLine::fork("wl_b1", &main, 60, WorldLineKind::Branch, "一");
        let b2 = WorldLine::fork("wl_b2", &main, 70, WorldLineKind::Branch, "二");
        let lines = map(vec![main.clone(), b1.clone(), b2.clone()]);
        assert_eq!(
            common_ancestor(&lines, &b1.id, &b2.id).unwrap(),
            Some(main.id)
        );
    }

    #[test]
    fn backup_and_road_not_taken_are_not_live() {
        assert!(!WorldLineKind::RetconBackup.accepts_new_events());
        assert!(!WorldLineKind::RoadNotTaken.accepts_new_events());
        assert!(WorldLineKind::Main.accepts_new_events());
        assert!(WorldLineKind::Branch.accepts_new_events());
        assert!(WorldLineKind::Swipe.collapsed_by_default());
        assert!(!WorldLineKind::Main.collapsed_by_default());
    }
}
