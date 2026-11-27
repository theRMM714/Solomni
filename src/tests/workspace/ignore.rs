//! 目的：`work_ignore` 工具面与「规则只管发现」的语义——名单读改写、越界拒绝、显式点名仍生效。
//! 管：list / add / remove 三种 op、去重、删规则、坏规则整次拒绝且不落盘、沙箱清单按规则剪枝。
//! 不管：规则匹配语义（`filter.rs`）；目录遍历剪枝（`tests::detail::fs`）。
//! 联动：`src/units/business/versions/service/ignore.rs`；`.gitignore` 不在这里改。

use super::support::*;
use crate::units::business::versions::ports::WorkStore;
use crate::units::business::workbench::api::{Rules, WORKIGNORE};

/// 目的：没有名单文件时按种子答话；add 去重、立刻生效、remove 精确删行。
#[test]
pub(crate) fn work_ignore_lists_seed_then_add_dedupes_and_remove_drops() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let path = ws.roots("w", &[]).unwrap().store.join(WORKIGNORE);

    let listed = ws.work_ignore("w", "list", &[]).unwrap();
    assert!(
        listed.rules.iter().any(|l| l.trim() == "node_modules"),
        "缺文件时回种子名单，而不是空名单"
    );
    assert!(store.read_text(&path).unwrap().is_none(), "list 不写文件");

    let added = ws
        .work_ignore("w", "add", &["*.log".to_string(), "*.log".to_string()])
        .unwrap();
    assert_eq!(
        added.rules.iter().filter(|l| l.trim() == "*.log").count(),
        1,
        "同一次 add 里的重复规则只加一条"
    );
    let text = store.read_text(&path).unwrap().expect("规则已落盘");
    assert!(text.contains("node_modules"), "种子规则不被覆盖掉");
    assert!(text.ends_with("*.log\n"), "写回以换行收尾");
    let rules = Rules::compile(std::path::Path::new("."), &text).unwrap();
    assert!(rules.is_ignored("debug.log", false), "新规则立刻生效");

    let removed = ws
        .work_ignore("w", "remove", &["*.log".to_string()])
        .unwrap();
    assert!(!removed.rules.iter().any(|l| l.trim() == "*.log"));
    assert!(!store.read_text(&path).unwrap().unwrap().contains("*.log"));
    assert!(
        ws.work_ignore("w", "remove", &["nope.tmp".to_string()])
            .is_ok(),
        "删一条不存在的规则是幂等操作"
    );
}

/// 目的：越界规则与坏 op 整次拒绝，且一个字节都不落到名单文件里。
#[test]
pub(crate) fn work_ignore_rejects_bad_rules_and_bad_ops_without_writing() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let path = ws.roots("w", &[]).unwrap().store.join(WORKIGNORE);

    let err = ws
        .work_ignore("w", "add", &["C:/tmp/x".to_string()])
        .unwrap_err();
    assert!(err.contains("绝对路径"), "{err}");
    let err = ws
        .work_ignore("w", "add", &["../up".to_string()])
        .unwrap_err();
    assert!(err.contains(".."), "{err}");
    let err = ws.work_ignore("w", "add", &["*".to_string()]).unwrap_err();
    assert!(err.contains("忽略全部文件"), "{err}");
    let err = ws.work_ignore("w", "add", &[]).unwrap_err();
    assert!(err.contains("至少一条"), "{err}");
    let err = ws
        .work_ignore("w", "list", &["*.log".to_string()])
        .unwrap_err();
    assert!(err.contains("list 不收 patterns"), "{err}");
    let err = ws.work_ignore("w", "drop", &[]).unwrap_err();
    assert!(err.contains("op 只认"), "{err}");
    assert!(
        store.read_text(&path).unwrap().is_none(),
        "被拒的操作不留半个文件"
    );
}

/// 目的：沙箱清单按名单剪枝；但显式点名的单个文件仍然算（同 git add -f）。
#[test]
pub(crate) fn sandbox_listing_follows_workignore_but_named_files_still_count() {
    let (ws, _dirs, store) = work_ws();
    ws.ensure_layout("w", &["a".to_string()]).unwrap();
    let sandbox = sandbox_of(&ws, "w", "a");
    store.seed(&sandbox, "src/main.rs", "fn main() {}\n");
    store.seed(
        &sandbox,
        "node_modules/pkg/index.js",
        "module.exports = 1;\n",
    );

    let status = ws.work_status("w", "a", &[]).unwrap();
    let paths: Vec<&str> = status.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"src/main.rs"), "{paths:?}");
    assert!(
        !paths.contains(&"node_modules/pkg/index.js"),
        "种子名单盖住 node_modules：未跟踪清单里不该出现它：{paths:?}"
    );

    // 点名一个被盖住的文件夹：不展开（文件夹属于「发现」）。
    let err = ws
        .work_commit(
            "w",
            "a",
            "s",
            &commit_req(&["node_modules"], &[], "vendor dir", 2),
        )
        .unwrap_err();
    assert!(err.contains("node_modules"), "{err}");

    // 点名一个被盖住的文件：照样能提交。
    let report = ws
        .work_commit(
            "w",
            "a",
            "s",
            &commit_req(&["node_modules/pkg/index.js"], &[], "vendor file", 3),
        )
        .unwrap();
    assert!(
        report
            .changes
            .iter()
            .any(|c| c.path == "node_modules/pkg/index.js"),
        "{:?}",
        report.changes
    );
}
