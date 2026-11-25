//! 目的：工作区过滤规则（`.workignore`）的语义——内容面与检索面共用的唯一名单。
//! 管：目录名命中整棵子树、取反、目录专属、锚定、空规则与非法行、以及播种的默认名单。
//! 不管：目录遍历与剪枝（见 `tests::detail::fs` 的 `list_visible` 用例）；`.gitignore`（只在 grep 的检索面生效）。
//! 联动：`src/units/workspace/domain/path_filter.rs`；工具面由 `work_ignore` 维护同一份文件。

use crate::units::workspace::api::{MAX_RULE_CHARS, Rules, WORKIGNORE_SEED, check_rule};
use std::path::Path;

fn rules(text: &str) -> Rules {
    Rules::compile(Path::new("."), text).expect("编译规则")
}

/// 目的：命中的目录名剪掉整棵子树（含任意层级的同名目录）。
#[test]
fn rules_ignore_a_whole_named_dir_at_any_depth() {
    let r = rules("node_modules\n");
    assert!(r.is_ignored("node_modules", true), "目录本身命中");
    assert!(
        r.is_ignored("node_modules/pkg/index.js", false),
        "目录下的文件随祖先后命中"
    );
    assert!(
        r.is_ignored("src/node_modules/x.js", false),
        "任意层级的同名目录同样命中"
    );
    assert!(!r.is_ignored("src/main.rs", false), "无关路径不命中");
}

/// 目的：取反、目录专属与锚定按 git 语义解释——不自创规则。
#[test]
fn rules_follow_gitignore_negation_dir_only_and_anchor() {
    let r = rules("*.log\n!keep.log\n");
    assert!(r.is_ignored("a.log", false), "通配命中");
    assert!(!r.is_ignored("keep.log", false), "取反放行");

    let r = rules("build/\n");
    assert!(r.is_ignored("build", true), "带 / 的规则匹配目录");
    assert!(!r.is_ignored("build", false), "同名文件不是目录，不命中");
    assert!(
        r.is_ignored("build/out.o", false),
        "目录下的文件随祖先后命中"
    );

    let r = rules("/top.txt\n");
    assert!(r.is_ignored("top.txt", false), "锚定到根");
    assert!(!r.is_ignored("sub/top.txt", false), "锚定后不匹配子目录");

    let r = rules("top.txt\n");
    assert!(r.is_ignored("sub/top.txt", false), "无锚定匹配任意层级");
}

/// 目的：空规则不过滤任何路径；非法行如实报错并点出行号与内容，不静默跳过。
#[test]
fn rules_treat_empty_text_as_no_filter_and_report_bad_lines() {
    assert!(!rules("").is_ignored("anything", false));
    assert!(!rules("# 只有注释\n\n").is_ignored("anything", false));

    let err = match Rules::compile(Path::new("."), "ok.txt\n[z-a]\n") {
        Ok(_) => panic!("非法字符类必须报错，不能静默跳过"),
        Err(e) => e,
    };
    assert!(
        err.contains("第 2 行") && err.contains("[z-a]"),
        "错误要点名行号与行内容：{err}"
    );
}

/// 目的：写规则前的越界检查——只认工作区内的相对路径，且不接受会忽略全部文件的模式。
/// 约束：普通 gitignore 写法（目录名、锚定、取反、结尾 /）都要放行，检查只管越界。
#[test]
fn check_rule_rejects_escaping_paths_and_catch_all_patterns() {
    for good in [
        "node_modules",
        "*.log",
        "/build",
        "!keep.txt",
        "a/b/c",
        "**/tmp",
    ] {
        assert!(check_rule(good).is_ok(), "正常规则要放行：{good}");
    }
    let bad = [
        ("", "空行"),
        ("C:/tmp/x", "绝对路径"),
        ("\\\\server\\share", "绝对路径"),
        ("../up", ".."),
        ("a/../../b", ".."),
        ("*", "忽略全部文件"),
        ("**", "忽略全部文件"),
        ("/*", "忽略全部文件"),
    ];
    for (rule, want) in bad {
        let err = check_rule(rule).unwrap_err();
        assert!(
            err.contains(want),
            "规则 {rule:?} 的错误要点名「{want}」：{err}"
        );
    }
    let long = "a".repeat(MAX_RULE_CHARS + 1);
    let err = check_rule(&long).unwrap_err();
    assert!(err.contains("太长"), "{err}");
    assert!(check_rule("a\0b").is_err(), "NUL 字符要拒绝");
}

/// 目的：播种的默认名单覆盖 VCS 元数据与依赖缓存，但不把隐藏文件一律忽略。
#[test]
fn seed_rules_cover_noise_dirs_but_keep_hidden_files() {
    let r = rules(WORKIGNORE_SEED);
    for dir in [
        ".git",
        ".svn",
        ".hg",
        "node_modules",
        "target",
        "__pycache__",
        ".venv",
        "venv",
    ] {
        assert!(r.is_ignored(dir, true), "种子要忽略 {dir}");
        assert!(
            r.is_ignored(&format!("{dir}/x"), false),
            "种子要连带忽略 {dir}/ 下的内容"
        );
    }
    assert!(!r.is_ignored("src/main.rs", false), "普通源码不在名单里");
    assert!(
        !r.is_ignored(".github/workflows/ci.yml", false),
        "隐藏文件不自动忽略：.workignore 只管名单"
    );
}
