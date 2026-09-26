//! 播种映射的单元测试。
//!
//! 这里有一条**跨 crate 的契约**要验：`plan` 预先推出的 `EventId` 必须与
//! `Store::append_batch` 实际发出来的号一致。`if-store` 那侧的
//! `event_ids_follow_next_seq` 钉住分配规则，`plan_matches_store_allocation` 钉住这头。

use super::*;
use if_domain::event::Patch;
use if_domain::narrative::LoreEntry;
use if_domain::subject::Subject;

const CARD: &str = r#"{
  "spec": "chara_card_v3",
  "spec_version": "3.0",
  "data": {
    "name": "裴聿",
    "nickname": "阎罗笔",
    "description": "长安县令，三十许。",
    "personality": "克制、寡言。",
    "scenario": "雨夜的长安城，宵禁未解。",
    "first_mes": "雨落在青石板上。",
    "alternate_greetings": ["另一版开场", "又一版开场"],
    "mes_example": "<START>\n{{user}}: 大人。\n裴聿: 嗯。",
    "system_prompt": "你要扮演一个不善言辞的县令。",
    "aliases": ["裴大人"],
    "character_book": {
      "entries": {
        "1": {"comment": "宵禁", "content": "入夜后坊门落锁。", "key": ["长安", "夜"], "constant": true, "order": 10, "position": "before_char"},
        "2": {"comment": "雨簿", "content": "雨簿记各省雨量。", "key": ["雨"], "order": 20, "position": "before_char"},
        "3": {"comment": "停用条目", "content": "这条在卡里是停用的。", "key": ["停"], "enabled": false},
        "4": {"comment": "没有关键词的条目", "content": "谁也不会激活它。"}
      }
    }
  }
}"#;

const LOREBOOK: &str = r#"{
  "spec": "lorebook_v3",
  "data": {
    "name": "雨城",
    "entries": [
      {"keys": ["河"], "content": "河水上涨。", "enabled": true, "insertion_order": 10}
    ]
  }
}"#;

fn world(json: &str) -> ImportedWorld {
    crate::importer::parse_json(json).expect("测试夹具应当能解析")
}

fn plan_of(json: &str) -> SeedPlan {
    plan(&world(json), &SeedContext::new("wl_main", 2))
}

fn subjects(plan: &SeedPlan) -> Vec<Subject> {
    plan.drafts
        .iter()
        .filter_map(|draft| match &draft.payload {
            Patch::SubjectCreated(subject) => Some(subject.as_ref().clone()),
            _ => None,
        })
        .collect()
}

fn lore(plan: &SeedPlan) -> Vec<LoreEntry> {
    plan.drafts
        .iter()
        .filter_map(|draft| match &draft.payload {
            Patch::LoreAdded(entry) => Some(entry.as_ref().clone()),
            _ => None,
        })
        .collect()
}

fn rules(plan: &SeedPlan) -> usize {
    plan.drafts
        .iter()
        .filter(|draft| matches!(draft.payload, Patch::RuleAdded(_)))
        .count()
}

fn notes(plan: &SeedPlan) -> String {
    plan.report.notes.join("\n")
}

// ---------------------------------------------------------------- 角色 → 主体

#[test]
fn character_becomes_a_subject_keeping_its_own_fields() {
    let plan = plan_of(CARD);
    let subjects = subjects(&plan);
    assert_eq!(subjects.len(), 1);
    let subject = &subjects[0];
    // ID 用名字而不是序号，好让它在投影、决策键与日志里可读
    assert_eq!(subject.id.as_str(), "c_裴聿");
    assert_eq!(subject.name, "裴聿");
    assert_eq!(subject.kind, SubjectKind::Character);
    // 卡给的是草稿，还没定型（docs/10 §6）
    assert!(!subject.shaped);
    // nickname 与 aliases 都进别名，且不重复本名
    assert_eq!(subject.aliases, vec!["阎罗笔", "裴大人"]);
    assert_eq!(subject.profile, "长安县令，三十许。\n\n克制、寡言。");
    assert_eq!(subject.voice.as_deref(), Some("<START>\n{{user}}: 大人。\n裴聿: 嗯。"));
}

#[test]
fn duplicate_names_get_distinct_subject_ids() {
    let world = world(
        r#"{"spec":"chara_card_v2","data":{"name":"甲","description":"x",
            "character_book":{"entries":{"1":{"comment":"a","content":"a","key":["a"]}}}}}"#,
    );
    let mut world = world;
    let mut second = world.characters[0].clone();
    second.name = "甲".into();
    second.description = "另一个人".into();
    world.characters.push(second);

    let plan = plan(&world, &SeedContext::new("wl_main", 2));
    let subjects = subjects(&plan);
    assert_eq!(subjects.len(), 2);
    assert_eq!(subjects[0].id.as_str(), "c_甲");
    assert_eq!(subjects[1].id.as_str(), "c_甲-2");
}

#[test]
fn empty_name_falls_back_to_a_visible_placeholder() {
    let mut world = world(r#"{"spec":"chara_card_v2","data":{"name":"占位","description":"x"}}"#);
    world.characters[0].name = "   ".into();
    let plan = plan(&world, &SeedContext::new("wl_main", 2));
    let subjects = subjects(&plan);
    assert_eq!(subjects[0].name, "未命名角色 1");
    // ID 跟着占位名走，所以它在日志里也是可读的
    assert_eq!(subjects[0].id.as_str(), "c_未命名角色-1");
}

#[test]
fn name_without_any_word_character_falls_back_to_a_numbered_id() {
    let mut world = world(r#"{"spec":"chara_card_v2","data":{"name":"占位","description":"x"}}"#);
    world.characters[0].name = "···".into();
    let plan = plan(&world, &SeedContext::new("wl_main", 2));
    assert_eq!(subjects(&plan)[0].id.as_str(), "c_character1");
}

// ---------------------------------------------------------------- scenario / 世界书 → 设定条目

#[test]
fn scenario_becomes_a_constant_world_section_entry() {
    let plan = plan_of(CARD);
    let lore = lore(&plan);
    let scenario = lore
        .iter()
        .find(|entry| entry.title == "裴聿 · 情境")
        .expect("scenario 应当落成一条常驻条目");
    assert_eq!(scenario.content, "雨夜的长安城，宵禁未解。");
    assert!(scenario.constant, "情境是一局的起点，不该等关键词命中");
    assert_eq!(scenario.section, DomainSection::World, "情境讲的是世界，不是角色");
    assert!(scenario.keys.is_empty());
}

#[test]
fn imported_entries_keep_keys_constant_order_and_section() {
    let plan = plan_of(CARD);
    let lore = lore(&plan);
    let curfew = lore.iter().find(|entry| entry.title == "宵禁").unwrap();
    assert_eq!(curfew.content, "入夜后坊门落锁。");
    assert_eq!(curfew.keys, vec!["长安", "夜"]);
    assert!(curfew.constant);
    assert_eq!(curfew.order, 10);
    // position: before_char → 角色段
    assert_eq!(curfew.section, DomainSection::Character);
    assert_eq!(curfew.status, LoreStatus::Active);
    // 酒馆没有可见性语义，全部按 public 进来
    assert_eq!(curfew.visibility, LoreVisibility::Public);
    assert!(curfew.subjects.is_empty(), "「这条属于哪个角色」是语义判断，留给 T-parse");
    assert!(curfew.when.is_none(), "条件激活引用事实，事实来自 T-parse");
}

#[test]
fn disabled_entries_are_skipped_and_counted() {
    let plan = plan_of(CARD);
    assert_eq!(plan.report.lore_disabled, 1);
    assert!(
        !lore(&plan).iter().any(|entry| entry.title == "停用条目"),
        "卡里停用的条目不该在 IF 里活过来"
    );
    assert!(notes(&plan).contains("停用的"));
}

#[test]
fn entry_without_keys_and_not_constant_is_reported_as_unreachable() {
    let plan = plan_of(CARD);
    assert_eq!(plan.report.lore_unreachable, 1);
    assert!(notes(&plan).contains("永远不会进入上下文"));
    // 仍然写进世界：内容不能因为「现在激活不了」就丢掉
    assert!(lore(&plan).iter().any(|entry| entry.title == "没有关键词的条目"));
}

#[test]
fn section_and_logic_and_probability_are_mapped_across_the_boundary() {
    let plan = plan_of(
        r#"{"spec":"lorebook_v3","data":{"name":"雨城","entries":[
            {"keys":["a"],"content":"1","selective":true,"selectiveLogic":1,"probability":60},
            {"keys":["b"],"content":"2","selective":true,"selectiveLogic":2,"probability":100},
            {"keys":["c"],"content":"3","selective":true,"selectiveLogic":3,"probability":0},
            {"keys":["d"],"content":"4","selective":true,"selectiveLogic":0}
        ]}}"#,
    );
    let lore = lore(&plan);
    assert_eq!(lore[0].logic, SecondaryLogic::NotAll);
    assert_eq!(lore[1].logic, SecondaryLogic::NotAny);
    assert_eq!(lore[2].logic, SecondaryLogic::AndAll);
    assert_eq!(lore[3].logic, SecondaryLogic::AndAny);

    // 酒馆是 0–100，IF 的命运骰子是 0–1（docs/06 §3）
    assert_eq!(lore[0].probability, Some(0.6));
    assert_eq!(lore[1].probability, Some(1.0));
    assert_eq!(lore[2].probability, Some(0.0));
    assert_eq!(lore[3].probability, None, "没写概率就是没写，不补默认值");
}

#[test]
fn all_four_sections_survive_the_mapping() {
    for (position, expected) in [
        ("before_char", DomainSection::Character),
        ("after_char", DomainSection::Character),
    ] {
        let json = format!(
            r#"{{"spec":"chara_card_v2","data":{{"name":"甲","description":"x",
                "character_book":{{"entries":{{"1":{{"comment":"a","content":"a","key":["a"],"position":"{position}"}}}}}}}}}}"#
        );
        let plan = plan_of(&json);
        assert_eq!(lore(&plan)[0].section, expected, "position={position}");
    }

    // 独立世界书没有 `position`，落世界段
    let plan = plan_of(LOREBOOK);
    assert_eq!(lore(&plan)[0].section, DomainSection::World);
}

// ---------------------------------------------------------------- 不猜：报告里说清楚

#[test]
fn system_prompt_becomes_a_pending_note_not_a_world_rule() {
    let plan = plan_of(CARD);
    assert_eq!(rules(&plan), 0, "卡里的系统提示词不自动变成世界规则");
    assert!(notes(&plan).contains("system_prompt"));
    assert!(notes(&plan).contains("裁定卡"));
}

#[test]
fn user_macro_is_reported_as_still_unhandled() {
    let plan = plan_of(CARD);
    let notes = notes(&plan);
    assert!(notes.contains("{{user}}"));
    assert!(notes.contains("D14"));
}

#[test]
fn character_section_entries_are_flagged_for_t_parse() {
    let plan = plan_of(CARD);
    assert!(notes(&plan).contains("角色段设定"));
    assert!(notes(&plan).contains("T-parse"));
}

#[test]
fn lorebook_without_characters_says_so() {
    let plan = plan_of(LOREBOOK);
    assert_eq!(plan.report.subjects, 0);
    assert!(notes(&plan).contains("没有角色"));
}

#[test]
fn opening_is_reported_but_never_written_into_the_world() {
    let plan = plan_of(CARD);
    assert_eq!(plan.report.opening.as_deref(), Some("雨落在青石板上。"));
    assert_eq!(plan.report.alternate_openings, 2);
    let all = serde_json::to_string(&plan.drafts.iter().map(|d| &d.payload).collect::<Vec<_>>()).unwrap();
    assert!(
        !all.contains("雨落在青石板上"),
        "开场白是素材，不是世界状态：它不该出现在任何补丁里"
    );
}

#[test]
fn multi_character_world_does_not_pick_a_protagonist() {
    let json = r#"{"spec":"chara_card_v2","data":{"name":"甲","description":"x"}}"#;
    let mut world = world(json);
    let mut second = world.characters[0].clone();
    second.name = "乙".into();
    world.characters.push(second);

    let plan = plan(&world, &SeedContext::new("wl_main", 2));
    assert_eq!(plan.report.subjects, 2);
    assert!(notes(&plan).contains("谁是镜头由场景计划决定"));
}

// ---------------------------------------------------------------- 确定性

#[test]
fn same_input_yields_byte_identical_plans() {
    let render = |plan: &SeedPlan| {
        serde_json::to_string(&plan.drafts.iter().map(|d| &d.payload).collect::<Vec<_>>()).unwrap()
    };
    let a = plan_of(CARD);
    let b = plan_of(CARD);
    assert_eq!(render(&a), render(&b));
    assert_eq!(a.report, b.report);
}

#[test]
fn order_is_subjects_then_scenario_then_card_lore() {
    let plan = plan_of(CARD);
    let kinds: Vec<&str> = plan
        .drafts
        .iter()
        .map(|draft| match &draft.payload {
            Patch::SubjectCreated(_) => "subject",
            Patch::LoreAdded(entry) if entry.title == "裴聿 · 情境" => "scenario",
            Patch::LoreAdded(_) => "lore",
            _ => "other",
        })
        .collect();
    // 卡里有 4 条，其中 1 条停用——停用的那条不写进来
    assert_eq!(kinds, vec!["subject", "scenario", "lore", "lore", "lore"]);
}

#[test]
fn event_ids_are_predicted_from_first_seq() {
    let first = 2;
    let plan = plan(&world(CARD), &SeedContext::new("wl_main", first));
    for (index, draft) in plan.drafts.iter().enumerate() {
        let expected = EventId::numbered(first + index as u64);
        assert_eq!(draft.line.as_str(), "wl_main");
        match &draft.payload {
            Patch::SubjectCreated(subject) => assert_eq!(subject.created_by, expected),
            Patch::LoreAdded(entry) => assert_eq!(entry.source, expected),
            other => panic!("不该出现这种补丁：{other:?}"),
        }
        assert!(
            !draft.payload.event_type().as_str().is_empty(),
            "每条补丁都必须有事件类型"
        );
    }
}

// ---------------------------------------------------------------- 落库契约

/// 播种只是在纸上推号。这里把「纸上」和 `if-store` 实际发出来的号对一遍——
/// 两边不一致的话，`Subject::created_by` 与 `LoreEntry::source` 会指向别的事件，
/// 而且是静默指错。
#[test]
fn plan_matches_store_allocation() {
    use if_store::Store;

    let mut store = Store::open_in_memory().unwrap();
    store
        .create_world("裴聿", if_domain::rule::WorldSettings { seed: 7, ..Default::default() })
        .unwrap();
    let line = store.active_line().unwrap().unwrap();

    let plan = plan(&world(CARD), &SeedContext::new(line.clone(), store.next_seq().unwrap()));
    assert_eq!(store.event_count().unwrap(), 1, "播种前世界里只有 world_created");

    let written = store.append_batch(plan.drafts.clone()).unwrap();
    assert_eq!(written.len(), plan.drafts.len());
    for (index, draft) in plan.drafts.iter().enumerate() {
        match &draft.payload {
            Patch::SubjectCreated(subject) => assert_eq!(subject.created_by, written[index].id),
            Patch::LoreAdded(entry) => assert_eq!(entry.source, written[index].id),
            other => panic!("不该出现这种补丁：{other:?}"),
        }
    }
}

#[test]
fn applying_the_plan_builds_a_world_with_subjects_and_lore() {
    use if_store::Store;

    let mut store = Store::open_in_memory().unwrap();
    store
        .create_world("裴聿", if_domain::rule::WorldSettings { seed: 7, ..Default::default() })
        .unwrap();
    let line = store.active_line().unwrap().unwrap();
    let plan = plan(&world(CARD), &SeedContext::new(line.clone(), store.next_seq().unwrap()));
    store.append_batch(plan.drafts).unwrap();

    let projection = store.load_projection(&line).unwrap();
    assert_eq!(projection.subjects.len(), 1);
    assert_eq!(projection.lore.len(), 4, "情境 + 三条启用的世界书条目（停用的那条不算）");
    assert_eq!(
        projection.lore.values().filter(|entry| entry.constant).count(),
        2,
        "情境与「宵禁」是常驻的"
    );
    // 世界时间是起点，播种不推进时间，也不产生场景与节拍
    assert_eq!(projection.world_time, WorldTime::EPOCH);
    assert!(projection.beats.is_empty());
    assert!(projection.scenes.is_empty());
}
