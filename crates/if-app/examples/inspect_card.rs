//! 开发用：对真实文件跑一遍导入器，打印结构摘要。
//!
//! ```bash
//! cargo run -p if-app --example inspect_card -- "sk-example/风之絮言.png"
//! ```
//!
//! 只做确定性解析，不联网、不落盘、不执行卡内脚本。

use std::collections::BTreeMap;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("用法: cargo run -p if-app --example inspect_card -- <文件...>");
        std::process::exit(2);
    };
    let mut paths = vec![path];
    paths.extend(args);

    let mut failed = 0usize;
    for p in &paths {
        if let Err(e) = inspect(Path::new(p)) {
            eprintln!("✗ {p}: {e}");
            failed += 1;
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}

fn inspect(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let name = path.file_name().map(|s| s.to_string_lossy().to_string());
    let world = if_app_lib::importer::parse_bytes(&bytes, name.as_deref())?;

    println!("{}", "=".repeat(78));
    println!("文件      {}", path.display());
    println!("格式      {} / {}  spec={:?}", world.source_kind, world.source_format, world.spec_version);
    println!("世界名    {}  genre={:?}", world.name, world.genre);

    for c in &world.characters {
        println!("角色      {}  nickname={:?}  creator={:?}  ver={:?}",
            c.name, c.nickname, c.creator, c.character_version);
        println!("          desc={} personality={} scenario={} first_mes={} alt={}",
            c.description.chars().count(),
            c.personality.chars().count(),
            c.scenario.chars().count(),
            c.first_message.chars().count(),
            c.alternate_greetings.len());
        println!("          system_prompt={} post_history={} tags={:?}",
            c.system_prompt.chars().count(),
            c.post_history_instructions.chars().count(),
            c.tags);
        println!("          assets={} source={:?}", c.assets.len(), c.source);
    }

    println!("设定条目  {} 条", world.lore.len());
    {
        let meta = &world.lore_meta;
        println!("  世界书  name={:?} scan_depth={:?} token_budget={:?} recursive={:?}",
            meta.name, meta.scan_depth, meta.token_budget, meta.recursive_scanning);
    }

    // 方言分布
    let mut dialects: BTreeMap<&str, usize> = BTreeMap::new();
    for l in &world.lore {
        *dialects.entry(l.source_dialect.as_str()).or_default() += 1;
    }
    println!("  方言    {dialects:?}");

    // 归段分布
    let mut sections: BTreeMap<String, usize> = BTreeMap::new();
    for l in &world.lore {
        *sections.entry(format!("{:?}", l.section)).or_default() += 1;
    }
    println!("  归段    {sections:?}");

    // 开关 / 结构化程度
    let disabled = world.lore.iter().filter(|l| !l.enabled).count();
    let constant = world.lore.iter().filter(|l| l.constant).count();
    let keyed = world.lore.iter().filter(|l| !l.constant && !l.keys.is_empty()).count();
    let orphan = world.lore.iter().filter(|l| !l.constant && l.keys.is_empty()).count();
    let selective = world.lore.iter().filter(|l| l.selective).count();
    let regex = world.lore.iter().filter(|l| l.use_regex == Some(true)).count();
    let decorators = world.lore.iter().filter(|l| !l.decorators.is_empty()).count();
    println!("  常驻={constant} 关键词={keyed} 无关键词={orphan} 停用={disabled}");
    println!("  selective={selective} use_regex={regex} 带装饰器={decorators}");

    let total: usize = world.lore.iter().map(|l| l.content.chars().count()).sum();
    let max = world.lore.iter().map(|l| l.content.chars().count()).max().unwrap_or(0);
    println!("  正文   总 {total} 字符  最长 {max}");

    // 最长的几条，看承重性
    let mut sorted: Vec<_> = world.lore.iter().collect();
    sorted.sort_by_key(|l| std::cmp::Reverse(l.content.chars().count()));
    for l in sorted.iter().take(5) {
        let head: String = l.content.chars().take(48).collect();
        println!("    {:>5} 字 | {:?} | {}", l.content.chars().count(), l.title, head.replace('\n', " "));
    }

    println!("宏        {:?}", world.macros);
    println!("warnings  {}", world.warnings.len());
    for w in &world.warnings {
        println!("    ! {w}");
    }
    Ok(())
}
