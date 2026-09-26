//! Single-world actor. The SQLite connection is created and used only on its owner thread.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use if_domain::event::{DigestionStrategy, IfInjection, IfKind, IfScope, Patch, TimeAnchor};
use if_domain::id::TurnId;
use if_domain::projection::Projection;
use if_domain::rule::WorldSettings;
use if_domain::turn::{IfCardStatus, IfConflict, IfRulingCard, TurnKind, TurnRecord};
use if_domain::value::Lock;
use if_store::Store;
use serde::Serialize;

use crate::importer::ImportedWorld;
use crate::seed::{self, SeedContext, SeedReport};

enum Request {
    Snapshot(Sender<Result<WorldSnapshot, String>>),
    SubmitIf {
        input: String,
        reply: Sender<Result<WorldSnapshot, String>>,
    },
    SubmitIfDraft {
        draft: crate::if_parser::IfDraft,
        reply: Sender<Result<WorldSnapshot, String>>,
    },
    ConfirmIf {
        input: Option<String>,
        reply: Sender<Result<WorldSnapshot, String>>,
    },
    ReinterpretIf {
        reply: Sender<Result<WorldSnapshot, String>>,
    },
    CancelIf {
        reply: Sender<Result<WorldSnapshot, String>>,
    },
}

pub struct WorldWorker {
    requests: Option<Sender<Request>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for WorldWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldWorker").finish_non_exhaustive()
    }
}

/// 新建一个世界时要写进去的东西：名字、设置，以及要播种的世界材料。
///
/// `world` 是 `Option`：世界可以不带材料建出来（测试与恢复路径用），但**客户端不该走这条路**——
/// docs/10 §3 要求新建会话先选一个已保存的世界。
pub struct CreateRequest {
    pub label: String,
    pub settings: WorldSettings,
    pub world: Option<ImportedWorld>,
}

impl CreateRequest {
    pub fn new(label: impl Into<String>, settings: WorldSettings, world: Option<ImportedWorld>) -> Self {
        Self {
            label: label.into(),
            settings,
            world,
        }
    }
}

/// 打开或新建世界的结果：句柄 + 打开那一刻的快照 + 新建时的播种简报。
///
/// 快照跟着一起返回，是因为打开与新建本来就要在 store 线程上算一遍；
/// 让调用方再问一次 `snapshot()` 只是多一趟往返。
#[derive(Debug)]
pub struct WorldHandle {
    pub world: WorldWorker,
    pub snapshot: WorldSnapshot,
    /// 只有新建时有。
    pub seed: Option<SeedReport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldSnapshot {
    pub path: String,
    pub label: String,
    pub active_line_id: String,
    pub head_seq: u64,
    pub event_count: u64,
    pub projection: Projection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_if: Option<IfRulingCard>,
}

impl WorldWorker {
    /// 新建一个世界，并按 `request.world` 播种（docs/10 §3）。
    pub fn create(path: PathBuf, request: CreateRequest) -> Result<WorldHandle, String> {
        if path.exists() {
            return Err(format!("世界文件已存在：{}", path.display()));
        }
        Self::start(path, Some(request))
    }

    pub fn open(path: PathBuf) -> Result<WorldHandle, String> {
        if !path.is_file() {
            return Err(format!("世界文件不存在：{}", path.display()));
        }
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map_or(true, |e| !e.eq_ignore_ascii_case("ifworld"))
        {
            return Err("请选择 .ifworld 世界文件".into());
        }
        Self::start(path, None)
    }

    fn start(path: PathBuf, create: Option<CreateRequest>) -> Result<WorldHandle, String> {
        let (requests, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_path = path.clone();
        let thread = thread::Builder::new()
            .name("if-world-worker".into())
            .spawn(move || {
                let opened = (|| {
                    let mut store = Store::open(&thread_path).map_err(|e| e.to_string())?;
                    let mut report = None;
                    if let Some(create) = create {
                        store
                            .create_world(&create.label, create.settings)
                            .map_err(|e| e.to_string())?;
                        if let Some(world) = create.world.as_ref() {
                            report = Some(sow(&mut store, world)?);
                        }
                    }
                    let initial = snapshot(&store, &thread_path)?;
                    Ok::<_, String>((store, initial, report))
                })();

                match opened {
                    Ok((mut store, initial, report)) => {
                        if ready_tx.send(Ok((initial, report))).is_err() {
                            return;
                        }
                        while let Ok(request) = receiver.recv() {
                            match request {
                                Request::Snapshot(reply) => {
                                    let _ = reply.send(snapshot(&store, &thread_path));
                                }
                                Request::SubmitIf { input, reply } => {
                                    let result = submit_if(&mut store, &thread_path, input);
                                    let _ = reply.send(result);
                                }
                                Request::SubmitIfDraft { draft, reply } => {
                                    let result = submit_if_draft(&mut store, &thread_path, draft);
                                    let _ = reply.send(result);
                                }
                                Request::ConfirmIf { input, reply } => {
                                    let result = confirm_if(&mut store, &thread_path, input);
                                    let _ = reply.send(result);
                                }
                                Request::ReinterpretIf { reply } => {
                                    let result = reinterpret_if(&mut store, &thread_path);
                                    let _ = reply.send(result);
                                }
                                Request::CancelIf { reply } => {
                                    let result = cancel_if(&mut store, &thread_path);
                                    let _ = reply.send(result);
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            })
            .map_err(|e| format!("无法启动世界工作线程：{e}"))?;

        match ready_rx
            .recv()
            .map_err(|e| format!("世界工作线程未就绪：{e}"))?
        {
            Ok((snapshot, seed)) => Ok(WorldHandle {
                world: Self {
                    requests: Some(requests),
                    thread: Some(thread),
                },
                snapshot,
                seed,
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }

    pub fn snapshot(&self) -> Result<WorldSnapshot, String> {
        let (reply, response) = mpsc::channel();
        self.requests
            .as_ref()
            .ok_or_else(|| "世界工作线程已停止".to_owned())?
            .send(Request::Snapshot(reply))
            .map_err(|e| format!("世界工作线程已停止：{e}"))?;
        response
            .recv()
            .map_err(|e| format!("读取世界状态失败：{e}"))?
    }

    pub fn submit_if(&self, input: String) -> Result<WorldSnapshot, String> {
        let (reply, response) = mpsc::channel();
        self.requests
            .as_ref()
            .ok_or_else(|| "世界工作线程已停止".to_owned())?
            .send(Request::SubmitIf { input, reply })
            .map_err(|e| format!("世界工作线程已停止：{e}"))?;
        response
            .recv()
            .map_err(|e| format!("提交 IF 失败：{e}"))?
    }

    pub fn submit_if_draft(&self, draft: crate::if_parser::IfDraft) -> Result<WorldSnapshot, String> {
        let (reply, response) = mpsc::channel();
        self.requests.as_ref().ok_or_else(|| "世界工作线程已停止".to_owned())?
            .send(Request::SubmitIfDraft { draft, reply })
            .map_err(|e| format!("世界工作线程已停止：{e}"))?;
        response.recv().map_err(|e| format!("提交 IF 草案失败：{e}"))?
    }

    pub fn confirm_if(&self, input: Option<String>) -> Result<WorldSnapshot, String> {
        let (reply, response) = mpsc::channel();
        self.requests.as_ref().ok_or_else(|| "世界工作线程已停止".to_owned())?
            .send(Request::ConfirmIf { input, reply })
            .map_err(|e| format!("世界工作线程已停止：{e}"))?;
        response.recv().map_err(|e| format!("确认 IF 失败：{e}"))?
    }

    pub fn cancel_if(&self) -> Result<WorldSnapshot, String> {
        let (reply, response) = mpsc::channel();
        self.requests.as_ref().ok_or_else(|| "世界工作线程已停止".to_owned())?
            .send(Request::CancelIf { reply })
            .map_err(|e| format!("世界工作线程已停止：{e}"))?;
        response.recv().map_err(|e| format!("取消 IF 失败：{e}"))?
    }

    pub fn reinterpret_if(&self) -> Result<WorldSnapshot, String> {
        let (reply, response) = mpsc::channel();
        self.requests.as_ref().ok_or_else(|| "世界工作线程已停止".to_owned())?
            .send(Request::ReinterpretIf { reply })
            .map_err(|e| format!("世界工作线程已停止：{e}"))?;
        response.recv().map_err(|e| format!("重释 IF 失败：{e}"))?
    }
}

impl Drop for WorldWorker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 新世界的设置：默认值 + 一个刚生成的世界种子（docs/10 §5）。
///
/// 种子属于世界、不属于全局设置，所以在这里现生成，而不是从全局设置里读。
pub fn new_world_settings() -> WorldSettings {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    WorldSettings {
        seed,
        ..Default::default()
    }
}

/// 把一个世界资产播种成新世界的第一批事件（docs/10 §3）。
///
/// 播种必须在 store 线程上做：`seed::plan` 要先读 `next_seq` 再写入，中间不能让别的写插进来。
/// 这也正是 `Store::next_seq` 存在的理由——`Subject::created_by` 与 `LoreEntry::source`
/// 存的是引入它的事件 ID，而播种是一次成批写入的。
fn sow(store: &mut Store, world: &ImportedWorld) -> Result<SeedReport, String> {
    let line = store
        .active_line()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "新世界里没有活跃世界线".to_owned())?;
    let first_seq = store.next_seq().map_err(|e| e.to_string())?;
    let plan = seed::plan(world, &SeedContext::new(line, first_seq));
    // `append_batch` 从 `first_seq` 起按顺序发号，所以 `plan` 预先推出的 ID 就是实际 ID。
    // 「发号规则」由 if-store 的 `event_ids_follow_next_seq` 钉住，
    // 「推出来的号与实际写出来的一致」由 if-app 的 `plan_matches_store_allocation` 钉住。
    store
        .append_batch(plan.drafts)
        .map_err(|e| format!("播种世界失败：{e}"))?;
    Ok(plan.report)
}

fn snapshot(store: &Store, path: &Path) -> Result<WorldSnapshot, String> {
    let line_id = store
        .active_line()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "文件不是有效的 IF 世界：缺少活跃世界线".to_owned())?;
    let line = store
        .world_line(&line_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("活跃世界线不存在：{line_id}"))?;
    let projection = store.load_projection(&line_id).map_err(|e| e.to_string())?;
    if projection.settings.is_none() {
        return Err("文件不是有效的 IF 世界：缺少 world_created 事件".into());
    }
    Ok(WorldSnapshot {
        path: path.display().to_string(),
        label: line.label,
        active_line_id: line_id.to_string(),
        head_seq: line.head_seq,
        event_count: store.event_count().map_err(|e| e.to_string())?,
        projection,
        pending_if: store
            .turns_of_line(&line_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .rev()
            .find_map(|turn| turn.ruling_card.filter(|card| card.status == IfCardStatus::Pending)),
    })
}

fn submit_if(store: &mut Store, path: &Path, input: String) -> Result<WorldSnapshot, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("IF 输入不能为空".into());
    }
    let line = store
        .active_line()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "当前没有活跃世界线".to_owned())?;
    if store
        .turns_of_line(&line)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|turn| turn.ruling_card.as_ref().is_some_and(|card| card.status == IfCardStatus::Pending))
    {
        return Err("已有待确认的 IF 裁定卡，请先确认或取消".into());
    }
    let parsed = crate::if_parser::parse(input)?;
    submit_if_parsed(store, path, parsed)
}

fn submit_if_draft(store: &mut Store, path: &Path, parsed: crate::if_parser::IfDraft) -> Result<WorldSnapshot, String> {
    submit_if_parsed(store, path, parsed)
}

fn submit_if_parsed(store: &mut Store, path: &Path, parsed: crate::if_parser::IfDraft) -> Result<WorldSnapshot, String> {
    let line = store.active_line().map_err(|e| e.to_string())?.ok_or_else(|| "当前没有活跃世界线".to_owned())?;
    if store.turns_of_line(&line).map_err(|e| e.to_string())?.iter().any(|turn| turn.ruling_card.as_ref().is_some_and(|card| card.status == IfCardStatus::Pending)) { return Err("已有待确认的 IF 裁定卡，请先确认或取消".into()); }
    let projection = store.load_projection(&line).map_err(|e| e.to_string())?;
    let turn = TurnId::numbered(store.event_count().map_err(|e| e.to_string())? + 1);
    let injection = injection_from_draft(&parsed);
    let conflicts = find_conflicts(&projection, &injection);
    let mut warnings = parsed.warnings;
    if !conflicts.is_empty() {
        warnings.push(format!("本地预检发现 {} 条可能冲突的既有事实；确认前应先重释，失败后再按锁定等级处理。", conflicts.len()));
    }
    let mut record = TurnRecord::new(turn.clone(), line.clone(), TurnKind::If, projection.world_time);
    record.input = Some(parsed.input.clone());
    record.ruling_card = Some(IfRulingCard {
        turn,
        status: IfCardStatus::Pending,
        injection,
        warnings,
        conflicts,
        confirmed_event: None,
    });
    store.save_turn(&record).map_err(|e| e.to_string())?;
    snapshot(store, path)
}

fn find_conflicts(projection: &if_domain::projection::Projection, injection: &IfInjection) -> Vec<IfConflict> {
    let (base, polarity) = conflict_signature(&injection.core);
    projection
        .injections
        .iter()
        .filter_map(|(event, existing)| {
            let (existing_base, existing_polarity) = conflict_signature(&existing.core);
            if !base.is_empty() && base == existing_base && polarity != existing_polarity {
                Some(IfConflict {
                    event: event.clone(),
                    existing_core: existing.core.clone(),
                    lock: existing.lock,
                    reason: "核心命题相同但否定极性相反".into(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn conflict_signature(core: &str) -> (String, bool) {
    const NEGATIONS: [&str; 8] = ["无法", "不能", "没有", "不是", "并非", "从未", "未曾", "不"];
    let mut text = core.trim().to_owned();
    let mut negative = false;
    for word in NEGATIONS {
        if text.contains(word) {
            text = text.replace(word, "");
            negative = !negative;
        }
    }
    (text.chars().filter(|c| !c.is_whitespace()).collect(), negative)
}

fn injection_from_draft(draft: &crate::if_parser::IfDraft) -> IfInjection {
    let kind = match draft.kind {
        crate::if_parser::ParsedIfKind::State => IfKind::State,
        crate::if_parser::ParsedIfKind::Belief => IfKind::Belief,
        crate::if_parser::ParsedIfKind::Rule => IfKind::Rule,
        crate::if_parser::ParsedIfKind::Occurrence => IfKind::Occurrence,
        crate::if_parser::ParsedIfKind::Truth => IfKind::Truth,
        crate::if_parser::ParsedIfKind::Retcon => IfKind::Retcon,
        crate::if_parser::ParsedIfKind::Unknown => IfKind::Occurrence,
    };
    let time_anchor = match draft.time_anchor {
        crate::if_parser::ParsedTimeAnchor::Now => TimeAnchor::Now,
        crate::if_parser::ParsedTimeAnchor::Past => TimeAnchor::Past,
        crate::if_parser::ParsedTimeAnchor::Always => TimeAnchor::Always,
    };
    let scope = match draft.scope.as_str() {
        "global" => IfScope::Global,
        _ => IfScope::Individual,
    };
    let lock = match draft.suggested_lock.as_str() {
        "L3" => Lock::L3,
        "L2" => Lock::L2,
        "L1" => Lock::L1,
        _ => Lock::L0,
    };
    IfInjection {
        input: draft.input.clone(),
        kind,
        core: draft.core.clone(),
        core_prop: None,
        time_anchor,
        scope,
        lock,
        digestion: (kind == IfKind::Retcon).then_some(DigestionStrategy::Reinterpret),
        non_commitments: draft.non_commitments.clone(),
        world_line_shift: 0.0,
        rewritten_from: None,
        auto_confirmed: false,
    }
}

fn confirm_if(store: &mut Store, path: &Path, edited_input: Option<String>) -> Result<WorldSnapshot, String> {
    let line = store.active_line().map_err(|e| e.to_string())?.ok_or_else(|| "当前没有活跃世界线".to_owned())?;
    let turns = store.turns_of_line(&line).map_err(|e| e.to_string())?;
    let mut turn = turns.into_iter().rev().find(|turn| turn.ruling_card.as_ref().is_some_and(|card| card.status == IfCardStatus::Pending))
        .ok_or_else(|| "当前没有待确认的 IF 裁定卡".to_owned())?;
    let projection = store.load_projection(&line).map_err(|e| e.to_string())?;
    let card = turn.ruling_card.as_mut().expect("pending card");
    if !card.conflicts.is_empty() {
        return Err("当前 IF 存在冲突，请选择“按重释确认”".into());
    }
    if let Some(input) = edited_input.filter(|value| !value.trim().is_empty()) {
        let parsed = crate::if_parser::parse(&input)?;
        card.injection = injection_from_draft(&parsed);
        card.conflicts = find_conflicts(&projection, &card.injection);
        card.warnings = parsed.warnings;
        if !card.conflicts.is_empty() {
            card.warnings.push(format!("本地预检发现 {} 条可能冲突的既有事实；确认前应先重释，失败后再按锁定等级处理。", card.conflicts.len()));
        }
        turn.input = Some(parsed.input);
    }
    card.injection.auto_confirmed = false;
    let event = store.append(if_store::EventDraft::new(line.clone(), turn.id.clone(), projection.world_time, Patch::IfInjected(Box::new(card.injection.clone())))).map_err(|e| e.to_string())?;
    card.status = IfCardStatus::Confirmed;
    card.confirmed_event = Some(event.id.clone());
    turn.committed = vec![event.id];
    store.save_turn(&turn).map_err(|e| e.to_string())?;
    snapshot(store, path)
}

fn reinterpret_if(store: &mut Store, path: &Path) -> Result<WorldSnapshot, String> {
    let line = store.active_line().map_err(|e| e.to_string())?.ok_or_else(|| "当前没有活跃世界线".to_owned())?;
    let mut turn = store.turns_of_line(&line).map_err(|e| e.to_string())?.into_iter().rev()
        .find(|turn| turn.ruling_card.as_ref().is_some_and(|card| card.status == IfCardStatus::Pending))
        .ok_or_else(|| "当前没有待确认的 IF 裁定卡".to_owned())?;
    let projection = store.load_projection(&line).map_err(|e| e.to_string())?;
    let card = turn.ruling_card.as_mut().expect("pending card");
    if card.conflicts.is_empty() { return Err("当前 IF 没有需要重释的冲突".into()); }
    card.injection.digestion = Some(DigestionStrategy::Reinterpret);
    let note = format!("按重释处理 {} 条冲突；保留既有事实并补充新 IF 的意义。", card.conflicts.len());
    store.append(if_store::EventDraft::new(line.clone(), turn.id.clone(), projection.world_time, Patch::IfConflictResolved { note })).map_err(|e| e.to_string())?;
    let event = store.append(if_store::EventDraft::new(line.clone(), turn.id.clone(), projection.world_time, Patch::IfInjected(Box::new(card.injection.clone())))).map_err(|e| e.to_string())?;
    card.status = IfCardStatus::Confirmed;
    card.confirmed_event = Some(event.id.clone());
    turn.committed = vec![event.id];
    store.save_turn(&turn).map_err(|e| e.to_string())?;
    snapshot(store, path)
}

fn cancel_if(store: &mut Store, path: &Path) -> Result<WorldSnapshot, String> {
    let line = store.active_line().map_err(|e| e.to_string())?.ok_or_else(|| "当前没有活跃世界线".to_owned())?;
    let mut turn = store.turns_of_line(&line).map_err(|e| e.to_string())?.into_iter().rev().find(|turn| turn.ruling_card.as_ref().is_some_and(|card| card.status == IfCardStatus::Pending))
        .ok_or_else(|| "当前没有待确认的 IF 裁定卡".to_owned())?;
    turn.ruling_card.as_mut().expect("pending card").status = IfCardStatus::Cancelled;
    store.save_turn(&turn).map_err(|e| e.to_string())?;
    snapshot(store, path)
}

/// 世界文件名：`<名字 slug>-<纳秒>.ifworld`。
///
/// 名字基本是中文，所以 slug 保留 CJK（见 [`crate::slug`]）；折成纯 `world` 的话，
/// 一个目录里的所有世界就只能靠时间戳认了。截到 40 个字符是为了别把路径顶爆。
pub fn new_world_path(dir: &Path, label: &str) -> PathBuf {
    let slug: String = crate::slug::slug(label).chars().take(40).collect();
    let slug = if slug.is_empty() { "world".to_owned() } else { slug };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.join(format!("{slug}-{timestamp}.ifworld"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_world_path() -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("if-worker-test-{id}.ifworld"))
    }

    /// 不带材料地建一个空世界——只为把 worker 的请求处理跑起来。
    fn new_world(path: &Path) -> WorldWorker {
        WorldWorker::create(
            path.to_path_buf(),
            CreateRequest::new("测试世界", WorldSettings::default(), None),
        )
        .unwrap()
        .world
    }

    #[test]
    fn create_snapshot_reopen_and_close_world_on_owner_thread() {
        let path = temp_world_path();
        {
            let handle = WorldWorker::create(
                path.clone(),
                CreateRequest::new("测试世界", WorldSettings::default(), None),
            )
            .unwrap();
            assert_eq!(handle.snapshot.label, "测试世界");
            assert_eq!(handle.snapshot.active_line_id, "wl_main");
            assert_eq!(handle.snapshot.event_count, 1);
            assert!(handle.seed.is_none(), "没有材料就没有播种简报");
            assert_eq!(handle.world.snapshot().unwrap().label, "测试世界");
        }
        let reopened = WorldWorker::open(path.clone()).unwrap();
        assert_eq!(reopened.snapshot.label, "测试世界");
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }

    /// 播种走的是「先读 next_seq 再成批写入」这条路，所以这里验的是端到端结果：
    /// 一张真实形态的卡进去，世界里的主体与设定条目要真的在投影里。
    #[test]
    fn create_sows_the_world_asset_and_reports_what_it_did() {
        let path = temp_world_path();
        let world = crate::importer::parse_json(
            r#"{"spec":"chara_card_v2","data":{
                "name":"裴聿","description":"长安县令","scenario":"雨夜的长安",
                "character_book":{"entries":{
                    "1":{"comment":"宵禁","content":"入夜后坊门落锁。","key":["长安"],"constant":true}
                }}}}"#,
        )
        .unwrap();

        let handle = WorldWorker::create(
            path.clone(),
            CreateRequest::new("裴聿", WorldSettings::default(), Some(world)),
        )
        .unwrap();

        // world_created + 主体 + 情境条目 + 宵禁条目
        assert_eq!(handle.snapshot.event_count, 4);
        let projection = &handle.snapshot.projection;
        assert_eq!(projection.subjects.len(), 1);
        assert_eq!(projection.subjects.values().next().unwrap().name, "裴聿");
        assert!(projection.subjects.values().next().unwrap().aliases.is_empty());
        assert_eq!(projection.lore.len(), 2);
        assert!(projection
            .lore
            .values()
            .any(|entry| entry.title == "裴聿 · 情境" && entry.constant));

        let report = handle.seed.as_ref().expect("有材料就应当有播种简报");
        assert_eq!(report.subjects, 1);
        assert_eq!(report.lore, 2);
        assert_eq!(report.world_name, "裴聿");

        drop(handle);
        std::fs::remove_file(path).unwrap();
    }

    /// 中文世界名不能全被折成 `world`——那样一个目录里就只剩时间戳能区分了。
    #[test]
    fn world_file_name_keeps_the_chinese_label() {
        let dir = std::env::temp_dir();
        let path = new_world_path(&dir, "雨城 · 第一卷");
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("雨城-第一卷-"), "{name}");
        assert!(name.ends_with(".ifworld"), "{name}");

        // 全是标点的名字退到 `world`，但仍然唯一
        let fallback = new_world_path(&dir, "···");
        assert!(fallback
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("world-"));
    }

    #[test]
    fn refuses_to_open_non_world_file() {
        let path = std::env::temp_dir().join("not-an-if-world.txt");
        assert!(WorldWorker::open(path).unwrap_err().contains("不存在"));
    }

    #[test]
    fn rejects_existing_file_with_wrong_extension() {
        let path = std::env::temp_dir().join(format!(
            "if-worker-test-{}.txt",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"not a world").unwrap();
        assert!(WorldWorker::open(path.clone())
            .unwrap_err()
            .contains(".ifworld"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_empty_sqlite_database_as_world() {
        let path = temp_world_path();
        drop(Store::open(&path).unwrap());
        let error = WorldWorker::open(path.clone()).unwrap_err();
        assert!(error.contains("有效的 IF 世界"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn submit_if_creates_pending_card_without_injecting() {
        let path = temp_world_path();
        let worker = new_world(&path);
        let before = worker.snapshot().unwrap();
        let after = worker.submit_if("IF 城市的钟声突然停止".into()).unwrap();
        assert_eq!(before.event_count, after.event_count);
        assert_eq!(after.head_seq, before.head_seq);
        assert!(after.projection.injections.is_empty());
        assert_eq!(after.pending_if.as_ref().map(|card| card.status), Some(IfCardStatus::Pending));

        let confirmed = worker.confirm_if(None).unwrap();
        assert_eq!(confirmed.event_count, before.event_count + 1);
        assert_eq!(confirmed.head_seq, before.head_seq + 1);
        assert_eq!(confirmed.projection.injections.len(), 1);
        assert_eq!(confirmed.pending_if, None);
        drop(worker);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn preflight_marks_opposite_negation_as_conflict() {
        let path = temp_world_path();
        let worker = new_world(&path);
        worker.submit_if("IF 城市的钟声停止".into()).unwrap();
        worker.confirm_if(None).unwrap();
        let after = worker.submit_if("IF 城市的钟声没有停止".into()).unwrap();
        let card = after.pending_if.expect("pending card");
        assert_eq!(card.conflicts.len(), 1);
        assert!(card.warnings.iter().any(|warning| warning.contains("可能冲突")));
        drop(worker);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reinterpret_conflict_commits_resolution_and_injection() {
        let path = temp_world_path();
        let worker = new_world(&path);
        worker.submit_if("IF 城市的钟声停止".into()).unwrap();
        worker.confirm_if(None).unwrap();
        worker.submit_if("IF 城市的钟声没有停止".into()).unwrap();
        let resolved = worker.reinterpret_if().unwrap();
        assert_eq!(resolved.event_count, 4);
        assert_eq!(resolved.projection.injections.len(), 2);
        assert_eq!(resolved.pending_if, None);
        drop(worker);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn directive_language_is_advisory_and_can_enter_pending_card() {
        let path = temp_world_path();
        let worker = new_world(&path);
        let snapshot = worker.submit_if("让林夏表白".into()).unwrap();
        assert_eq!(snapshot.pending_if.as_ref().map(|card| card.status), Some(IfCardStatus::Pending));
        assert!(snapshot.pending_if.as_ref().is_some_and(|card| card.warnings.iter().any(|warning| warning.contains("导演意图"))));
        let confirmed = worker.confirm_if(None).unwrap();
        assert_eq!(confirmed.projection.injections.len(), 1);
        drop(worker);
        std::fs::remove_file(path).unwrap();
    }
}
