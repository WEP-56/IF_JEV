//! 把一份**作者世界包**落成一个可打开的 `.ifworld`，并打印它的投影摘要。
//!
//! ```text
//! cargo run -p if-app --example build_world -- crates/if-app/examples/worlds/linjiang.world.json
//! cargo run -p if-app --example build_world -- <世界包.json> <输出.ifworld>
//! cargo run -p if-app --example build_world -- --check <世界包.json>    # 只校验，不写文件
//! ```
//!
//! 不联网、不花额度。这个入口有两个用处：
//!
//! 1. **看一份世界包里到底有什么**——投影摘要按类别报数，不用打开 SQLite。
//! 2. **造一个能直接跑的世界**——`authored::write` 走的是和会话播种同一条代码路径
//!    （事件 ID 先算号再写），所以这里写出来的世界和应用里建出来的没有区别。
//!
//! 它是 [`if_app_lib::authored`] 的**唯一非测试调用方**。会话创建那条路暂时还只认
//! 导入结果（`ImportedWorld`），把作者世界接进 `create_session` 是下一步（docs/16 §5）。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use if_app_lib::authored::{self, AuthoredWorld};
use if_domain::id::WorldLineId;
use if_store::Store;

fn main() -> ExitCode {
    let mut check_only = false;
    let mut positional: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => check_only = true,
            "-h" | "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            _ => positional.push(arg),
        }
    }

    let Some(input) = positional.first() else {
        usage();
        return ExitCode::FAILURE;
    };
    let input = PathBuf::from(input);

    let world = match authored::load(&input) {
        Ok(world) => world,
        Err(error) => {
            eprintln!("✗ {error}");
            return ExitCode::FAILURE;
        }
    };
    println!("✓ 世界包读通了：{}", world.name);

    if check_only {
        describe(&world);
        println!("\n（--check：只校验，没有写文件）");
        return ExitCode::SUCCESS;
    }

    let output = positional
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_output(&input));
    // 显式指定的输出路径由调用方负责——它是一个构建产物，重复跑要能覆盖。
    // 库里的 `write` 仍然坚持「不覆盖」（见它的文档），这条宽松只属于本入口。
    if output.exists() {
        println!("· 移除已存在的输出：{}", output.display());
        if let Err(error) = std::fs::remove_file(&output) {
            eprintln!("✗ 删不掉旧文件：{error}");
            return ExitCode::FAILURE;
        }
    }

    match authored::write(&output, &world) {
        Ok(report) => {
            println!("✓ 已写出世界：{}", output.display());
            print_report(&report);
        }
        Err(error) => {
            eprintln!("✗ {error}");
            return ExitCode::FAILURE;
        }
    }

    // 再打开一次：写出来的东西要真的能读，而不是「写完就算了」。
    match summarize(&output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("✗ 写出来的世界打不开：{error}");
            ExitCode::FAILURE
        }
    }
}

/// 打开写出来的文件，按类别报一遍投影里的东西。
fn summarize(path: &Path) -> Result<(), String> {
    let store = Store::open(path).map_err(|e| e.to_string())?;
    let line: WorldLineId = store
        .active_line()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "没有活跃世界线".to_owned())?;
    let projection = store.load_projection(&line).map_err(|e| e.to_string())?;

    println!("\n—— 写出来的世界 ——");
    println!("  世界线      {}", line);
    println!("  事件        {}", store.event_count().map_err(|e| e.to_string())?);
    println!("  主体        {}", projection.subjects.len());
    println!("  命题        {}", projection.propositions.len());
    println!("  事实        {}", projection.facts.len());
    println!("  规则        {}", projection.rules.len());
    println!("  故事线      {}", projection.threads.len());
    println!("  设定条目    {}", projection.lore.len());
    let settings = projection.settings.as_ref();
    println!(
        "  骰子种子    {}",
        settings.map(|s| s.seed.to_string()).unwrap_or_else(|| "（无）".to_owned())
    );

    println!("\n  可改写的命题（IF 的着力点）：");
    for proposition in projection.propositions.values() {
        println!("    {:<28} {}", proposition.key, proposition.text);
    }
    Ok(())
}

/// `--check` 时把包里的东西念一遍，让人不用打开文件也知道里面有什么。
fn describe(world: &AuthoredWorld) {
    println!("\n—— 世界包 ——");
    println!("  题材        {}", world.genre);
    println!("  主体        {}", world.subjects.len());
    println!("  命题        {}", world.propositions.len());
    println!("  事实        {}", world.facts.len());
    println!("  规则        {}", world.rules.len());
    println!("  故事线      {}", world.threads.len());
    println!("  设定条目    {}", world.lore.len());
}

fn print_report(report: &authored::AuthoredReport) {
    println!(
        "  写入：主体 {} · 命题 {} · 事实 {} · 规则 {} · 设定 {} · 故事线 {}",
        report.subjects,
        report.propositions,
        report.facts,
        report.rules,
        report.lore,
        report.threads
    );
    for note in &report.notes {
        println!("  ⚠ {note}");
    }
}

/// 没给输出路径时，写在输入旁边：`linjiang.world.json` → `linjiang.ifworld`。
fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".json"))
        .map(|name| name.trim_end_matches(".world"))
        .filter(|name| !name.is_empty())
        .unwrap_or("world");
    input.with_file_name(format!("{stem}.ifworld"))
}

fn usage() {
    eprintln!(
        "用法：build_world [--check] <世界包.json> [输出.ifworld]\n\
         \n\
         把一份作者世界包落成可打开的 .ifworld。不给输出路径时写在输入旁边。"
    );
}
