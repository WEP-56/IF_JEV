//! 开发用：把一个真实 `.ifworld` 的 T-impact 提示词 dump 出来。
//!
//! ```bash
//! cargo run -p if-app --example dump_turn_prompt -- "<path.ifworld>" "IF 故事自然向前推进一点"
//! ```
//!
//! 只做确定性编译，不联网、不落盘。用来回答「模型到底收到了什么」——
//! 当任务报「模型没有产出提议」时，先看这里的视图是不是空的/自相矛盾的。

use if_domain::id::{SubjectId, TurnId, WorldLineId};
use if_domain::turn::{TurnKind, ViewKind};
use if_pipeline::context::{activate_for_turn, TurnContext};
use if_store::Store;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("用法: cargo run -p if-app --example dump_turn_prompt -- <文件.ifworld> [IF 文本]");
        std::process::exit(2);
    };
    let input = args.next();

    let store = Store::open(std::path::Path::new(&path)).expect("打开世界文件失败");
    let line: WorldLineId = store.active_line().expect("读活动世界线失败").expect("没有活动世界线");
    let projection = store.load_projection(&line).expect("折叠投影失败");
    let settings = projection.settings.clone().expect("缺少 world_created 事件");

    println!("世界线      {line}");
    println!("事件数      {}", store.event_count().unwrap_or(0));
    println!("主体        {}", projection.subjects.len());
    println!("命题        {}", projection.propositions.len());
    println!("事实        {}", projection.facts.len());
    println!("规则        {}", projection.rules.len());
    println!("故事线      {}", projection.threads.len());
    println!("设定条目    {}", projection.lore.len());
    println!("场景        {}", projection.scenes.len());
    println!();

    let subjects = focus_and_present(&projection);
    let mut ctx = TurnContext::new(
        TurnId::numbered(store.event_count().unwrap_or(0) + 1),
        line.clone(),
        TurnKind::If,
        projection.world_time,
        projection.scenes.len() as u64,
    )
    .focus(subjects.clone())
    .present(subjects.clone());
    ctx.user_input = input.clone();
    let scan = input.clone().unwrap_or_default();
    ctx.lore = activate_for_turn(&projection, &settings, &ctx, &scan);

    println!("焦点/在场   {subjects:?}");
    println!("激活设定    {}", ctx.lore.len());
    println!();

    // T-impact 用的就是上帝视图（`tasks::impact::prompt`）。
    let request = ctx.view(ViewKind::God).task("T-impact");
    let compiled = if_views::compile(&projection, &request);
    let json = serde_json::to_string_pretty(&compiled.state).unwrap_or_default();
    println!("===== 上帝视图（{len} 字符）=====", len = json.len());
    println!("{json}");
}

fn focus_and_present(projection: &if_domain::projection::Projection) -> Vec<SubjectId> {
    use if_domain::subject::Tier;
    let mut focused: Vec<SubjectId> = projection
        .subjects
        .values()
        .filter(|subject| matches!(subject.tier, Tier::Foreground | Tier::Active))
        .map(|subject| subject.id.clone())
        .collect();
    if focused.is_empty() {
        focused = projection.subjects.keys().cloned().collect();
    }
    focused
}
