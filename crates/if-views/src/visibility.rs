//! 可见性判定——P9「视角隔离」的唯一落点。
//!
//! 这里的判断决定两件事：**谁有权看到什么**，以及**检查视图里的认知边界有多宽**。
//! 它是「正文不得泄露角色认知边界外的信息」这条硬通货的实现基础，所以规则集中在此，
//! 不散落到各个视图的构造函数里。
//!
//! 三档可见性（docs/02 §3）：
//!
//! - `public`：在场者都能感知 → 无差别可见。
//! - `private`：只有相关主体知道 → 按命题的 `subjects` 过滤。
//! - `secret`：被刻意隐藏 → **默认无人可知**，只能通过信念 / 声称 / 观测进入认知层。
//!
//! 三档之外还有 L1 保护期：保护期内的改变不让任何主体提前感知（docs/02 §3、docs/06 §4）。
//!
//! 玩家侧另有一条：读者就是玩家（docs/08 §2），玩家能看到公开事实、自己观测到的内容，
//! 以及**自己注入的 IF 核心命题**——他毕竟是这条断言的作者。

use std::collections::BTreeSet;

use if_domain::id::{PropositionId, SubjectId, USER_HOLDER};
use if_domain::projection::Projection;
use if_domain::state::Fact;
use if_domain::subject::Proposition;
use if_domain::turn::ViewKind;
use if_domain::value::Visibility;

/// 判定一条事实能否被某个视图看到。
///
/// `holder` 为 `None` 表示视图没有持有者（上帝 / 导演 / 检查 / 解析 / 创作视图），
/// 这类视图按 docs/08 §1 看得到全部事实。角色视图必须给持有者：
/// **缺持有者时退化成空视图，而不是上帝视图**——宁可什么都不给，也不能给错。
pub fn fact_is_visible(
    kind: ViewKind,
    holder: Option<&SubjectId>,
    projection: &Projection,
    proposition: &Proposition,
    fact: &Fact,
    scene_index: u64,
) -> bool {
    match kind {
        // 全知视图：冲突判定、影响候选、节拍检查都必须看到秘密，否则查不出泄露。
        ViewKind::God
        | ViewKind::Director
        | ViewKind::Check
        | ViewKind::Parse
        | ViewKind::Creation => true,
        ViewKind::Pov => match holder {
            Some(subject) => subject_can_perceive(proposition, fact, subject, scene_index),
            None => false,
        },
        // 叙事视图的过滤基准是「读者 = 玩家」，视角人物只影响措辞（docs/08 §2）。
        ViewKind::Narration | ViewKind::Player => {
            player_can_see(projection, proposition, fact, scene_index)
        }
    }
}

/// 某个主体能否感知这条事实。
fn subject_can_perceive(
    proposition: &Proposition,
    fact: &Fact,
    subject: &SubjectId,
    scene_index: u64,
) -> bool {
    if fact.is_protected_at(scene_index) {
        return false;
    }
    match fact.visibility {
        Visibility::Public => true,
        Visibility::Private => proposition.subjects.iter().any(|s| s == subject),
        // 刻意隐藏：相关主体也默认不知道。知道的人会在 Belief 层留下记录，
        // 这正是「事实与认知分离」的用处——不要用 here 猜，去问 `knowledge_of`。
        Visibility::Secret => false,
    }
}

/// 玩家能否看到这条事实。
fn player_can_see(
    projection: &Projection,
    proposition: &Proposition,
    fact: &Fact,
    scene_index: u64,
) -> bool {
    if fact.is_protected_at(scene_index) {
        return false;
    }
    if fact.visibility.is_public() {
        return true;
    }
    published_to_player(projection, proposition)
}

/// 非公开事实是否已经向玩家公开：玩家注入的 IF，或玩家观测到的内容。
fn published_to_player(projection: &Projection, proposition: &Proposition) -> bool {
    if projection
        .injections
        .values()
        .any(|injection| injection.core_prop.as_ref() == Some(&proposition.id))
    {
        return true;
    }
    projection
        .observations
        .iter()
        .any(|observation| observation.revealed.contains(&proposition.id))
}

/// 某个主体此刻「知道哪些命题」——信念层里有记录，或感知得到当前有效的事实。
///
/// 这是 `q.beat.knowledge_leak` 的检查范围（docs/04 §4）：正文让某个在场角色说出
/// 集合外的命题，就是泄露。
pub fn knowledge_of(
    projection: &Projection,
    subject: &SubjectId,
    scene_index: u64,
) -> BTreeSet<PropositionId> {
    let mut known: BTreeSet<PropositionId> = projection
        .beliefs
        .values()
        .filter(|belief| &belief.holder == subject)
        .map(|belief| belief.prop.clone())
        .collect();
    for (prop_id, fact) in &projection.facts {
        if let Some(proposition) = projection.propositions.get(prop_id) {
            if subject_can_perceive(proposition, fact, subject, scene_index) {
                known.insert(prop_id.clone());
            }
        }
    }
    known
}

/// 玩家已知的命题集合。玩家视图与叙事视图的过滤都据此。
pub fn player_knowledge(projection: &Projection, scene_index: u64) -> BTreeSet<PropositionId> {
    let mut known: BTreeSet<PropositionId> = projection
        .beliefs
        .values()
        .filter(|belief| belief.holder.as_str() == USER_HOLDER)
        .map(|belief| belief.prop.clone())
        .collect();
    for (prop_id, fact) in &projection.facts {
        if let Some(proposition) = projection.propositions.get(prop_id) {
            if player_can_see(projection, proposition, fact, scene_index) {
                known.insert(prop_id.clone());
            }
        }
    }
    known
}

/// 本场景「不许揭示」的事实清单：刻意隐藏的，以及关联主体也感知不到的。
///
/// 只进检查视图，不进叙事视图（docs/04 §2.1）。
pub fn secret_facts(projection: &Projection, scene_index: u64) -> Vec<&PropositionId> {
    let mut secrets: Vec<&PropositionId> = projection
        .facts
        .iter()
        .filter(|(prop_id, fact)| {
            if fact.visibility.is_public() {
                return false;
            }
            let Some(proposition) = projection.propositions.get(*prop_id) else {
                // 事实引用了不存在的命题：投影自身有问题，不当作秘密处理。
                return false;
            };
            if fact.visibility == Visibility::Secret {
                return true;
            }
            proposition
                .subjects
                .iter()
                .all(|holder| !subject_can_perceive(proposition, fact, holder, scene_index))
        })
        .map(|(prop_id, _)| prop_id)
        .collect();
    secrets.sort();
    secrets
}

/// 某个提案引用的事实，对该主体是否「有依据」。
///
/// `q.cand.in_character` 与 `q.cand.knowledge_gap` 的前置本地检查：
/// 候选声明自己 `based_on` 了某些命题，其中一个该主体根本不可能知道，就值得送检。
pub fn unjustified_basis(
    projection: &Projection,
    subject: &SubjectId,
    based_on: &[PropositionId],
    scene_index: u64,
) -> Vec<PropositionId> {
    let known = knowledge_of(projection, subject, scene_index);
    let mut gaps: Vec<PropositionId> = based_on
        .iter()
        .filter(|prop| !known.contains(*prop))
        .cloned()
        .collect();
    gaps.sort();
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_domain::id::EventId;
    use if_domain::subject::{PropositionKind, ValueType};
    use if_domain::value::{Lock, WorldTime};

    fn prop(id: &str, subjects: &[&str], internal: bool) -> Proposition {
        Proposition {
            id: PropositionId::new(id),
            key: format!("{id}.state"),
            text: format!("命题 {id}"),
            subjects: subjects.iter().map(|s| SubjectId::new(*s)).collect(),
            kind: PropositionKind::State,
            value_type: ValueType::Bool,
            internal,
        }
    }

    fn fact(prop: &str, visibility: Visibility) -> Fact {
        Fact::new(
            PropositionId::new(prop),
            true,
            WorldTime::EPOCH,
            Lock::L2,
            EventId::new("evt_1"),
        )
        .with_visibility(visibility)
    }

    fn seeded(p: &Proposition, f: &Fact) -> Projection {
        let mut projection = Projection::genesis("wl_main");
        projection.propositions.insert(p.id.clone(), p.clone());
        projection.facts.insert(p.id.clone(), f.clone());
        projection
    }

    #[test]
    fn public_fact_is_visible_to_everyone() {
        let p = prop("p_open", &[], false);
        let f = fact("p_open", Visibility::Public);
        let projection = seeded(&p, &f);

        let gu = SubjectId::new("c_gu");
        for kind in [ViewKind::Pov, ViewKind::Player, ViewKind::Narration, ViewKind::God] {
            let holder = matches!(kind, ViewKind::Pov).then_some(&gu);
            assert!(
                fact_is_visible(kind, holder, &projection, &p, &f, 0),
                "{kind:?} 应该看得到公开事实"
            );
        }
    }

    #[test]
    fn private_fact_only_reaches_related_subjects() {
        let p = prop("p_secret", &["c_lin"], false);
        let f = fact("p_secret", Visibility::Private);
        let projection = seeded(&p, &f);

        assert!(fact_is_visible(ViewKind::Pov, Some(&SubjectId::new("c_lin")), &projection, &p, &f, 0));
        assert!(
            !fact_is_visible(ViewKind::Pov, Some(&SubjectId::new("c_gu")), &projection, &p, &f, 0),
            "顾言无权知道林夏的私密状态"
        );
        // 玩家看不到：还没上屏。
        assert!(!fact_is_visible(ViewKind::Narration, None, &projection, &p, &f, 0));
        // 但上帝视图必须看得到——否则冲突检查与节拍检查都瞎了。
        assert!(fact_is_visible(ViewKind::God, None, &projection, &p, &f, 0));
    }

    /// `secret` 是「刻意隐藏」：连相关主体也默认不知道，只能靠 Belief 进入认知层。
    #[test]
    fn secret_fact_is_hidden_even_from_related_subjects() {
        let p = prop("p_hidden", &["c_lin"], false);
        let f = fact("p_hidden", Visibility::Secret);
        let projection = seeded(&p, &f);

        assert!(!fact_is_visible(ViewKind::Pov, Some(&SubjectId::new("c_lin")), &projection, &p, &f, 0));
        assert!(!fact_is_visible(ViewKind::Narration, None, &projection, &p, &f, 0));
        assert!(fact_is_visible(ViewKind::Check, None, &projection, &p, &f, 0));
        assert_eq!(secret_facts(&projection, 0), vec![&p.id]);
    }

    #[test]
    fn player_sees_what_they_observed_or_injected() {
        use if_domain::state::{Observation, ObservationTarget};

        let p = prop("p_secret", &["c_lin"], false);
        let f = fact("p_secret", Visibility::Private);
        let mut projection = seeded(&p, &f);
        assert!(!fact_is_visible(ViewKind::Narration, None, &projection, &p, &f, 0));

        projection.observations.push(Observation {
            target: ObservationTarget::Proposition { prop: p.id.clone() },
            text: "玩家查到了".into(),
            revealed: vec![p.id.clone()],
            world_time: WorldTime::EPOCH,
            source: EventId::new("evt_2"),
        });
        assert!(
            fact_is_visible(ViewKind::Narration, None, &projection, &p, &f, 0),
            "观测过的内容，读者就知道"
        );
    }

    /// L1 保护期按「场景序号」计时，所以它只对 L1 锁定有效（docs/02 §3）。
    /// 这里特意用 L1 构造，否则 `is_protected_at` 恒为假，测试就成了空转。
    #[test]
    fn l1_protection_shields_even_related_subjects() {
        let p = prop("p_secret", &["c_lin"], false);
        let f = Fact::new(
            PropositionId::new("p_secret"),
            true,
            WorldTime::EPOCH,
            Lock::L1,
            EventId::new("evt_1"),
        )
        .with_visibility(Visibility::Private)
        .with_protection(3);
        let projection = seeded(&p, &f);

        let lin = SubjectId::new("c_lin");
        assert!(!fact_is_visible(ViewKind::Pov, Some(&lin), &projection, &p, &f, 2));
        assert!(fact_is_visible(ViewKind::Pov, Some(&lin), &projection, &p, &f, 3));

        // 非 L1 的锁定即使带 protected_until 也不进保护期，可见性不受场景序号影响。
        let l2 = fact("p_secret", Visibility::Private).with_protection(3);
        assert!(fact_is_visible(ViewKind::Pov, Some(&lin), &projection, &p, &l2, 0));
    }

    #[test]
    fn pov_without_holder_sees_nothing_private() {
        let p = prop("p_secret", &["c_lin"], false);
        let f = fact("p_secret", Visibility::Private);
        let projection = seeded(&p, &f);
        assert!(
            !fact_is_visible(ViewKind::Pov, None, &projection, &p, &f, 0),
            "缺持有者时必须退化成空视图，而不是上帝视图"
        );
    }

    #[test]
    fn knowledge_comes_from_beliefs_and_reach() {
        let p = prop("p_private", &["c_lin"], false);
        let f = fact("p_private", Visibility::Private);
        let projection = seeded(&p, &f);

        assert!(knowledge_of(&projection, &SubjectId::new("c_gu"), 0).is_empty());
        assert_eq!(knowledge_of(&projection, &SubjectId::new("c_lin"), 0).len(), 1);
    }

    #[test]
    fn unjustified_basis_reports_what_the_subject_cannot_know() {
        let secret = prop("p_secret", &["c_lin"], false);
        let sf = fact("p_secret", Visibility::Secret);
        let mut projection = seeded(&secret, &sf);

        let open = prop("p_open", &[], false);
        let of = fact("p_open", Visibility::Public);
        projection.propositions.insert(open.id.clone(), open.clone());
        projection.facts.insert(open.id.clone(), of);

        let gu = SubjectId::new("c_gu");
        let gaps = unjustified_basis(
            &projection,
            &gu,
            &[secret.id.clone(), open.id.clone()],
            0,
        );
        assert_eq!(gaps, vec![secret.id.clone()], "只有藏在暗处的那条算越界");
    }
}
