//! 导入结果 → `if-domain` 的**确定性**映射：把一个世界资产播种成新会话的第一批事件
//! （docs/10 §3 第 4 步、docs/10 §7 第 2 步）。
//!
//! ## 为什么只做确定性映射
//!
//! docs/02 §9.1 把导入定义成**分流**，不是「把世界书变成世界」：每一条条目被分成两半——
//!
//! - **氛围的那一半**（地理、历史、风俗、机构、人物设定）→ 保留为 [`LoreEntry`]，
//!   继续按关键词 / 常驻激活，只提供背景质感。**这一半就是本模块做的**。
//! - **承重的那一半**（会被引用、会被 IF 改写、要参与冲突检测的断言）→ 抽成命题 / 事实 /
//!   规则。**这一半留给 T-parse，本模块刻意不做。**
//!
//! 理由是 P13：抽取断言是语义判断，猜错等于**静默改写世界前提**，而任何改变世界前提的动作
//! 都要过裁定卡。用正则或小模型猜出来的「事实」如果直接进投影，用户连它错了都看不出来。
//! 所以这里只做**字段搬运**：卡里明说了是角色的，就建主体；卡里明说是设定条目的，就建条目。
//! 需要判断的地方一律不猜，改写成 [`SeedReport::notes`] 交给用户与 T-parse。
//!
//! ## 确定性
//!
//! 同样的 [`ImportedWorld`] 加同样的起始 `seq`，得到**逐字节相同**的草稿序列：不读时钟、
//! 不发网络、不用 HashMap。这是重放与命运骰子能成立的前提（docs/12 §7）。
//!
//! ## 事件 ID 为什么要先算出来
//!
//! [`Subject::created_by`] 与 [`LoreEntry::source`] 存的是「引入它的事件 ID」，而播种是
//! **一次成批写入**的——写完再补就晚了。所以 [`plan`] 用 [`SeedContext::first_seq`]
//! 预先推出发号结果：第 `i` 条草稿拿到 `seq = first_seq + i`，ID 是 `EventId::numbered(seq)`。
//! 这条规则由 `if-store` 的 `event_ids_follow_next_seq` 钉住。
//!
//! ## 字段去向
//!
//! | 导入字段 | 去向 |
//! |---|---|
//! | `characters[].name` | [`Subject::name`]（同时决定 `id`） |
//! | `characters[].nickname` / `aliases` | [`Subject::aliases`] |
//! | `characters[].description` + `personality` | [`Subject::profile`] |
//! | `characters[].example_messages` | [`Subject::voice`] |
//! | `characters[].scenario` | 常驻 [`LoreEntry`]（世界段）——它讲的是「现在处在什么情境」 |
//! | `characters[].first_message` | 报告里的 `opening`，**不写进世界状态** |
//! | `characters[].system_prompt` / `post_history_instructions` | 报告的待确认项，**不自动变成规则** |
//! | `lore[]` | [`LoreEntry`]，字段一一对应 |

use std::collections::BTreeSet;

use if_domain::event::Patch;
use if_domain::id::{EventId, LoreId, SubjectId, TurnId, WorldLineId};
use if_domain::narrative::{
    LoreEntry, LoreSection as DomainSection, LoreStatus, LoreVisibility, SecondaryLogic,
};
use if_domain::subject::{Subject, SubjectKind};
use if_domain::value::WorldTime;
use if_store::EventDraft;
use serde::Serialize;

use crate::importer::{ImportedCharacter, ImportedLore, ImportedWorld, LoreLogic, LoreSection};

/// 播种所需的位置信息。
///
/// `first_seq` 必须是**这批事件的第一条将拿到的 `seq`**，即 `Store::next_seq()`
/// （在 `create_world` 之后调用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedContext {
    pub line: WorldLineId,
    /// 世界创建不是回合，所以整批播种记在 `turn_0000`——与 `Store::create_world` 一致。
    pub turn: TurnId,
    pub first_seq: u64,
}

impl SeedContext {
    pub fn new(line: impl Into<WorldLineId>, first_seq: u64) -> Self {
        Self {
            line: line.into(),
            turn: TurnId::numbered(0),
            first_seq,
        }
    }
}

/// 一次播种的结果：要写的事件，以及必须让人看到的那部分。
#[derive(Clone, Debug)]
pub struct SeedPlan {
    pub drafts: Vec<EventDraft>,
    pub report: SeedReport,
}

/// 会话创建时要展示给用户的简报。
///
/// 只放**影响这一局**的事：世界里有谁、有多少设定、哪些设定永远不会激活、卡里有哪些东西
/// 需要人来拿主意。导入期的字段级警告（未知字段、被推迟的 V3 分支）不在重复之列——
/// 那些在导入预览里已经看过一遍了。
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SeedReport {
    /// 世界名（取自导入结果，可能与资产名不同）。
    pub world_name: String,
    /// 播种出来的主体数。
    pub subjects: usize,
    /// 写进世界的设定条目数（不含跳过的停用条目）。
    pub lore: usize,
    /// 其中常驻的条数。
    pub lore_constant: usize,
    /// 既非常驻、又没有关键词的条数——按现在的激活规则永远不会进入上下文。
    pub lore_unreachable: usize,
    /// 卡里被停用（`disable` / `enabled: false`）而**没有**写进世界的条数。
    pub lore_disabled: usize,
    /// 卡里的开场白（`first_message`）。作为素材给客户端显示，不是世界状态。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening: Option<String>,
    /// 备用开场（`alternate_greetings`）的条数。
    pub alternate_openings: usize,
    /// 需要人拿主意的事。
    pub notes: Vec<String>,
}

/// 把导入结果映射成一批事件草稿。
///
/// 顺序是固定的：**先主体，再 `scenario` 条目，最后卡内世界书条目**。同一份输入永远得到
/// 同一个顺序，事件 ID 也就永远是同一组。
pub fn plan(world: &ImportedWorld, ctx: &SeedContext) -> SeedPlan {
    let mut writer = Writer::new(ctx);
    let mut report = SeedReport {
        world_name: world.name.trim().to_owned(),
        ..Default::default()
    };

    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut names: Vec<String> = Vec::new();

    for (index, character) in world.characters.iter().enumerate() {
        let name = display_name(character, index);
        let id = subject_id(character, index, &mut used);
        let mut subject = Subject::new(id, SubjectKind::Character, name.clone());
        // 未定型：卡给的是草稿，LLM 可以通过候选补充它（docs/10 §6）。
        subject.shaped = false;
        subject.aliases = aliases_of(character);
        subject.profile = profile_of(character);
        subject.voice = trimmed(&character.example_messages);
        writer.push(|event| {
            subject.created_by = event.clone();
            Patch::SubjectCreated(Box::new(subject))
        });
        names.push(name);
        report.subjects += 1;
    }

    // `scenario` 讲的是「此刻处在什么情境」，是世界层的东西；常驻是因为它是一局的起点，
    // 不该等关键词命中才进上下文。
    for (character, name) in world.characters.iter().zip(names.iter()) {
        let Some(scenario) = trimmed(&character.scenario) else {
            continue;
        };
        let mut entry = lore_entry(
            writer.next_lore_id(),
            format!("{name} · 情境"),
            scenario,
            DomainSection::World,
        );
        // 常驻；`order` 取 0，情境应当排在别的设定之前进上下文。
        entry.constant = true;
        writer.push(move |event| {
            entry.source = event.clone();
            Patch::LoreAdded(Box::new(entry))
        });
        report.lore += 1;
        report.lore_constant += 1;
    }

    let mut character_section = 0usize;
    for (ordinal, imported) in world.lore.iter().enumerate() {
        if !imported.enabled {
            // 卡里明确停用的条目不该在 IF 里活过来。内容仍在来源附件里，不是丢了。
            report.lore_disabled += 1;
            continue;
        }
        let entry = from_imported_lore(writer.next_lore_id(), ordinal, imported);
        if entry.section == DomainSection::Character {
            character_section += 1;
        }
        if entry.constant {
            report.lore_constant += 1;
        } else if entry.keys.is_empty() {
            // 既非常驻又无关键词：激活器没有入口能选中它。
            report.lore_unreachable += 1;
        }
        writer.push(move |event| {
            let mut entry = entry;
            entry.source = event.clone();
            Patch::LoreAdded(Box::new(entry))
        });
        report.lore += 1;
    }

    report.opening = opening_of(world);
    report.alternate_openings = world
        .characters
        .iter()
        .find(|character| trimmed(&character.first_message).is_some())
        .map(|character| character.alternate_greetings.len())
        .unwrap_or(0);

    report.notes = notes(world, &report, character_section);
    SeedPlan {
        drafts: writer.finish(),
        report,
    }
}

/// 卡里的开场白（`first_message`）。
///
/// 它是**素材**，不是世界状态——所以它不进任何补丁，只随简报给客户端。
/// 创建与恢复两条路都要它（恢复时没有播种，但会话的第一条消息仍要补上），所以单独一个函数。
pub fn opening_of(world: &ImportedWorld) -> Option<String> {
    world
        .characters
        .iter()
        .find_map(|character| trimmed(&character.first_message))
}

/// 组装待确认项。每一句都要说清「为什么这里不动手」。
fn notes(world: &ImportedWorld, report: &SeedReport, character_section: usize) -> Vec<String> {
    let mut notes = Vec::new();

    if report.subjects == 0 {
        notes.push(
            "来源里没有角色：世界里还没有可模拟的主体。独立世界书要先把主角补上，才能进入回合。"
                .to_owned(),
        );
    } else if report.subjects > 1 {
        notes.push(format!(
            "有 {} 个角色。播种不替你指定主角——谁是镜头由场景计划决定（docs/02 §10.1）。",
            report.subjects
        ));
    }

    let unconfirmed: Vec<&str> = world
        .characters
        .iter()
        .filter(|character| {
            !character.system_prompt.trim().is_empty()
                || !character.post_history_instructions.trim().is_empty()
        })
        .map(|character| character.name.as_str())
        .collect();
    if !unconfirmed.is_empty() {
        notes.push(format!(
            "{} 的 system_prompt / post_history_instructions 没有自动变成世界规则——\
             那是写给「聊天助手」的，算不算世界规则要人判断；改世界前提得过裁定卡（P13）。",
            unconfirmed.join("、")
        ));
    }

    if world.macros.iter().any(|macro_| macro_ == "{{user}}") {
        notes.push(
            "卡里用了 {{user}}：本步原样保留，还没替换成主角。按 D14，主角由世界推演确定，\
             用户不扮演主角，只改写主角的命运。"
                .to_owned(),
        );
    }

    if report.lore_unreachable > 0 {
        notes.push(format!(
            "{} 条设定既非常驻、又没有关键词，按现在的激活规则永远不会进入上下文（docs/08 §5）。\
             要么给关键词，要么改常驻，要么等 T-parse 判断它的承重断言。",
            report.lore_unreachable
        ));
    }

    if report.lore_disabled > 0 {
        notes.push(format!(
            "{} 条设定在卡里是停用的（disable / enabled: false），没有写进世界；内容仍在来源附件里。",
            report.lore_disabled
        ));
    }

    if character_section > 0 {
        notes.push(format!(
            "有 {character_section} 条角色段设定。它们可以关联到主体（主体聚焦时直接激活，\
             比关键词可靠），但「这条属于哪个角色」是语义判断，留给 T-parse（docs/02 §9.1）。"
        ));
    }

    if report.opening.is_none() && report.subjects > 0 {
        notes.push("卡里没有开场白。第一个场景要从零起——先给一个 IF 或让角色自己动。".to_owned());
    }

    notes
}

// ---------------------------------------------------------------- 写入

/// 按顺序发号、按顺序推草稿的小工具。
///
/// `push` 把事件 ID 交给构造闭包，是因为有些补丁的载荷里存着引入它的事件 ID，
/// 而这个 ID 只有在写入那一刻才定得下来。
struct Writer {
    line: WorldLineId,
    turn: TurnId,
    seq: u64,
    drafts: Vec<EventDraft>,
    lore_seq: u64,
}

impl Writer {
    fn new(ctx: &SeedContext) -> Self {
        Self {
            line: ctx.line.clone(),
            turn: ctx.turn.clone(),
            seq: ctx.first_seq,
            drafts: Vec::new(),
            lore_seq: 0,
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

    /// 设定条目的 ID 自己一条序号，与事件号无关——条目是静态资料，不是「发生过的事」。
    fn next_lore_id(&mut self) -> LoreId {
        self.lore_seq += 1;
        LoreId::numbered(self.lore_seq)
    }

    fn finish(self) -> Vec<EventDraft> {
        self.drafts
    }
}

// ---------------------------------------------------------------- 字段映射

fn display_name(character: &ImportedCharacter, index: usize) -> String {
    let name = character.name.trim();
    if name.is_empty() {
        // 导入层会拦掉没有名字的 v2 / v3 卡，所以这里基本不可达；但「不可达」会腐烂，
        // 所以还是给一个明显是占位的名字，而不是让空名字流进投影。
        format!("未命名角色 {}", index + 1)
    } else {
        name.to_owned()
    }
}

/// 主体 ID：`c_<名字 slug>`，重名时退到 `-2` / `-3`。
///
/// 用名字而不是序号，是为了让 ID 在投影、决策键和日志里可读（规则见 [`crate::slug`]）。
/// 名字改了 ID 就变了——播种只发生一次，改名是之后的事。
fn subject_id(
    character: &ImportedCharacter,
    index: usize,
    used: &mut BTreeSet<String>,
) -> SubjectId {
    let name = display_name(character, index);
    let base = crate::slug::slug(&name);
    let base = if base.is_empty() {
        format!("character{}", index + 1)
    } else {
        base
    };
    let mut candidate = format!("c_{base}");
    let mut n = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("c_{base}-{n}");
        n += 1;
    }
    SubjectId::new(candidate)
}

/// 别名：`nickname` 在前，`aliases` 在后；去掉空白项与和本名重复的项。
fn aliases_of(character: &ImportedCharacter) -> Vec<String> {
    let name = character.name.trim();
    let mut out: Vec<String> = Vec::new();
    let mut add = |value: Option<&str>| {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return;
        };
        if value == name || out.iter().any(|existing| existing == value) {
            return;
        }
        out.push(value.to_owned());
    };
    add(character.nickname.as_deref());
    for alias in &character.aliases {
        add(Some(alias.as_str()));
    }
    out
}

/// 自然语言设定：`description` 与 `personality` 按顺序拼接。
///
/// 两者都在讲「这个人是谁」，拆开存没有别处会用。`scenario` 不进这里——它讲的是
/// 「现在处在什么情境」，属于世界层，走常驻设定条目。
fn profile_of(character: &ImportedCharacter) -> String {
    [
        character.description.as_str(),
        character.personality.as_str(),
    ]
    .into_iter()
    .map(str::trim)
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n")
}

fn from_imported_lore(id: LoreId, ordinal: usize, imported: &ImportedLore) -> LoreEntry {
    let title = if imported.title.trim().is_empty() {
        format!("设定 {}", ordinal + 1)
    } else {
        imported.title.trim().to_owned()
    };
    let mut entry = lore_entry(id, title, imported.content.clone(), domain_section(imported.section));
    entry.keys = imported.keys.clone();
    entry.secondary_keys = imported.secondary_keys.clone();
    entry.logic = secondary_logic(imported.logic);
    entry.constant = imported.constant;
    entry.order = imported.order;
    entry.probability = probability_of(imported.probability);
    entry
}

/// 设定条目的基座：只有「什么段、叫什么、写了什么」是每条都必须给的，
/// 其余按最保守的默认（不常驻、无关键词、无概率）。
///
/// 不常驻是刻意的默认——条目该不该进上下文应当由关键词或人工标定决定，
/// 而不是由「构造时忘了设」决定。
fn lore_entry(id: LoreId, title: String, content: String, section: DomainSection) -> LoreEntry {
    LoreEntry {
        id,
        title,
        content,
        keys: Vec::new(),
        secondary_keys: Vec::new(),
        logic: SecondaryLogic::AndAny,
        // 结构化激活由 `if-lore` 在激活时按场景计划填，播种不猜（docs/02 §9.1）。
        subjects: Vec::new(),
        // 条件激活引用事实；事实来自 T-parse，这里还没有。
        when: None,
        constant: false,
        order: 0,
        section,
        // 酒馆没有可见性这个概念。全部按 public 进来，secret 是 IF 自己新增的语义，
        // 由用户在裁定卡或世界库里标（docs/08 §5）。
        visibility: LoreVisibility::Public,
        known_by: Vec::new(),
        probability: None,
        status: LoreStatus::Active,
        // 由 `Writer::push` 在写入那一刻覆盖成真实事件 ID。
        source: EventId::new("evt_pending"),
    }
}

fn secondary_logic(logic: LoreLogic) -> SecondaryLogic {
    match logic {
        LoreLogic::AndAny => SecondaryLogic::AndAny,
        LoreLogic::NotAll => SecondaryLogic::NotAll,
        LoreLogic::NotAny => SecondaryLogic::NotAny,
        LoreLogic::AndAll => SecondaryLogic::AndAll,
    }
}

fn domain_section(section: LoreSection) -> DomainSection {
    match section {
        LoreSection::World => DomainSection::World,
        LoreSection::Character => DomainSection::Character,
        LoreSection::Scene => DomainSection::Scene,
        LoreSection::Style => DomainSection::Style,
    }
}

/// 酒馆的 `probability` 是 0–100 的百分比，IF 的命运骰子是 0–1（`die < p` 即发生，
/// docs/06 §3）。**必须换算**：不换算的话 `60` 会被当成必中还富余 60 倍。
fn probability_of(raw: Option<f64>) -> Option<f64> {
    raw.map(|percent| (percent / 100.0).clamp(0.0, 1.0))
}

fn trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests;
