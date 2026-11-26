//! 目的：内置 `grep` 的行为契约——目录递归、忽略规则与隐藏 / 二进制剪枝、glob、上下文、files_only、字面匹配与非法输入。
//! 管：`grep` 的输出形态与剪枝结论；规则来源是版本库目录里的 `.workignore`。
//! 不管：工具面发放（`src/tests/tools/face.rs`）与派发（`src/tests/tools/dispatch.rs`），也不管过滤规则本身的匹配语义（`src/tests/workspace/filter.rs`）。
//! 联动：被测实现在 `src/units/business/tools/service/grep.rs`，夹具是 `src/tests/doubles/io.rs` 的 `InMemorySysIo`。

use super::super::prelude::*;

/// 目的：一棵带噪音的测试目录：正常文件、被 .workignore 排除的目录、隐藏目录、二进制文件。
fn seed_tree(io: &InMemorySysIo) {
    io.seed(
        &["demo", "a1", "src", "app.txt"],
        "alpha one\nbeta\nALPHA two\n",
    );
    io.seed(&["demo", "a1", "vendor", "skip.txt"], "alpha vendor\n");
    io.seed(&["demo", "a1", ".secret", "hidden.txt"], "alpha hidden\n");
    io.seed(&["demo", "a1", "logo.bin"], "alpha\0binary\n");
    io.seed(&["demo", ".work", ".workignore"], "vendor/\n");
}

#[test]
pub(crate) fn grep_walks_a_tree_and_prunes_ignore_hidden_and_binary() {
    let io = InMemorySysIo::new();
    seed_tree(&io);
    let sb = test_sandbox("a1", &[]);
    let dir = s(&["demo", "a1"]);
    // 默认：.workignore + 隐藏 + 二进制；目录递归，按行给「路径:行号:正文」。
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"alpha\",\"ignore_case\":true}}",
            dir
        ),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains(":1:alpha one"), "{}", out.output);
    assert!(out.output.contains(":3:ALPHA two"), "{}", out.output);
    assert!(
        !out.output.contains("vendor") && !out.output.contains("hidden"),
        "被规则排除与隐藏的文件不该出现：{}",
        out.output
    );
    assert!(!out.output.contains("logo.bin"), "{}", out.output);
    assert!(
        out.output.contains("命中 2 行 / 1 个文件"),
        "{}",
        out.output
    );
    assert!(
        out.output.contains("跳过 1 个二进制"),
        "二进制要计数：{}",
        out.output
    );

    // 两个逃生口：hidden 放行点文件，no_ignore 连工作区过滤也关掉。
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"alpha\",\"ignore_case\":true,\"hidden\":true,\"no_ignore\":true}}",
            dir
        ),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("alpha vendor"), "{}", out.output);
    assert!(out.output.contains("alpha hidden"), "{}", out.output);
    assert!(
        out.output.contains("命中 4 行 / 3 个文件"),
        "{}",
        out.output
    );
    assert!(
        out.output.contains("[过滤]"),
        "要如实说这次没套过滤：{}",
        out.output
    );
}

#[test]
pub(crate) fn grep_honors_glob_context_files_only_and_literal_mode() {
    let io = InMemorySysIo::new();
    io.seed(&["demo", "a1", "a.md"], "x one\nsecond\nx two\n");
    io.seed(&["demo", "a1", "b.txt"], "x three\n");
    io.seed(&["demo", "a1", "re.txt"], "a.b\naXb\n");
    let sb = test_sandbox("a1", &[]);
    let dir = s(&["demo", "a1"]);

    // glob 只留 *.md；context=1 带出前后文（上下文行用 路径-行号-正文）。
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"x\",\"glob\":[\"*.md\"],\"context\":1}}",
            dir
        ),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains(":1:x one"), "{}", out.output);
    assert!(out.output.contains("-2-second"), "{}", out.output);
    assert!(out.output.contains(":3:x two"), "{}", out.output);
    assert!(
        !out.output.contains("x three"),
        "glob 之外的该文件不应被扫：{}",
        out.output
    );
    assert!(
        out.output.contains("命中 2 行 / 1 个文件；扫描 1 个文件"),
        "{}",
        out.output
    );

    // files_only：只回路径与条数。
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"x\",\"files_only\":true}}",
            dir
        ),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("（2 行）"), "{}", out.output);
    assert!(out.output.contains("（1 行）"), "{}", out.output);
    assert!(
        !out.output.contains(":1:x one"),
        "files_only 不该回正文：{}",
        out.output
    );

    // 默认按正则：「a.b」连 aXb 一起命中；fixed 只认字面。
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"a.b\"}}",
            s(&["demo", "a1", "re.txt"])
        ),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("命中 2 行"), "{}", out.output);
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"a.b\",\"fixed\":true}}",
            s(&["demo", "a1", "re.txt"])
        ),
    );
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains(":1:a.b"), "{}", out.output);
    assert!(
        out.output.contains("命中 1 行"),
        "fixed 只按字面：{}",
        out.output
    );
}

#[test]
pub(crate) fn grep_reports_bad_patterns_and_the_empty_result_hint() {
    let io = InMemorySysIo::new();
    io.seed(&["demo", "a1", "a.txt"], "alpha\n");
    let sb = test_sandbox("a1", &[]);
    let dir = s(&["demo", "a1"]);

    let bad = run_builtin(
        &sb,
        &io,
        "grep",
        &format!("{{\"path\":\"{}\",\"pattern\":\"[\"}}", dir),
    );
    assert!(!bad.ok, "{}", bad.output);
    assert!(
        bad.output.contains("pattern 不是合法正则") && bad.output.contains("fixed"),
        "{}",
        bad.output
    );

    let bad = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"a\",\"glob\":[\"[\"]}}",
            dir
        ),
    );
    assert!(
        !bad.ok && bad.output.contains("glob 不合法"),
        "{}",
        bad.output
    );

    let none = run_builtin(
        &sb,
        &io,
        "grep",
        &format!("{{\"path\":\"{}\",\"pattern\":\"没有这个\"}}", dir),
    );
    assert!(none.ok, "{}", none.output);
    assert!(none.output.contains("（没有命中行）"), "{}", none.output);
    assert!(
        none.output.contains("no_ignore") && none.output.contains("hidden"),
        "没有命中时要给出逃生口：{}",
        none.output
    );
    assert!(
        none.output.contains("命中 0 行 / 0 个文件"),
        "{}",
        none.output
    );
}

#[test]
pub(crate) fn grep_stops_at_max_and_says_so() {
    let io = InMemorySysIo::new();
    io.seed(&["demo", "a1", "a.txt"], "x1\nx2\nx3\n");
    io.seed(&["demo", "a1", "b.txt"], "x4\n");
    let sb = test_sandbox("a1", &[]);
    let out = run_builtin(
        &sb,
        &io,
        "grep",
        &format!(
            "{{\"path\":\"{}\",\"pattern\":\"x\",\"max\":1}}",
            s(&["demo", "a1"])
        ),
    );
    assert!(out.ok, "{}", out.output);
    assert!(
        out.output.contains("命中 1 行 / 1 个文件"),
        "{}",
        out.output
    );
    assert!(
        out.output.contains("已达到 max=1 条上限"),
        "到上限要如实说：{}",
        out.output
    );
}
