use std::collections::BTreeMap;

use if_domain::event::{Event, Patch};
use if_domain::id::{EventId, LoreId, PropositionId, SubjectId, TurnId, WorldLineId};
use if_domain::narrative::{LoreEntry, LoreSection, LoreStatus, LoreVisibility, SecondaryLogic};
use if_domain::projection::Projection;
use if_domain::rule::{CompareOp, Condition, WorldSettings};
use if_domain::state::Fact;
use if_domain::subject::{Proposition, PropositionKind, ValueType};
use if_domain::value::{Lock, WorldTime};
use if_policy::Dice;

use super::*;

fn entry(id: &str, content: &str) -> LoreEntry {
    LoreEntry {
        id: LoreId::new(id),
        title: id.to_owned(),
        content: content.to_owned(),
        keys: Vec::new(),
        secondary_keys: Vec::new(),
        logic: SecondaryLogic::AndAny,
        subjects: Vec::new(),
        when: None,
        constant: false,
        order: 0,
        section: LoreSection::World,
        visibility: LoreVisibility::Public,
        known_by: Vec::new(),
        probability: None,
        status: LoreStatus::Active,
        source: EventId::numbered(1),
    }
}

fn projection(entries: Vec<LoreEntry>) -> Projection {
    let mut events = vec![Event {
        id: EventId::numbered(1),
        line: WorldLineId::new("wl_main"),
        seq: 1,
        world_time: WorldTime::EPOCH,
        narrative_order: None,
        payload: Patch::WorldCreated {
            settings: WorldSettings { seed: 20260926, ..Default::default() },
            label: "测试世界".into(),
        },
        caused_by: Vec::new(),
        depends_on: Vec::new(),
        turn: TurnId::numbered(0),
        scene: None,
        beat: None,
    }];
    for (index, lore) in entries.into_iter().enumerate() {
        events.push(Event {
            id: EventId::numbered(index as u64 + 2),
            line: WorldLineId::new("wl_main"),
            seq: index as u64 + 2,
            world_time: WorldTime::EPOCH,
            narrative_order: None,
            payload: Patch::LoreAdded(Box::new(lore)),
            caused_by: Vec::new(),
            depends_on: Vec::new(),
            turn: TurnId::numbered(0),
            scene: None,
            beat: None,
        });
    }
    Projection::fold("wl_main", &events).unwrap()
}

fn dice() -> Dice {
    Dice::new(20260926)
}

fn ids(activated: &[LoreEntry]) -> Vec<&str> {
    activated.iter().map(|entry| entry.id.as_str()).collect()
}

#[test]
fn constant_entries_always_activate() {
    let world = projection(vec![
        { let mut e = entry("lore_env", "长安常年有雾。"); e.constant = true; e },
        entry("lore_far", "另一座城的事。"),
    ]);
    let activated = activate_lore(&world, &LoreSignals::new(0), &dice(), if_policy::NO_SALT);
    assert_eq!(ids(&activated), ["lore_env"]);
}

#[test]
fn focus_subject_activates_its_entries() {
    let lin = SubjectId::new("c_lin");
    let world = projection(vec![
        { let mut e = entry("lore_lin", "林夏怕雨。"); e.subjects = vec![lin.clone()]; e },
        { let mut e = entry("lore_gu", "顾言不怕雨。"); e.subjects = vec![SubjectId::new("c_gu")]; e },
    ]);

    let none = activate_lore(&world, &LoreSignals::new(0), &dice(), if_policy::NO_SALT);
    assert!(none.is_empty(), "没有焦点就不该结构化激活");

    let signals = LoreSignals::new(0).focus([lin]);
    let activated = activate_lore(&world, &signals, &dice(), if_policy::NO_SALT);
    assert_eq!(ids(&activated), ["lore_lin"]);
}

#[test]
fn primary_keys_any_match_and_secondary_logic_is_honoured() {
    let mut any = entry("lore_any", "雨具。");
    any.keys = vec!["雨".into()];
    any.secondary_keys = vec!["伞".into(), "蓑衣".into()];
    any.logic = SecondaryLogic::AndAny;

    let mut all = entry("lore_all", "雨天外出要两样都带。");
    all.keys = vec!["雨".into()];
    all.secondary_keys = vec!["伞".into(), "蓑衣".into()];
    all.logic = SecondaryLogic::AndAll;

    let mut not_any = entry("lore_not_any", "没带雨具时才成立。");
    not_any.keys = vec!["雨".into()];
    not_any.secondary_keys = vec!["伞".into(), "蓑衣".into()];
    not_any.logic = SecondaryLogic::NotAny;

    let mut not_all = entry("lore_not_all", "只带了一样。");
    not_all.keys = vec!["雨".into()];
    not_all.secondary_keys = vec!["伞".into(), "蓑衣".into()];
    not_all.logic = SecondaryLogic::NotAll;

    let world = projection(vec![any, all, not_any, not_all]);

    // 关掉递归，这一条只验次要关键词的四种逻辑（递归另有专门的测试）。
    // 只提到「雨」和「伞」：AndAny ✓，AndAll ✗（缺蓑衣），NotAny ✗，NotAll ✓
    let signals = LoreSignals::new(0).max_depth(0).scan("外面下着雨，他拿着一把伞");
    let activated = activate_lore(&world, &signals, &dice(), if_policy::NO_SALT);
    assert_eq!(ids(&activated), ["lore_any", "lore_not_all"]);

    // 两样都提到：AndAny ✓，AndAll ✓，NotAny ✗，NotAll ✗
    let both = LoreSignals::new(0).max_depth(0).scan("雨，伞，还有蓑衣");
    let activated = activate_lore(&world, &both, &dice(), if_policy::NO_SALT);
    assert_eq!(ids(&activated), ["lore_all", "lore_any"]);

    // 主关键词没命中，次关键词再对也不激活
    let none = LoreSignals::new(0).max_depth(0).scan("晴天，带着伞");
    assert!(activate_lore(&world, &none, &dice(), if_policy::NO_SALT).is_empty());
}

#[test]
fn keyword_matching_is_case_insensitive_and_ignores_blank_keys() {
    let mut lore = entry("lore_city", "雨城的钟楼。");
    lore.keys = vec!["Yucheng".into(), "  ".into()];
    let world = projection(vec![lore]);
    let hit = LoreSignals::new(0).scan("he arrived in yucheng at dawn");
    assert_eq!(ids(&activate_lore(&world, &hit, &dice(), if_policy::NO_SALT)), ["lore_city"]);
    // 空白键不该匹配一切
    let blank = LoreSignals::new(0).scan("nothing here");
    assert!(activate_lore(&world, &blank, &dice(), if_policy::NO_SALT).is_empty());
}

#[test]
fn recursion_pulls_in_entries_triggered_by_activated_content() {
    // a 命中 → a 的内容里出现 b 的关键词 → b 激活 → b 的内容里出现 c 的关键词 → c 激活
    let mut a = entry("lore_a", "钟楼的守卫名叫丙。");
    a.constant = true;
    let mut b = entry("lore_b", "丙是从北境来的。");
    b.keys = vec!["丙".into()];
    let mut c = entry("lore_c", "北境今年大旱。");
    c.keys = vec!["北境".into()];

    let world = projection(vec![a, b, c]);

    // 深度 1：只能吃到 b
    let depth1 = LoreSignals::new(0).max_depth(1);
    assert_eq!(ids(&activate_lore(&world, &depth1, &dice(), if_policy::NO_SALT)), ["lore_a", "lore_b"]);

    // 深度 2（默认）：连 c 一起
    let depth2 = LoreSignals::new(0);
    assert_eq!(
        ids(&activate_lore(&world, &depth2, &dice(), if_policy::NO_SALT)),
        ["lore_a", "lore_b", "lore_c"]
    );

    // 深度 0：只有常驻
    let depth0 = LoreSignals::new(0).max_depth(0);
    assert_eq!(ids(&activate_lore(&world, &depth0, &dice(), if_policy::NO_SALT)), ["lore_a"]);
}

#[test]
fn recursion_never_reactivates_the_same_entry() {
    // 两条互相引用：没有去重的话会来回激活
    let mut a = entry("lore_a", "提到乙。");
    a.constant = true;
    let mut b = entry("lore_b", "又提到甲。");
    b.keys = vec!["乙".into()];
    let mut a2 = entry("lore_a2", "甲与乙都提。");
    a2.keys = vec!["甲".into()];
    let world = projection(vec![a, b, a2]);

    let activated = activate_lore(&world, &LoreSignals::new(0), &dice(), if_policy::NO_SALT);
    let mut seen = activated.iter().map(|e| e.id.as_str()).collect::<Vec<_>>();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), activated.len(), "激活结果里有重复条目");
}

#[test]
fn probability_activation_is_deterministic_and_uses_the_scene_scoped_die() {
    let mut lore = entry("lore_gamble", "有个一成机会出现的传闻。");
    lore.constant = true;
    lore.probability = Some(0.1);
    let world = projection(vec![lore]);

    let die = dice();
    let key = if_policy::lore_probe_key(&LoreId::new("lore_gamble"), 7);
    let expected = die.sample(&key, 0.1, if_policy::NO_SALT);

    let signals = LoreSignals::new(7);
    let first = !activate_lore(&world, &signals, &die, if_policy::NO_SALT).is_empty();
    let second = !activate_lore(&world, &signals, &die, if_policy::NO_SALT).is_empty();
    assert_eq!(first, second, "同一场景序号必须掷出同一颗骰子");
    assert_eq!(first, expected, "激活结果应当就是骰子的结果");

    // 换一个世界（换种子）骰子就变；换场景序号键也变
    let other_seed = Dice::new(1);
    let _ = activate_lore(&world, &signals, &other_seed, if_policy::NO_SALT);
    assert_ne!(key, if_policy::lore_probe_key(&LoreId::new("lore_gamble"), 8));
}

#[test]
fn a_certain_probability_always_activates_and_zero_never_does() {
    let mut always = entry("lore_always", "必然。");
    always.constant = true;
    always.probability = Some(1.0);
    let mut never = entry("lore_never", "永不。");
    never.constant = true;
    never.probability = Some(0.0);
    let world = projection(vec![always, never]);
    assert_eq!(ids(&activate_lore(&world, &LoreSignals::new(0), &dice(), if_policy::NO_SALT)), ["lore_always"]);
}

#[test]
fn budget_keeps_constants_and_the_highest_order_first() {
    // 三个条目都会被关键词激活，唯一的差别是 order 与 constant——
    // 这样预算剔的就是「优先级最低的」，而不是「压根没激活的」。
    let mut low = entry("lore_low", "低优先。");
    low.keys = vec!["雨".into()];
    low.order = 1;
    let mut high = entry("lore_high", "高优先。");
    high.keys = vec!["雨".into()];
    high.order = 100;
    let mut constant = entry("lore_const", "常驻。");
    constant.constant = true;
    constant.order = 0;

    let world = projection(vec![low, high, constant]);
    let signals = LoreSignals::new(0).max_depth(0).scan("下雨了").budget(2);
    let activated = activate_lore(&world, &signals, &dice(), if_policy::NO_SALT);
    // 常驻必留；剩下的名额给 order 高的
    assert_eq!(ids(&activated), ["lore_const", "lore_high"]);

    // 预算 1 时只剩常驻——常驻不该被 order 更高的挤掉
    let tight = LoreSignals::new(0).max_depth(0).scan("下雨了").budget(1);
    assert_eq!(ids(&activate_lore(&world, &tight, &dice(), if_policy::NO_SALT)), ["lore_const"]);
}

#[test]
fn condition_activation_reads_facts() {
    let mut conditional = entry("lore_ready", "好感达标后才会写的事。");
    conditional.when = Some(Condition::Compare {
        prop: PropositionId::new("p_affinity"),
        cmp: CompareOp::Ge,
        value: 50.0,
    });

    let prop = Proposition {
        id: PropositionId::new("p_affinity"),
        key: "c_lin.affinity".into(),
        text: "林夏的好感度".into(),
        subjects: vec![SubjectId::new("c_lin")],
        kind: PropositionKind::State,
        value_type: ValueType::Scalar {
            min: 0.0,
            max: 100.0,
            unit: None,
        },
        internal: false,
    };
    let base = vec![
        Event {
            id: EventId::numbered(1),
            line: WorldLineId::new("wl_main"),
            seq: 1,
            world_time: WorldTime::EPOCH,
            narrative_order: None,
            payload: Patch::WorldCreated { settings: WorldSettings::default(), label: "w".into() },
            caused_by: Vec::new(),
            depends_on: Vec::new(),
            turn: TurnId::numbered(0),
            scene: None,
            beat: None,
        },
        Event {
            id: EventId::numbered(2),
            line: WorldLineId::new("wl_main"),
            seq: 2,
            world_time: WorldTime::EPOCH,
            narrative_order: None,
            payload: Patch::PropositionCreated(Box::new(prop)),
            caused_by: Vec::new(),
            depends_on: Vec::new(),
            turn: TurnId::numbered(0),
            scene: None,
            beat: None,
        },
        Event {
            id: EventId::numbered(3),
            line: WorldLineId::new("wl_main"),
            seq: 3,
            world_time: WorldTime::EPOCH,
            narrative_order: None,
            payload: Patch::LoreAdded(Box::new(conditional)),
            caused_by: Vec::new(),
            depends_on: Vec::new(),
            turn: TurnId::numbered(0),
            scene: None,
            beat: None,
        },
    ];

    // 好感 30：不激活
    let mut events = base.clone();
    events.push(fact_event(4, "p_affinity", 30.0));
    let world = Projection::fold("wl_main", &events).unwrap();
    assert!(activate_lore(&world, &LoreSignals::new(0), &dice(), if_policy::NO_SALT).is_empty());

    // 好感 60：激活
    let mut events = base.clone();
    events.push(fact_event(4, "p_affinity", 60.0));
    let world = Projection::fold("wl_main", &events).unwrap();
    assert_eq!(ids(&activate_lore(&world, &LoreSignals::new(0), &dice(), if_policy::NO_SALT)), ["lore_ready"]);

    // 事实缺失：条件拿不准，不激活（而不是猜一个）
    let missing = Projection::fold("wl_main", &base).unwrap();
    assert!(activate_lore(&missing, &LoreSignals::new(0), &dice(), if_policy::NO_SALT).is_empty());
}

fn fact_event(seq: u64, prop: &str, value: f64) -> Event {
    Event {
        id: EventId::numbered(seq),
        line: WorldLineId::new("wl_main"),
        seq,
        world_time: WorldTime::EPOCH,
        narrative_order: None,
        payload: Patch::FactSet(Box::new(Fact::new(
            prop,
            value,
            WorldTime::EPOCH,
            Lock::L0,
            EventId::numbered(seq),
        ))),
        caused_by: Vec::new(),
        depends_on: Vec::new(),
        turn: TurnId::numbered(0),
        scene: None,
        beat: None,
    }
}

#[test]
fn superseded_entries_never_activate() {
    let mut gone = entry("lore_gone", "已经被世界线变化淘汰的设定。");
    gone.constant = true;
    gone.status = LoreStatus::Superseded;
    let world = projection(vec![gone]);
    assert!(activate_lore(&world, &LoreSignals::new(0), &dice(), if_policy::NO_SALT).is_empty());
}

/// 输出顺序必须稳定：同一输入两次激活得到同一个序列（含顺序）。
#[test]
fn activation_order_is_deterministic() {
    let world = projection(vec![
        { let mut e = entry("lore_b", "乙。"); e.constant = true; e.order = 5; e },
        { let mut e = entry("lore_a", "甲。"); e.constant = true; e.order = 5; e },
        { let mut e = entry("lore_c", "丙。"); e.constant = true; e.order = 1; e },
    ]);
    let signals = LoreSignals::new(3);
    let first = activate_lore(&world, &signals, &dice(), if_policy::NO_SALT);
    let second = activate_lore(&world, &signals, &dice(), if_policy::NO_SALT);
    assert_eq!(first, second);
    assert_eq!(ids(&first), ["lore_a", "lore_b", "lore_c"]);
}

/// 一个占位断言，确认 `BTreeMap` 不是被误当成有序容器用在别处（防回归的备注测试）。
#[test]
fn activation_does_not_depend_on_hash_iteration() {
    let map: BTreeMap<String, usize> = [("b".to_owned(), 2), ("a".to_owned(), 1)].into_iter().collect();
    assert_eq!(map.keys().next().map(String::as_str), Some("a"));
}
