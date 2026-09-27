//! 作者世界：把**一份写好的世界**直接映射成事件（docs/10 §1「手动撰写」的完整形态）。
//!
//! ## 它和 [`crate::seed`] 是什么关系
//!
//! 两条路都产出「建会话时要写的第二批事件」，但输入完全不同：
//!
//! | | [`crate::seed`]（导入） | 本模块（作者世界） |
//! |---|---|---|
//! | 输入 | 酒馆卡 / 世界书的导入结果 | 一份手写的世界包 JSON |
//! | 产出 | **只有**主体 + 设定条目 | 主体 + 命题 + 事实 + 规则 + 设定 + 故事线 |
//! | 为什么要少做 | 承重断言要**猜**，猜错等于静默改写世界前提（P13） | 每一条都是作者**明说**的，没有猜 |
//!
//! 换句话说：`seed` 少做是因为**不能猜**；这里做全是因为**不用猜**。一张导进来的卡
//! 里「上游水库的数据被人改了」到底算命题还是氛围，只有 T-parse 能判断；而作者世界里
//! 这句话就是一条 `Proposition` + 一条 `Fact`，写在文件里，没有歧义。
//!
//! ## 为什么需要一个格式而不是直接建世界
//!
//! 命题是 IF 的着力点：**没有命题，IF 就没有东西可改写**。一个只有主体的世界跑得起回合，
//! 但每个候选都拿不到稳定决策键（`if-policy` 只能退回候选 ID 兜底，跨世界线不可复现，
//! docs/03 §8）——那正是 2026-09-27 真机上跑出「T-impact 没有产出任何提议」的根子
//! （docs/16 §3）。所以「适合 IF 的世界」必须能把命题写进来，这就需要一个承载它的格式。
//!
//! ## 确定性
//!
//! 与 `seed` 同一条契约：同样的世界包加同样的起始 `seq`，得到**逐字节相同**的草稿序列。
//! 不读时钟、不发网络、不用 HashMap。事件 ID 一律**先算号再写**——`Subject::created_by` /
//! `Fact::source` / `WorldRule::source` / `LoreEntry::source` 存的是「引入它的事件 ID」，
//! 而整批是一次成批写入的（见 [`crate::seed`] 的同名说明，以及 `Store::next_seq`）。
//!
//! ## 写入顺序
//!
//! **主体 → 命题 → 事实 → 规则 → 设定 → 故事线**。读起来就是「世界里有谁 → 能断言什么 →
//! 此刻什么为真 → 世界怎么运作 → 氛围 → 悬念」。顺序固定，所以 ID 永远可复现。

use std::collections::BTreeSet;
use std::path::Path;

use if_domain::event::Patch;
use if_domain::id::{EventId, LoreId, PropositionId, RuleId, SubjectId, ThreadId};
use if_domain::narrative::{
    LoreEntry, LoreSection, LoreStatus, LoreVisibility, SecondaryLogic, Thread, ThreadStage,
};
use if_domain::rule::{
    ActiveWindow, Condition, Invariant, InvariantCheck, Mechanic, Trigger, WorldRule, WorldSettings,
};
use if_domain::state::{DependsOn, Fact};
use if_domain::subject::{Proposition, Subject, SubjectKind, Tier};
use if_domain::value::{Lock, Value, Visibility, WorldTime};
use if_store::{EventDraft, Store};
use serde::{Deserialize, Serialize};

use crate::seed::SeedContext;

/// 一份作者世界：可以直接落成一个 `.ifworld` 的完整内容。
///
/// 字段名与 `if-domain` 的领域类型一一对应，只有一处刻意的不同：**凡是「由哪个事件引入」
/// 的字段（`created_by` / `source`）都不在文件里**——它们要到写入那一刻才知道，由本模块
/// 回填。写进 JSON 只会逼作者填一个假值。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredWorld {
    pub name: String,
    #[serde(default)]
    pub genre: String,
    #[serde(default)]
    pub summary: String,
    /// 世界设置（命运骰子种子、叙事风格、机制步长……）。
    ///
    /// 给定时**种子是钉死的**，所以同一份世界包永远掷出同一串骰子——示例世界与测试都需要
    /// 这一点。不给就现生成一个（见 [`crate::world_worker::new_world_settings`]）。
    #[serde(default)]
    pub settings: Option<WorldSettings>,
    #[serde(default)]
    pub subjects: Vec<AuthoredSubject>,
    #[serde(default)]
    pub propositions: Vec<Proposition>,
    #[serde(default)]
    pub facts: Vec<AuthoredFact>,
    #[serde(default)]
    pub rules: Vec<AuthoredRule>,
    #[serde(default)]
    pub lore: Vec<AuthoredLore>,
    #[serde(default)]
    pub threads: Vec<AuthoredThread>,
}

/// 主体。与 [`Subject`] 的差别只有：没有 `created_by`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredSubject {
    pub id: SubjectId,
    pub kind: SubjectKind,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default = "tier_active")]
    pub tier: Tier,
    /// 是否已定型（docs/10 §6）。默认**未定型**：作者写的是草稿，LLM 可以补。
    #[serde(default)]
    pub shaped: bool,
}

/// 事实。与 [`Fact`] 的差别：`source` 由本模块回填；世界时间用「第几天」写，
/// 比裸分钟数好读；`depends_on` 只收命题（作者写不出、也不该写事件的 ID）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredFact {
    pub prop: PropositionId,
    /// 取值。可以直接写标量：`true` / `22.1` / `"晴"`；也可以写完整形态
    /// `{"kind":"number","value":22.1}`。两种都要能读，因为写世界时前者顺手、
    /// 机器生成时后者稳妥。
    #[serde(deserialize_with = "value_from_json")]
    pub value: Value,
    /// 这条事实的锁定等级。默认 **L0**：作者写下的是世界此刻的样子，不是公理；
    /// 要把它钉住就在 IF 里改，或在文件里显式写 `L2` / `L3`。
    #[serde(default = "lock_free")]
    pub lock: Lock,
    /// **必填**：一条事实要不要让叙事看见，是作者必须拿的主意，不该有一个静默默认。
    /// 世界前提（水位、在不在场）通常 `public`；秘密（谁篡改了数据）用 `secret`。
    pub visibility: Visibility,
    /// 从第几天起为真（`WorldTime` 从纪元起算的**天**数，可以是负数）。
    #[serde(default)]
    pub valid_from_days: i64,
    /// L1 保护期截止的场景序号（docs/01 §7）。
    #[serde(default)]
    pub protected_until: Option<u64>,
    /// 这条事实依赖哪些命题。回溯修复要沿它做传递闭包（docs/03 §7）。
    #[serde(default)]
    pub depends_on_props: Vec<PropositionId>,
}

/// 世界规则。与 [`WorldRule`] 的差别：`source` 回填；`active` 用天数写；
/// `lock` 默认 **L2**——docs/01 §6 规定规则型一律锚定，绝大多数规则不该由作者再选一次。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredRule {
    pub id: RuleId,
    pub text: String,
    #[serde(default = "lock_anchored")]
    pub lock: Lock,
    #[serde(default)]
    pub active_from_days: i64,
    #[serde(default)]
    pub active_to_days: Option<i64>,
    /// 适用的主体或地域；空表示全局（docs/02 §6）。
    #[serde(default)]
    pub scope: Vec<SubjectId>,
    #[serde(default)]
    pub invariants: Vec<Invariant>,
    #[serde(default)]
    pub mechanics: Vec<Mechanic>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub constraints: Vec<String>,
    /// 边界解释，供裁定卡确认（docs/01 §11）。
    #[serde(default)]
    pub boundaries: Vec<String>,
}

/// 设定条目。与 [`LoreEntry`] 的差别：`source` 回填；`status` 不收——新世界里没有
/// 「已被取代」的条目，能写进来的就一定是 Active。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredLore {
    pub id: LoreId,
    #[serde(default)]
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub secondary_keys: Vec<String>,
    #[serde(default = "logic_and_any")]
    pub logic: SecondaryLogic,
    #[serde(default)]
    pub subjects: Vec<SubjectId>,
    #[serde(default)]
    pub when: Option<Condition>,
    #[serde(default)]
    pub constant: bool,
    #[serde(default)]
    pub order: i32,
    #[serde(default = "section_world")]
    pub section: LoreSection,
    #[serde(default = "visibility_public")]
    pub visibility: LoreVisibility,
    #[serde(default)]
    pub known_by: Vec<SubjectId>,
    #[serde(default)]
    pub probability: Option<f64>,
}

/// 故事线。与 [`Thread`] 的差别：压力、节奏、上次推进的序号这些**运转参数**有默认值，
/// 作者只需要写「这条线在问什么、什么阶段、保护到哪」。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredThread {
    pub id: ThreadId,
    pub title: String,
    pub question: String,
    #[serde(default)]
    pub stakes: String,
    #[serde(default)]
    pub subjects: Vec<SubjectId>,
    pub stage: ThreadStage,
    #[serde(default)]
    pub protected_until: Option<ThreadStage>,
    #[serde(default = "thread_pressure")]
    pub pressure: f64,
    #[serde(default)]
    pub last_advanced: u64,
    /// 期望每隔几个场景推进一次（docs/06 §6 的 `overdue_bonus` 用它）。
    #[serde(default = "thread_cadence")]
    pub cadence: f64,
}

fn tier_active() -> Tier {
    Tier::Active
}
fn lock_anchored() -> Lock {
    Lock::L2
}
fn lock_free() -> Lock {
    Lock::L0
}
fn logic_and_any() -> SecondaryLogic {
    SecondaryLogic::AndAny
}
fn section_world() -> LoreSection {
    LoreSection::World
}
fn visibility_public() -> LoreVisibility {
    LoreVisibility::Public
}
fn thread_pressure() -> f64 {
    0.2
}
fn thread_cadence() -> f64 {
    3.0
}

/// 把标量写成 [`Value`]。见 [`AuthoredFact::value`]。
fn value_from_json<'de, D>(de: D) -> Result<Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let raw = serde_json::Value::deserialize(de)?;
    match raw {
        serde_json::Value::Bool(value) => Ok(Value::Bool(value)),
        serde_json::Value::Number(number) => number
            .as_f64()
            .map(Value::Number)
            .ok_or_else(|| serde::de::Error::custom("数值超出 f64 能表示的范围")),
        serde_json::Value::String(text) => Ok(Value::Text(text)),
        // 完整形态 `{"kind": "...", "value": ...}` 也收。
        other => serde_json::from_value(other).map_err(serde::de::Error::custom),
    }
}

/// 一次播种的结果：要写的事件，以及值得作者知道的事。
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AuthoredReport {
    pub world_name: String,
    pub subjects: usize,
    pub propositions: usize,
    pub facts: usize,
    pub rules: usize,
    pub lore: usize,
    pub threads: usize,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AuthoredPlan {
    pub drafts: Vec<EventDraft>,
    pub report: AuthoredReport,
}

/// 把一份作者世界映射成一批事件草稿。**先校验再映射**——悬空引用会让整批事件在
/// `append_batch` 折叠时被拒绝（那里是「全部成功或全部不写」），但报错会是一句
/// 「引用了不存在的命题」，指不出是文件里哪一行写错了。
pub fn plan(world: &AuthoredWorld, ctx: &SeedContext) -> Result<AuthoredPlan, String> {
    validate(world)?;
    let mut writer = Writer::new(ctx);
    let mut report = AuthoredReport {
        world_name: world.name.trim().to_owned(),
        ..Default::default()
    };

    for authored in &world.subjects {
        let name = authored.name.trim().to_owned();
        writer.push(|event| {
            Patch::SubjectCreated(Box::new(Subject {
                id: authored.id.clone(),
                kind: authored.kind,
                name,
                aliases: authored.aliases.clone(),
                profile: authored.profile.trim().to_owned(),
                voice: authored
                    .voice
                    .as_deref()
                    .map(str::trim)
                    .filter(|voice| !voice.is_empty())
                    .map(str::to_owned),
                tier: authored.tier,
                shaped: authored.shaped,
                created_by: event.clone(),
            }))
        });
        report.subjects += 1;
    }

    for proposition in &world.propositions {
        writer.push(|_| Patch::PropositionCreated(Box::new(proposition.clone())));
        report.propositions += 1;
    }

    for authored in &world.facts {
        writer.push(|event| {
            Patch::FactSet(Box::new(Fact {
                prop: authored.prop.clone(),
                value: authored.value.clone(),
                valid_from: WorldTime::from_days(authored.valid_from_days),
                valid_to: None,
                lock: authored.lock,
                protected_until: authored.protected_until,
                visibility: authored.visibility,
                depends_on: authored
                    .depends_on_props
                    .iter()
                    .cloned()
                    .map(|prop| DependsOn::Fact { prop })
                    .collect(),
                source: event.clone(),
            }))
        });
        report.facts += 1;
    }

    for authored in &world.rules {
        writer.push(|event| {
            Patch::RuleAdded(Box::new(WorldRule {
                id: authored.id.clone(),
                text: authored.text.trim().to_owned(),
                lock: authored.lock,
                source: event.clone(),
                active: ActiveWindow {
                    from: WorldTime::from_days(authored.active_from_days),
                    to: authored.active_to_days.map(WorldTime::from_days),
                },
                scope: authored.scope.clone(),
                invariants: authored.invariants.clone(),
                mechanics: authored.mechanics.clone(),
                triggers: authored.triggers.clone(),
                constraints: authored.constraints.clone(),
                boundaries: authored.boundaries.clone(),
            }))
        });
        report.rules += 1;
    }

    for authored in &world.lore {
        writer.push(|event| {
            Patch::LoreAdded(Box::new(LoreEntry {
                id: authored.id.clone(),
                title: authored.title.trim().to_owned(),
                content: authored.content.clone(),
                keys: authored.keys.clone(),
                secondary_keys: authored.secondary_keys.clone(),
                logic: authored.logic,
                subjects: authored.subjects.clone(),
                when: authored.when.clone(),
                constant: authored.constant,
                order: authored.order,
                section: authored.section,
                visibility: authored.visibility,
                known_by: authored.known_by.clone(),
                probability: authored.probability,
                status: LoreStatus::Active,
                source: event.clone(),
            }))
        });
        report.lore += 1;
    }

    for authored in &world.threads {
        writer.push(|_| Patch::ThreadOpened(Box::new(authored.to_thread())));
        report.threads += 1;
    }

    report.notes = notes(world, &report);
    Ok(AuthoredPlan {
        drafts: writer.finish(),
        report,
    })
}

/// 把世界包的内容写进一个**已经有世界线**的 store（`create_world` 已经跑过）。
///
/// 拆出来是为了两种用法都能走同一条代码路径：落了文件的（[`write`]）与内存库的（测试）。
pub fn sow(store: &mut Store, world: &AuthoredWorld) -> Result<AuthoredReport, String> {
    let line = store
        .active_line()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "世界里没有活跃世界线：先调用 create_world".to_owned())?;
    let first_seq = store.next_seq().map_err(|e| e.to_string())?;
    let plan = plan(world, &SeedContext::new(line, first_seq))?;
    // `append_batch` 从 `first_seq` 起顺序发号，所以 `plan` 预先推出的 ID 就是实际 ID。
    store
        .append_batch(plan.drafts)
        .map_err(|e| format!("写入作者世界失败：{e}"))?;
    Ok(plan.report)
}

/// 把一份世界包落成一个完整的 `.ifworld`。
///
/// **不覆盖已存在的文件**：世界文件是历史，命名又是「名字 + 纳秒」，同名的可能性极低——
/// 真撞上了，宁可报错让人换一个路径，也不要悄悄把别人的世界抹掉。
pub fn write(path: &Path, world: &AuthoredWorld) -> Result<AuthoredReport, String> {
    if path.exists() {
        return Err(format!("目标文件已存在，不覆盖：{}", path.display()));
    }
    let settings = world.settings.clone().unwrap_or_else(default_settings);
    let mut store = Store::open(path).map_err(|e| format!("无法创建世界文件：{e}"))?;
    store
        .create_world(world.name.trim(), settings)
        .map_err(|e| format!("写入 world_created 失败：{e}"))?;
    sow(&mut store, world)
}

/// 作者世界不给设置时的兜底：默认值 + 一个固定种子。
///
/// 刻意**不用时钟**——`world_worker::new_world_settings` 用纳秒做种子（每个会话都不同），
/// 那对交互是对的，对一份要能重放的世界包是错的。这里固定成 0，作者想换就在
/// `settings.seed` 里明写。
fn default_settings() -> WorldSettings {
    WorldSettings {
        seed: 0,
        ..Default::default()
    }
}

/// 读一份世界包：解析 + 校验。
pub fn load(path: &Path) -> Result<AuthoredWorld, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("读不到世界包 {}：{e}", path.display()))?;
    from_json(&text).map_err(|e| format!("{}：{e}", path.display()))
}

/// 从 JSON 文本解析一份世界包，并校验引用完整性。
pub fn from_json(text: &str) -> Result<AuthoredWorld, String> {
    let world: AuthoredWorld =
        serde_json::from_str(text).map_err(|e| format!("世界包不是合法的 JSON：{e}"))?;
    validate(&world)?;
    Ok(world)
}

// ---------------------------------------------------------------- 校验

/// 校验世界包。**一次报出全部问题**，而不是撞到第一个就返回——作者改一版只想改一轮。
///
/// 引擎侧还有一道保险：`append_batch` 在折叠投影时也会拒绝悬空引用。但那道保险的报错
/// 长这样：「引用了不存在的命题 p_xxx」——它不会说这是文件第几条、是哪个字段。作者格式
/// 的报错必须指得出文件里的位置，所以校验放在这里，而不是指望存储层兜。
pub fn validate(world: &AuthoredWorld) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();

    if world.name.trim().is_empty() {
        problems.push("世界没有名字（name）".to_owned());
    }

    unique(world.subjects.iter().map(|s| s.id.as_str()), "主体", &mut problems);
    unique(
        world.propositions.iter().map(|p| p.id.as_str()),
        "命题",
        &mut problems,
    );
    unique(world.rules.iter().map(|r| r.id.as_str()), "规则", &mut problems);
    unique(world.lore.iter().map(|l| l.id.as_str()), "设定条目", &mut problems);
    unique(
        world.threads.iter().map(|t| t.id.as_str()),
        "故事线",
        &mut problems,
    );
    // 规范键是决策键的一部分（docs/03 §8）：撞键会让两个不同的命题共用同一颗骰子。
    unique(
        world.propositions.iter().map(|p| p.key.as_str()),
        "命题规范键",
        &mut problems,
    );

    let subjects: BTreeSet<&str> = world.subjects.iter().map(|s| s.id.as_str()).collect();
    let propositions: BTreeSet<&str> = world.propositions.iter().map(|p| p.id.as_str()).collect();

    for proposition in &world.propositions {
        if proposition.key.trim().is_empty() {
            problems.push(format!("命题 {} 没有规范键（key）", proposition.id));
        }
        require(
            &subjects,
            proposition.subjects.iter().map(|id| id.as_str()),
            "命题",
            proposition.id.as_str(),
            "主体",
            &mut problems,
        );
    }

    for fact in &world.facts {
        if !propositions.contains(fact.prop.as_str()) {
            problems.push(format!("事实引用了不存在的命题：{}", fact.prop));
        }
        require(
            &propositions,
            fact.depends_on_props.iter().map(|id| id.as_str()),
            "事实",
            fact.prop.as_str(),
            "命题",
            &mut problems,
        );
    }

    for rule in &world.rules {
        require(
            &subjects,
            rule.scope.iter().map(|id| id.as_str()),
            "规则",
            rule.id.as_str(),
            "主体",
            &mut problems,
        );
        for mechanic in &rule.mechanics {
            if !propositions.contains(mechanic.target.as_str()) {
                problems.push(format!(
                    "规则 {} 的机制 `{}` 作用在不存在的命题上：{}",
                    rule.id, mechanic.id, mechanic.target
                ));
            }
        }
        for invariant in &rule.invariants {
            if let InvariantCheck::NumericUnchanged { prop, .. } = &invariant.check {
                if !propositions.contains(prop.as_str()) {
                    problems.push(format!(
                        "规则 {} 的不变量 `{}` 挂在不存在的命题上：{prop}",
                        rule.id, invariant.id
                    ));
                }
            }
        }
        for trigger in &rule.triggers {
            require_condition(&propositions, &trigger.when, "规则", rule.id.as_str(), &mut problems);
        }
    }

    for entry in &world.lore {
        require(
            &subjects,
            entry.subjects.iter().map(|id| id.as_str()),
            "设定条目",
            entry.id.as_str(),
            "主体",
            &mut problems,
        );
        require(
            &subjects,
            entry.known_by.iter().map(|id| id.as_str()),
            "设定条目",
            entry.id.as_str(),
            "主体",
            &mut problems,
        );
        if let Some(when) = &entry.when {
            require_condition(&propositions, when, "设定条目", entry.id.as_str(), &mut problems);
        }
    }

    for thread in &world.threads {
        require(
            &subjects,
            thread.subjects.iter().map(|id| id.as_str()),
            "故事线",
            thread.id.as_str(),
            "主体",
            &mut problems,
        );
    }

    if problems.is_empty() {
        return Ok(());
    }
    Err(format!(
        "世界包 `{}` 有 {} 处问题：\n- {}",
        world.name.trim(),
        problems.len(),
        problems.join("\n- ")
    ))
}

fn unique<'a>(ids: impl Iterator<Item = &'a str>, what: &str, problems: &mut Vec<String>) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            problems.push(format!("{what} ID 重复：{id}"));
        }
    }
}

fn require<'a>(
    known: &BTreeSet<&str>,
    refs: impl IntoIterator<Item = &'a str>,
    owner: &str,
    owner_id: &str,
    target: &str,
    problems: &mut Vec<String>,
) {
    for reference in refs {
        if !known.contains(reference) {
            problems.push(format!("{owner} {owner_id} 引用了不存在的{target}：{reference}"));
        }
    }
}

fn require_condition(
    propositions: &BTreeSet<&str>,
    condition: &Condition,
    owner: &str,
    owner_id: &str,
    problems: &mut Vec<String>,
) {
    for prop in condition_props(condition) {
        if !propositions.contains(prop.as_str()) {
            problems.push(format!("{owner} {owner_id} 的条件引用了不存在的命题：{prop}"));
        }
    }
}

/// 递归收出条件树里引用到的命题。
fn condition_props(condition: &Condition) -> Vec<&PropositionId> {
    match condition {
        Condition::Compare { prop, .. } => vec![prop],
        Condition::All { all } => all.iter().flat_map(condition_props).collect(),
        Condition::Any { any } => any.iter().flat_map(condition_props).collect(),
        Condition::Not { not } => condition_props(not),
    }
}

// ---------------------------------------------------------------- 写入

/// 按顺序发号、按顺序推草稿。与 [`crate::seed`] 的同名小工具同一套契约——
/// 事件 ID 只有到写入那一刻才定得下来，而有些载荷里存着它。
struct Writer {
    line: if_domain::id::WorldLineId,
    turn: if_domain::id::TurnId,
    seq: u64,
    drafts: Vec<EventDraft>,
}

impl Writer {
    fn new(ctx: &SeedContext) -> Self {
        Self {
            line: ctx.line.clone(),
            turn: ctx.turn.clone(),
            seq: ctx.first_seq,
            drafts: Vec::new(),
        }
    }

    fn push(&mut self, build: impl FnOnce(&EventId) -> Patch) -> EventId {
        let id = EventId::numbered(self.seq);
        self.seq += 1;
        let patch = build(&id);
        self.drafts.push(EventDraft::new(
            self.line.clone(),
            self.turn.clone(),
            WorldTime::EPOCH,
            patch,
        ));
        id
    }

    fn finish(self) -> Vec<EventDraft> {
        self.drafts
    }
}

impl AuthoredThread {
    fn to_thread(&self) -> Thread {
        Thread {
            id: self.id.clone(),
            title: self.title.trim().to_owned(),
            question: self.question.trim().to_owned(),
            stakes: self.stakes.trim().to_owned(),
            subjects: self.subjects.clone(),
            stage: self.stage,
            protected_until: self.protected_until,
            pressure: self.pressure.clamp(0.0, 1.0),
            last_advanced: self.last_advanced,
            cadence: self.cadence,
        }
    }
}

// ---------------------------------------------------------------- 提示

/// 组装「值得作者知道的事」。与 `seed` 的 `notes` 同一个作用：说清楚哪些地方**没做**、
/// 为什么，而不是让作者事后从行为里猜。
fn notes(world: &AuthoredWorld, report: &AuthoredReport) -> Vec<String> {
    let mut notes = Vec::new();

    if report.subjects == 0 {
        notes.push(
            "世界没有任何主体：回合跑得起来，但没有谁可以被聚焦，场景计划的视角人物会没人可选。"
                .to_owned(),
        );
    }
    if report.propositions == 0 {
        notes.push(
            "世界没有任何命题：IF 没有东西可改写，候选也只能拿候选 ID 兜底当决策键——\
             本回合跑得通，但跨世界线不可复现（docs/03 §8）。适合 IF 的世界至少要有一条命题。"
                .to_owned(),
        );
    }
    if report.threads == 0 {
        notes.push("世界没有故事线：导演没有可以推进的悬念，世界会显得在原地打转。".to_owned());
    }
    let secret_facts = world
        .facts
        .iter()
        .filter(|fact| fact.visibility != Visibility::Public)
        .count();
    if secret_facts > 0 {
        notes.push(format!(
            "{secret_facts} 条事实不是 public：叙事视图默认看不见它们，这正是「秘密」的用法——\
             观测会把它变成线索而不是答案（docs/01 §13）。"
        ));
    }
    if world.settings.is_none() {
        notes.push(
            "没有给 settings：命运骰子种子取 0。同一份世界包会永远掷出同一串骰子；\
             想要每次不同的开局，就在 settings.seed 里写一个种子。"
                .to_owned(),
        );
    }

    notes
}

#[cfg(test)]
mod tests;
