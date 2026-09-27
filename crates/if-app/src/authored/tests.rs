//! 作者世界的单元测试：解析、校验、确定性、事件 ID 契约。
//!
//! 端到端那一半（建世界 → 跑完整回合）在 `tests/authored_world.rs`——
//! 那里用的是 `tests/fixtures/worlds/` 下的测试世界包。

use super::*;

const MINIMAL: &str = r#"{
  "name": "小店",
  "genre": "日常",
  "summary": "一间只有两个人的店。",
  "settings": { "seed": 7, "narrative_style": "文学叙事", "narrative_pov": "third_limited",
                 "mechanic_step": "day", "director_style": "均衡", "mode": "sandbox" },
  "subjects": [
    { "id": "c_a", "kind": "character", "name": "阿夏", "profile": "掌柜", "tier": "foreground" },
    { "id": "c_b", "kind": "character", "name": "阿冬", "voice": "话少", "shaped": true }
  ],
  "propositions": [
    { "id": "p_open", "key": "c_a.shop_open", "text": "店开着", "subjects": ["c_a"],
      "kind": "state", "value_type": { "type": "bool" }, "internal": false },
    { "id": "p_mood", "key": "c_a.mood", "text": "阿夏的心情", "subjects": ["c_a"],
      "kind": "state", "value_type": { "type": "enum", "values": ["好", "坏"] }, "internal": true }
  ],
  "facts": [
    { "prop": "p_open", "value": true, "visibility": "public" },
    { "prop": "p_mood", "value": "好", "visibility": "private", "valid_from_days": 1 }
  ],
  "rules": [
    { "id": "rule_1", "text": "天黑后不再营业", "scope": ["c_b"],
      "triggers": [ { "id": "t1", "when": { "op": "compare", "prop": "p_open", "cmp": "eq", "value": 0.0 },
                      "produces": { "kind": "candidate", "text": "打烊" } } ] }
  ],
  "lore": [
    { "id": "lore_1", "title": "小店", "content": "街角的一间店。", "keys": ["店"], "constant": true }
  ],
  "threads": [
    { "id": "thr_1", "title": "谁会先走", "question": "两个人谁先离开", "stage": "seeded",
      "protected_until": "climax", "subjects": ["c_a", "c_b"] }
  ]
}"#;

fn minimal() -> AuthoredWorld {
    from_json(MINIMAL).expect("最小世界包应当解析通过")
}

#[test]
fn a_minimal_package_parses_and_reports_what_it_holds() {
    let world = minimal();
    assert_eq!(world.name, "小店");
    // 标量取值与默认值都要落到领域类型上。
    assert_eq!(world.facts[0].value, Value::Bool(true));
    assert_eq!(world.facts[1].valid_from_days, 1);
    // 规则没写 lock → docs/01 §6 的规则型默认 L2。
    assert_eq!(world.rules[0].lock, Lock::L2);
    // 故事线的运转参数有默认值，作者不用写。
    assert_eq!(world.threads[0].cadence, 3.0);
    assert_eq!(world.threads[0].pressure, 0.2);
    // 主体写了 tier → 保留；没写 → Active。
    assert_eq!(world.subjects[0].tier, Tier::Foreground);
    assert_eq!(world.subjects[1].tier, Tier::Active);
    assert!(world.subjects[1].shaped);
}

#[test]
fn a_tagged_value_form_is_accepted_too() {
    let world = from_json(
        r#"{"name":"x","propositions":[{"id":"p","key":"k","text":"t",
            "kind":"state","value_type":{"type":"bool"},"internal":false}],
            "facts":[{"prop":"p","value":{"kind":"number","value":3.5},"visibility":"public"}]}"#,
    )
    .unwrap();
    assert_eq!(world.facts[0].value, Value::Number(3.5));
}

#[test]
fn dangling_references_name_the_offending_field() {
    let error = from_json(&MINIMAL.replace("\"prop\": \"p_open\"", "\"prop\": \"p_typo\""))
        .expect_err("引用了不存在的命题就该报错");
    assert!(error.contains("p_typo"), "{error}");
    assert!(error.contains("不存在的命题"), "{error}");
    // 报错要指得出是哪一类对象里的哪一条，而不是笼统地说「世界坏了」。
    assert!(error.contains("事实"), "{error}");
}

#[test]
fn duplicate_proposition_keys_are_rejected_before_anything_is_written() {
    // 规范键是决策键的一部分：撞键会让两条不同的命题共用同一颗骰子（docs/03 §8）。
    let world = MINIMAL.replace("\"key\": \"c_a.mood\"", "\"key\": \"c_a.shop_open\"");
    let error = from_json(&world).expect_err("规范键撞车就该报错");
    assert!(error.contains("规范键"), "{error}");
}

#[test]
fn duplicate_ids_are_rejected() {
    let world = MINIMAL.replace("\"id\": \"c_b\"", "\"id\": \"c_a\"");
    let error = from_json(&world).expect_err("主体 ID 重复就该报错");
    assert!(error.contains("重复"), "{error}");
}

#[test]
fn a_typo_in_a_field_name_is_not_silently_ignored() {
    // `deny_unknown_fields` 的价值就在这里：把 `propositions` 打成 `propostions`，
    // 若被静默忽略，作者看到的是一个「没有命题的世界」，而不是一句「你打错了」。
    let error = from_json(&MINIMAL.replace("\"propositions\"", "\"propostions\""))
        .expect_err("未知字段必须报错");
    assert!(error.contains("propostions"), "{error}");
}

#[test]
fn planning_is_deterministic() {
    let world = minimal();
    let ctx = SeedContext::new("wl_main", 2);
    let first = plan(&world, &ctx).unwrap();
    let second = plan(&world, &ctx).unwrap();
    assert_eq!(first.drafts, second.drafts, "同一份世界包必须逐字节复现");
}

#[test]
fn event_ids_follow_next_seq_and_are_prefilled_into_the_payloads() {
    // 与 `if-store::tests::event_ids_follow_next_seq` 同一套契约，但验的是**本模块**
    // 推出来的号与实际写出来的号一致：`created_by` / `source` 存的是「引入它的事件 ID」。
    let mut store = Store::open_in_memory().unwrap();
    store
        .create_world("小店", WorldSettings::default())
        .unwrap();
    let first_seq = store.next_seq().unwrap();

    let world = minimal();
    let report = sow(&mut store, &world).unwrap();

    assert_eq!(report.subjects, 2);
    assert_eq!(report.propositions, 2);
    assert_eq!(report.facts, 2);
    assert_eq!(report.rules, 1);
    assert_eq!(report.lore, 1);
    assert_eq!(report.threads, 1);

    let line = store.active_line().unwrap().unwrap();
    let projection = store.load_projection(&line).unwrap();
    assert_eq!(projection.subjects.len(), 2);
    assert_eq!(projection.propositions.len(), 2);
    assert_eq!(projection.facts.len(), 2);
    assert_eq!(projection.rules.len(), 1);
    assert_eq!(projection.threads.len(), 1);
    assert_eq!(projection.lore.len(), 1);

    // 主体按写入顺序拿到 first_seq + 0 / + 1。
    assert_eq!(
        projection.subjects[&SubjectId::new("c_a")].created_by,
        EventId::numbered(first_seq)
    );
    assert_eq!(
        projection.subjects[&SubjectId::new("c_b")].created_by,
        EventId::numbered(first_seq + 1)
    );
    // 设定条目在场，且 `source` 指向它自己的那个事件，不是占位值。
    let lore = projection.lore.values().next().unwrap();
    assert_ne!(lore.source, EventId::new("evt_pending"));
    assert_eq!(
        lore.source,
        EventId::numbered(first_seq + 2 + 2 + 2 + 1)
    );
    // 事实落到了投影里，值也没走样。
    assert_eq!(
        projection.facts[&PropositionId::new("p_open")].value,
        Value::Bool(true)
    );
}

#[test]
fn a_world_without_propositions_says_so_in_the_report() {
    let world = from_json(r#"{"name":"空","subjects":[{"id":"c_a","kind":"character","name":"甲"}]}"#)
        .unwrap();
    let plan = plan(&world, &SeedContext::new("wl_main", 2)).unwrap();
    assert!(plan
        .report
        .notes
        .iter()
        .any(|note| note.contains("没有任何命题") && note.contains("决策键")));
    assert!(plan.report.notes.iter().any(|note| note.contains("没有故事线")));
}

#[test]
fn writing_a_world_refuses_to_overwrite_an_existing_file() {
    let path = std::env::temp_dir().join(format!(
        "if-authored-{}.ifworld",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let world = minimal();
    write(&path, &world).unwrap();
    let error = write(&path, &world).expect_err("已存在的世界文件不该被覆盖");
    assert!(error.contains("已存在"), "{error}");
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------- 端到端：作者世界跑一个完整回合
//
// 这一段是「**适合 IF**」这句话的验收，而不是「能跑起来」的验收。
//
// 导入路径的产物只有主体与设定条目（`seed` 的文档说明了为什么不猜），于是一个刚导入的
// 世界**没有任何命题**——2026-09-27 真机上「T-impact 没有产出任何提议」就是这么来的
// （docs/16 §3）。作者世界把命题写进文件，候选因此能落在真实命题上，决策键跨世界线
// 可复现（docs/03 §8）。下面两个测试**成对**：同样的剧本、同样的裁判，唯一的差别是有没有
// 命题，结果就是「有稳定决策键」与「只能拿候选 ID 兜底」的区别。

use std::sync::atomic::AtomicBool;

use if_agent::provider::scripted::{ScriptedProvider, ScriptedTurn};
use if_judge::StubJudge;
use serde_json::json;

use crate::turn_runner::run_with;
use crate::world_worker::WorldWorker;

const BEAT_1: &str = "第一拍：江面起了雾。";
const BEAT_2: &str = "第二拍：顾行把船钱拍在船板上。";
const BEAT_3: &str = "第三拍：燕七没有说话。";

fn temp_world_path() -> std::path::PathBuf {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("if-authored-turn-{id}.ifworld"))
}

fn ferry_fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("worlds")
        .join("ferry.world.json")
}

/// 五个任务的剧本，顺序与 `run_if_turn` 一致。`observation` 为 `false` 时省掉回收那一步——
/// 没有命题的世界里，`record_observation` 指不出任何键。
fn script(observation: bool) -> Vec<ScriptedTurn> {
    let mut turns = vec![
        ScriptedTurn::tool(
            "propose_candidate",
            json!({
                "subject": "顾行",
                "content": "顾行当晚找到燕七，出双倍船钱要过江",
                "internal": false,
                "shape": "occurs",
                "options": [],
                "depends_on": [],
                "based_on": [],
                "affects": ["c_yan.ferry_ready"]
            }),
        ),
        ScriptedTurn::text("已提交候选。"),
        ScriptedTurn::tool(
            "propose_scene",
            json!({
                "summary": "雾夜渡口，顾行要过江",
                "threads": [],
                "resolves": [],
                "erupts": [],
                "focus": ["顾行"],
                "present": ["顾行"]
            }),
        ),
        ScriptedTurn::text("已提交场景候选。"),
        ScriptedTurn::tool(
            "submit_scene_plan",
            json!({
                "goal": "顾行把过江的事说出口",
                "pov": "顾行",
                "focus": ["顾行"],
                "present": ["顾行"],
                "time_span": "当晚",
                "required_beats": [],
                "stop_condition": "燕七开口",
                "reveal_facts": [],
                "reveal_lore": [],
                "proposed_changes": []
            }),
        ),
        ScriptedTurn::text("已提交场景计划。"),
        ScriptedTurn::text(format!("{BEAT_1}<<<BEAT>>>{BEAT_2}<<<BEAT>>>{BEAT_3}")),
    ];
    if observation {
        turns.push(ScriptedTurn::tool(
            "record_observation",
            json!({
                "prop": "c_yan.water_level",
                "value": 5.2,
                "internal": false,
                "text": "他把船缆又收紧了一扣。"
            }),
        ));
    }
    turns.push(ScriptedTurn::text("已回收。"));
    turns
}

/// 与 `if-pipeline` 的测试夹具同口径：约束全过、节拍全干净。
fn clean_judge() -> StubJudge {
    let mut judge = StubJudge::new();
    for template in [
        "q.cand.knowledge_gap",
        "q.scene.resolves_thread",
        "q.beat.violates_fact",
        "q.beat.violates_rule",
        "q.beat.knowledge_leak",
        "q.beat.forbidden_resolution",
        "q.beat.reveals_secret",
        "q.beat.stop_reached",
        "q.observe.leaks_secret",
    ] {
        judge = judge.noul_for_template(template, 0.0);
    }
    for template in ["q.cand.in_character", "q.behavior.occurs"] {
        judge = judge.noul_for_template(template, 1.0);
    }
    judge
}

/// 建世界 → 确认一张裁定卡 → 跑一个完整回合。返回 `(回合报告, 世界文件)`。
fn play_one_turn(world: &AuthoredWorld, observation: bool) -> (crate::turn_runner::TurnReport, std::path::PathBuf) {
    let path = temp_world_path();
    write(&path, world).expect("应当写出世界");

    // 裁定卡是回合的起点：用户的原话存在卡里。走的是真机同一条路（WorldWorker）。
    {
        let handle = WorldWorker::open(path.clone()).expect("应当能打开刚写出的世界");
        handle
            .world
            .submit_if("IF 顾行今夜一定要过江".into())
            .expect("应当能提交 IF");
        handle.world.confirm_if(None).expect("应当能确认");
    }

    let mut store = Store::open(&path).unwrap();
    let provider = ScriptedProvider::new(script(observation));
    let report = run_with(
        &mut store,
        &path,
        &provider,
        &provider,
        &clean_judge(),
        crate::settings::DEFAULT_STRICTNESS,
        &AtomicBool::new(false),
    )
    .expect("干净回合应当跑通");
    drop(store);
    (report, path)
}

#[test]
fn an_authored_world_plays_a_full_turn_with_stable_decision_keys() {
    let world = load(&ferry_fixture()).expect("夹具世界包应当读通");
    assert_eq!(world.name, "渡口");

    let (report, path) = play_one_turn(&world, true);

    // ---- 世界真的动了：三拍都放行才算推进
    assert!(report.advanced, "三拍都放行了才算推进");
    assert_eq!(report.beats.len(), 3);
    assert_eq!(report.beats[0].text, BEAT_1);
    assert!(!report.committed.is_empty(), "推进必须写事件");

    // ---- 五个任务按顺序跑过
    let names: Vec<&str> = report.tasks.iter().map(|task| task.task.as_str()).collect();
    assert_eq!(
        names,
        vec!["T-impact", "T-scenes", "T-plan", "T-render", "T-extract"]
    );

    // ---- ★ 验收点：候选落在真实命题上，没有退化成拿候选 ID 兜底。
    // 导入世界只有主体与设定条目，这条警告必然出现（见下面那个对照测试）。
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.contains("稳定决策键")),
        "作者世界的候选不该退化：{:?}",
        report.warnings
    );

    // ---- 命题 / 规则 / 故事线真的进了投影：这正是作者世界与导入世界的分界。
    let projection = &report.snapshot.projection;
    assert_eq!(projection.subjects.len(), 2);
    assert_eq!(projection.propositions.len(), 3);
    assert_eq!(projection.facts.len(), 3);
    assert_eq!(projection.rules.len(), 1);
    assert_eq!(projection.threads.len(), 1);

    // ---- 场景计划是给人看的：视角人物是名字，不是 `c_gu`
    let scene = report.scene.as_ref().expect("有场景");
    assert_eq!(scene.pov, "顾行");

    let _ = std::fs::remove_file(&path);
}

/// 对照：同样的剧本、同样的裁判，只把命题抽走——候选立刻拿不到稳定决策键。
///
/// 这个测试的作用不是「验证一个 bug」，而是把「命题是 IF 的着力点」钉成一条可执行的事实：
/// 谁要是哪天把作者世界的命题支持拿掉，或让 `affects` 静默失效，它会变红。
#[test]
fn a_world_without_propositions_falls_back_to_unstable_keys() {
    let hollow = from_json(
        r#"{"name":"空壳","subjects":[
            {"id":"c_gu","kind":"character","name":"顾行"},
            {"id":"c_yan","kind":"character","name":"燕七"}]}"#,
    )
    .unwrap();

    let (report, path) = play_one_turn(&hollow, false);

    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("稳定决策键")),
        "没有命题的世界必须如实报出决策键不稳定，而不是假装一切正常：{:?}",
        report.warnings
    );
    assert_eq!(report.snapshot.projection.propositions.len(), 0);

    let _ = std::fs::remove_file(&path);
}
