//! 目的：命令面门禁——`systools/commands.yaml` 是唯一声明，且与两处分派实现（启动相、转录相）逐条对齐。
//! 管：表自身自洽、手册覆盖全部启动命令、入口与 CLI 都认得表里的每一条命令（双向，含反向的孤儿旗语）。
//! 不管：手册排版（`src/tests/cli.rs`）与命令行为（各业务测试）。
//! 联动：`systools/commands.yaml`、`src/presentation/cli/boot.rs`、`src/presentation/cli.rs`、`src/entry/args.rs`。

use crate::capabilities::tools::api::{CommandBook, Phase};
use crate::presentation::cli::catalog;

/// 启动相的分派实现（启动命令与它们的旗语落在这里）。
const BOOT: &str = include_str!("../presentation/cli/boot.rs");
/// 入口的内部协议分派（这里不许再出现产品命令）。
const ENTRY_ARGS: &str = include_str!("../entry/args.rs");
/// 转录中心的分派实现（转录相命令的落点）。
const CLI: &str = include_str!("../presentation/cli.rs");

/// 目的：内部协议旗语（不进命令表）：只给测试、探针与守门子进程用。
const INTERNAL: &[&str] = &[
    "--proc-watchdog",
    "--fence-run",
    "--https-check",
    "--unpack",
    "--print-fence-env",
    "--fence-verify",
    "--print-routes",
    "--fence-clean",
    "--probe-tools",
    "--probe-replay",
];

fn book() -> &'static CommandBook {
    CommandBook::builtin().expect("命令表可解析")
}

/// 目的：从一段 Rust 源码里抠出所有 `"--xxx"` 旗语字面量（去重，保持出现顺序）。
fn flags_in(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 2 < chars.len() {
        if chars[i] == '"' && chars[i + 1] == '-' && chars[i + 2] == '-' {
            let mut j = i + 1;
            let mut name = String::new();
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-') {
                name.push(chars[j]);
                j += 1;
            }
            if name.len() > 2 && !out.contains(&name) {
                out.push(name);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// 表自身：能解析、无矛盾、两相都不为空、分组按首次出现排序。
#[test]
fn table_is_self_consistent() {
    let book = book();
    assert!(book.problems().is_empty(), "{:?}", book.problems());
    assert_eq!(
        book.phase(Phase::Boot).len(),
        6,
        "启动相六条：doctor/provision/work-gc/storage/gc/build"
    );
    assert_eq!(book.phase(Phase::Core).len(), 17, "转录相十七条");
    assert_eq!(book.groups(Phase::Boot), vec!["维护"]);
    let core = book.groups(Phase::Core);
    assert_eq!(core.first(), Some(&"会话"));
    assert_eq!(core.last(), Some(&"系统"));
}

/// 手册：启动命令一条不落（含它的写法），单条说明给出旗语与确认门槛。
#[test]
fn help_lists_every_boot_command() {
    let help = catalog::help_text(false);
    for cmd in book().phase(Phase::Boot) {
        assert!(
            help.contains(&cmd.invocation()),
            "手册漏了：{}",
            cmd.invocation()
        );
        assert!(help.contains(&cmd.brief), "手册漏了说明：{}", cmd.brief);
    }
    let gc = catalog::detail("--gc").expect("启动相命令有自己的说明");
    assert!(
        gc.contains(&book().find("gc").expect("gc").invocation()),
        "{gc}"
    );
    assert!(gc.contains("--yes"), "旗语要列出来：{gc}");
    assert!(gc.contains("--category <类别>"), "{gc}");
    assert!(gc.contains("确认："), "有写操作就要写确认门槛：{gc}");
    assert!(
        catalog::detail("--doctor")
            .expect("doctor")
            .contains("启动命令")
    );
}

/// 双向对齐：表里的启动命令启动相认得，启动相用的每个旗语表里也声明了。
#[test]
fn boot_commands_match_boot_dispatch() {
    let book = book();
    for cmd in book.phase(Phase::Boot) {
        let literal = format!("\"{}\"", cmd.name);
        assert!(
            BOOT.contains(&literal),
            "启动命令 {} 在 presentation/cli/boot.rs 里没有执行入口",
            cmd.name
        );
    }
    let declared: Vec<String> = book
        .phase(Phase::Boot)
        .iter()
        .flat_map(|c| c.flags.iter().map(|f| f.name.clone()))
        .collect();
    for flag in flags_in(BOOT) {
        assert!(
            declared.contains(&flag) || INTERNAL.contains(&flag.as_str()),
            "启动相用了没声明的旗语 {}（命令自己的旗语要在命令表里写）",
            flag
        );
    }
}

/// 入口只剩内部协议：产品命令与产品旗语都不许再出现在 src/entry/args.rs。
#[test]
fn entry_keeps_only_internal_protocol() {
    for flag in flags_in(ENTRY_ARGS) {
        assert!(
            INTERNAL.contains(&flag.as_str()) || flag.starts_with("--fence-"),
            "src/entry/args.rs 里还留着产品旗语 {}（产品命令归启动相）",
            flag
        );
    }
}

/// 双向对齐：表里的转录相命令 CLI 认得，且补全与建议只从表里出。
#[test]
fn core_commands_match_cli_dispatch() {
    let book = book();
    for cmd in book.phase(Phase::Core) {
        let literal = format!("\"{}\"", cmd.name);
        assert!(
            CLI.contains(&literal),
            "转录相命令 {} 在 presentation/cli.rs 里没有分派",
            cmd.name
        );
        assert!(
            catalog::complete(&cmd.name).contains(&cmd.name),
            "补全里没有 {}",
            cmd.name
        );
    }
    assert!(catalog::suggest("singel").as_deref() == Some("single"));
    assert!(catalog::suggest("porxy").as_deref() == Some("proxy"));
}
