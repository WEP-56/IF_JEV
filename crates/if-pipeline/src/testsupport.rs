//! 测试夹具：一个够小、但每种东西都有一份的世界。
//!
//! 各模块的测试都从这里取世界，好处不是省几行代码，而是**同一份输入被所有阶段共享**——
//! 「L1 保护期在候选裁决和节拍检查里算的是同一个场景序号」这类性质，只有在夹具唯一时
//! 才真的被验证。

use std::collections::BTreeMap;

use if_domain::id::{
    EventId, PropositionId, RuleId, SceneId, SubjectId, TendencyId, ThreadId, TurnId,
};
use if_domain::narrative::{
    LoreEntry, LoreSection, LoreStatus, LoreVisibility, SecondaryLogic, ScenePlan, Tendency,
    TendencyStatus, Thread, ThreadStage,
};
use if_domain::projection::Projection;
use if_domain::rule::{ActiveWindow, MechanicStep, Mode, NarrativePov, WorldRule, WorldSettings};
use if_domain::state::Fact;
use if_domain::subject::{Proposition, PropositionKind, Subject, SubjectKind, Tier, ValueType};
use if_domain::turn::TurnKind;
use if_domain::value::{Lock, Value, Visibility, WorldTime};

use crate::context::TurnContext;

/// 林夏。
pub const LIN: &str = "c_lin";
/// 顾言。
pub const GU: &str = "c_gu";

pub const P_MOOD: &str = "p_lin_mood";
pub const P_EVIDENCE: &str = "p_lin_evidence";
pub const P_SECRET: &str = "p_gu_secret";
pub const P_RAIN: &str = "p_world_rain";

pub const T_SHIELD: &str = "thr_shield";
pub const R_NIGHT: &str = "rule_night";

pub fn settings() -> WorldSettings {
    WorldSettings {
        seed: 20260926,
        narrative_style: "克制、白描".into(),
        narrative_pov: NarrativePov::ThirdLimited,
        mechanic_step: MechanicStep::Day,
        director_style: "均衡".into(),
        mode: Mode::Sandbox,
    }
}

fn character(id: &str, name: &str, profile: &str) -> Subject {
    Subject {
        id: SubjectId::new(id),
        kind: SubjectKind::Character,
        name: name.into(),
        aliases: Vec::new(),
        profile: profile.into(),
        voice: None,
        tier: Tier::Active,
        shaped: true,
        created_by: EventId::new("evt_0001"),
    }
}

fn state(id: &str, key: &str, text: &str, subjects: &[&str], internal: bool) -> Proposition {
    Proposition {
        id: PropositionId::new(id),
        key: key.into(),
        text: text.into(),
        subjects: subjects.iter().map(|s| SubjectId::new(*s)).collect(),
        kind: PropositionKind::State,
        value_type: if internal {
            ValueType::Scalar {
                min: 0.0,
                max: 1.0,
                unit: None,
            }
        } else {
            ValueType::Bool
        },
        internal,
    }
}

/// 一个有事实、有规则、有一条受保护故事线的世界。
///
/// 受保护故事线是刻意的：它是「引擎硬否决」唯一的触发条件，没有它，
/// 场景选择的两条分支里只有一条能被走到。
pub fn projection() -> Projection {
    let mut projection = Projection::genesis("wl_main");
    projection.world_time = WorldTime::from_days(7);
    projection.world_seed = Some(settings().seed);

    for subject in [
        character(LIN, "林夏", "冷静、克制，习惯先算后果"),
        character(GU, "顾言", "冲动、讲义气，藏不住话"),
    ] {
        projection.subjects.insert(subject.id.clone(), subject);
    }

    for proposition in [
        state(P_MOOD, "c_lin.mood", "林夏的心情", &[LIN], true),
        state(P_EVIDENCE, "c_lin.evidence", "那封信在林夏手上", &[LIN], false),
        state(P_SECRET, "c_gu.secret", "顾言是失踪的王储", &[GU], false),
        state(P_RAIN, "world.rain", "外面在下雨", &[], false),
    ] {
        projection.propositions.insert(proposition.id.clone(), proposition);
    }

    let at = WorldTime::from_days(7);
    let facts = [
        (
            P_MOOD,
            0.4,
            Lock::L0,
            Visibility::Private,
            "evt_0002",
        ),
        (P_EVIDENCE, true.into(), Lock::L2, Visibility::Public, "evt_0003"),
        (P_SECRET, true.into(), Lock::L2, Visibility::Secret, "evt_0004"),
        (P_RAIN, true.into(), Lock::L0, Visibility::Public, "evt_0005"),
    ];
    for (prop, value, lock, visibility, source) in facts {
        projection.facts.insert(
            PropositionId::new(prop),
            Fact::new(
                PropositionId::new(prop),
                value,
                at,
                lock,
                EventId::new(source),
            )
            .with_visibility(visibility),
        );
    }

    projection.rules.insert(
        RuleId::new(R_NIGHT),
        WorldRule {
            id: RuleId::new(R_NIGHT),
            text: "夜里宫门落锁，任何人都不得出入".into(),
            lock: Lock::L2,
            source: EventId::new("evt_0006"),
            active: ActiveWindow::from_now(WorldTime::EPOCH),
            scope: Vec::new(),
            invariants: Vec::new(),
            mechanics: Vec::new(),
            triggers: Vec::new(),
            constraints: Vec::new(),
            boundaries: vec!["仅限皇城宫门".into()],
        },
    );

    projection.threads.insert(
        ThreadId::new(T_SHIELD),
        Thread {
            id: ThreadId::new(T_SHIELD),
            title: "顾言的身世会不会被揭开".into(),
            question: "顾言是王储这件事最终会被证明吗".into(),
            stakes: "揭开意味着他必须离开".into(),
            subjects: vec![SubjectId::new(GU), SubjectId::new(LIN)],
            stage: ThreadStage::Developing,
            // 保护到高潮：这一回合不许它收束。
            protected_until: Some(ThreadStage::Climax),
            pressure: 0.5,
            last_advanced: 0,
            cadence: 3.0,
        },
    );

    projection
}

/// 一条常驻的世界条目 + 一条带关键词的场景条目。
pub fn lore() -> Vec<LoreEntry> {
    vec![
        LoreEntry {
            id: if_domain::id::LoreId::new("lore_world_1"),
            title: "大乾".into(),
            content: "大乾立国三百年，宫城在雨中显得更暗。".into(),
            keys: Vec::new(),
            secondary_keys: Vec::new(),
            logic: SecondaryLogic::AndAny,
            subjects: Vec::new(),
            when: None,
            constant: true,
            order: 100,
            section: LoreSection::World,
            visibility: LoreVisibility::Public,
            known_by: Vec::new(),
            probability: None,
            status: LoreStatus::Active,
            source: EventId::new("evt_0007"),
        },
        LoreEntry {
            id: if_domain::id::LoreId::new("lore_scene_1"),
            title: "宫门落锁".into(),
            content: "落锁之后，宫门外的灯会一盏盏熄掉。".into(),
            keys: vec!["宫门".into()],
            secondary_keys: Vec::new(),
            logic: SecondaryLogic::AndAny,
            subjects: Vec::new(),
            when: None,
            constant: false,
            order: 50,
            section: LoreSection::Scene,
            visibility: LoreVisibility::Public,
            known_by: Vec::new(),
            probability: None,
            status: LoreStatus::Active,
            source: EventId::new("evt_0008"),
        },
    ]
}

pub fn ctx(scene_index: u64) -> TurnContext {
    TurnContext::new(
        TurnId::numbered(1),
        "wl_main",
        TurnKind::If,
        WorldTime::from_days(7),
        scene_index,
    )
    .focus([SubjectId::new(LIN), SubjectId::new(GU)])
    .present([SubjectId::new(LIN), SubjectId::new(GU)])
    .lore(lore())
    .narrative_order(3)
}

/// 一份合法的场景计划：视角人物在场，禁止项留空（由引擎注入）。
pub fn plan(goal: &str) -> ScenePlan {
    ScenePlan {
        goal: goal.into(),
        pov: SubjectId::new(LIN),
        focus: vec![SubjectId::new(GU)],
        present: vec![SubjectId::new(LIN), SubjectId::new(GU)],
        time_span: "当晚".into(),
        required_beats: vec!["顾言开口".into(), "林夏没有回答".into()],
        stop_condition: "顾言把话说完".into(),
        forbidden_resolutions: Vec::new(),
        reveal_allowed: Vec::new(),
        proposed_changes: Vec::new(),
    }
}

/// 一条待决趋势，压力已过观察带，供导演评分与 `q.tendency.push` 用。
pub fn tendency(id: &str, text: &str, pressure: f64) -> Tendency {
    let mut tendency = Tendency::latent(TendencyId::new(id), text, pressure);
    tendency.status = TendencyStatus::Latent;
    tendency
}

/// 命题 → 值 的小表，方便断言提交了什么。
pub fn values(changes: &[crate::commit::CommittedChange]) -> BTreeMap<String, Value> {
    changes
        .iter()
        .map(|change| (change.prop.as_str().to_owned(), change.value.clone()))
        .collect()
}

/// 场景 ID 的简写。
pub fn scene_id(id: &str) -> SceneId {
    SceneId::new(id)
}
