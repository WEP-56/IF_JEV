//! 名字 ↔ ID 的解析。
//!
//! 视图里给模型看的是**名字**（文档、判定问题里都写「林夏」而不是 `c_lin`，见
//! [`crate::context::subject_name`]），而引擎要的是 ID。提议工具因此不能直接收 ID：
//! 让模型记住并复述一串 `c_*` / `p_*` 只会多一类「抄错 ID」的失败。
//!
//! 这里的规矩是**两个都认**——先按 ID，再按名字与别名走一遍；认不出来的返回 `None`，
//! 由调用方记成警告而不是抛错。原因和 `suggested_lock` 那次一样：模型答不上来的东西
//! 不该让整个任务失败，但**也不能悄悄当成猜对了**。

use if_domain::id::{LoreId, PropositionId, SubjectId, TendencyId, ThreadId};
use if_domain::projection::Projection;

/// 一批解析结果：认出来的 + 认不出来（供调用方记警告）。
#[derive(Debug, PartialEq)]
pub struct Resolved<T> {
    pub ids: Vec<T>,
    pub unknown: Vec<String>,
}

/// 手写 `Default`：派生的版本会要求 `T: Default`，而 `SubjectId` 这类 ID 没有默认值。
impl<T> Default for Resolved<T> {
    fn default() -> Self {
        Resolved {
            ids: Vec::new(),
            unknown: Vec::new(),
        }
    }
}

impl<T: PartialEq> Resolved<T> {
    fn push(&mut self, id: T) {
        if !self.ids.contains(&id) {
            self.ids.push(id);
        }
    }
}

#[derive(Debug)]
pub struct Resolver<'a> {
    projection: &'a Projection,
}

impl<'a> Resolver<'a> {
    pub fn new(projection: &'a Projection) -> Self {
        Self { projection }
    }

    pub fn projection(&self) -> &'a Projection {
        self.projection
    }

    /// 主体：先按 ID，再按名字（含大小写无关），最后按别名。
    pub fn subject(&self, raw: &str) -> Option<SubjectId> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let by_id = SubjectId::new(raw);
        if self.projection.subjects.contains_key(&by_id) {
            return Some(by_id);
        }
        if let Some(found) = self
            .projection
            .subjects
            .values()
            .find(|subject| subject.name == raw || subject.aliases.iter().any(|alias| alias == raw))
            .map(|subject| subject.id.clone())
        {
            return Some(found);
        }
        let lower = raw.to_lowercase();
        self.projection
            .subjects
            .values()
            .find(|subject| subject.name.to_lowercase() == lower)
            .map(|subject| subject.id.clone())
    }

    pub fn subjects(&self, raw: &[String]) -> Resolved<SubjectId> {
        let mut resolved = Resolved::default();
        for item in raw {
            match self.subject(item) {
                Some(id) => resolved.push(id),
                None => resolved.unknown.push(item.clone()),
            }
        }
        resolved
    }

    /// 命题：先按**规范键**（视图里显示的就是它），再按 ID。
    pub fn proposition(&self, raw: &str) -> Option<PropositionId> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if self.projection.propositions.contains_key(&PropositionId::new(raw)) {
            return Some(PropositionId::new(raw));
        }
        self.projection
            .propositions
            .values()
            .find(|prop| prop.key == raw)
            .map(|prop| prop.id.clone())
    }

    pub fn propositions(&self, raw: &[String]) -> Resolved<PropositionId> {
        let mut resolved = Resolved::default();
        for item in raw {
            match self.proposition(item) {
                Some(id) => resolved.push(id),
                None => resolved.unknown.push(item.clone()),
            }
        }
        resolved
    }

    /// 命题 ID → 规范键。视图给模型看的是键，所以回指、报错都要用它。
    pub fn key_of(&self, prop: &PropositionId) -> Option<String> {
        self.projection.propositions.get(prop).map(|prop| prop.key.clone())
    }

    /// 设定条目：按 ID，其次按标题（作者用 `comment` 当标题，见 docs/13 §6.4）。
    pub fn lore(&self, raw: &str) -> Option<LoreId> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if self.projection.lore.contains_key(&LoreId::new(raw)) {
            return Some(LoreId::new(raw));
        }
        self.projection
            .lore
            .values()
            .find(|entry| entry.title == raw)
            .map(|entry| entry.id.clone())
    }

    pub fn thread(&self, raw: &str) -> Option<ThreadId> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let id = ThreadId::new(raw);
        if self.projection.threads.contains_key(&id) {
            return Some(id);
        }
        self.projection
            .threads
            .values()
            .find(|thread| thread.title == raw)
            .map(|thread| thread.id.clone())
    }

    pub fn threads(&self, raw: &[String]) -> Resolved<ThreadId> {
        let mut resolved = Resolved::default();
        for item in raw {
            match self.thread(item) {
                Some(id) => resolved.push(id),
                None => resolved.unknown.push(item.clone()),
            }
        }
        resolved
    }

    pub fn tendency(&self, raw: &str) -> Option<TendencyId> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let id = TendencyId::new(raw);
        self.projection.tendencies.contains_key(&id).then_some(id)
    }

    pub fn tendencies(&self, raw: &[String]) -> Resolved<TendencyId> {
        let mut resolved = Resolved::default();
        for item in raw {
            match self.tendency(item) {
                Some(id) => resolved.push(id),
                None => resolved.unknown.push(item.clone()),
            }
        }
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport;

    #[test]
    fn resolves_by_id_name_and_key() {
        let projection = testsupport::projection_with_lore();
        let resolver = Resolver::new(&projection);

        assert_eq!(resolver.subject("c_lin").as_ref().map(SubjectId::as_str), Some("c_lin"));
        assert_eq!(resolver.subject("林夏").as_ref().map(SubjectId::as_str), Some("c_lin"));
        assert_eq!(resolver.subject("  顾言 ").as_ref().map(SubjectId::as_str), Some("c_gu"));
        assert_eq!(resolver.subject("查无此人"), None);
        assert_eq!(resolver.subject("   "), None);

        // 命题按规范键来（视图给模型看的就是这个）。
        assert_eq!(
            resolver.proposition("c_lin.mood").as_ref().map(PropositionId::as_str),
            Some("p_lin_mood")
        );
        assert_eq!(
            resolver.proposition("p_lin_mood").as_ref().map(PropositionId::as_str),
            Some("p_lin_mood")
        );
        assert_eq!(resolver.proposition("nope"), None);
        assert_eq!(resolver.key_of(&PropositionId::new("p_lin_mood")).as_deref(), Some("c_lin.mood"));

        assert_eq!(resolver.thread("thr_shield").as_ref().map(ThreadId::as_str), Some("thr_shield"));
        assert_eq!(
            resolver.thread("顾言的身世会不会被揭开").as_ref().map(ThreadId::as_str),
            Some("thr_shield")
        );
        assert_eq!(resolver.lore("大乾").as_ref().map(LoreId::as_str), Some("lore_world_1"));
    }

    /// 认不出来的名字要如实报出来——调用方拿它写警告，而不是静默丢掉。
    #[test]
    fn unknown_names_are_reported_not_silently_dropped() {
        let projection = testsupport::projection();
        let resolver = Resolver::new(&projection);
        let resolved = resolver.subjects(&["林夏".into(), "林夏".into(), "路人甲".into()]);
        assert_eq!(resolved.ids, vec![SubjectId::new("c_lin")]);
        assert_eq!(resolved.unknown, vec!["路人甲".to_owned()]);
    }
}
