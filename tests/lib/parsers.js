/**
 * T0 质量检查的输出解析器：把 cargo / rustfmt / rustup 的原始输出变成可判定的集合与计数。
 * 目的：解析器决定红绿，坏掉就是**静默全绿**，所以判定与比对只有这一处，自测（gateParserSelfTest）跟着函数走。
 * 路径归一与 '..' 收敛的理由：同一文件可被多个测试目标用 #[path] 引用，三平台报出的路径写法不同，归一后才可比。
 */
"use strict";

const { mask: rustMask } = require("./rust-mask.js");

/** 词法归一：统一分隔符、去掉 \\?\ 扩展前缀、收掉 '.' 与 '..' 段。 */
function normPath(s) {
  const parts = String(s).replace(/\\/g, "/").replace(/^\/\/\?\//, "").split("/");
  const out = [];
  for (const seg of parts) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") { out.pop(); continue; }
    out.push(seg);
  }
  return out.join("/");
}

function rootNorm(root) {
  return normPath(root) + "/";
}

/** fmt --check：有偏差的文件集合（仓库相对路径、/ 分隔）。先剥 ANSI 颜色：带转义前缀的行会被行首锚定漏掉。 */
function fmtDeviations(out, root) {
  const files = new Set();
  const rn = rootNorm(root);
  for (const line of String(out).replace(/\u001b\[[0-9;]*m/g, "").split(/\r?\n/)) {
    const m = line.match(/^Diff in (.+?):\d+:\s*$/);
    if (!m) continue;
    let p = normPath(m[1]);
    if (p.startsWith(rn)) p = p.slice(rn.length);
    files.add(p);
  }
  return files;
}

/** clippy：按 lint 名计数（每条诊断恰好一个 rust-clippy 文档锚点）。 */
function clippyLints(out) {
  const counts = {};
  for (const m of String(out).matchAll(/rust-clippy\/[^#\s]*#([a-z0-9_-]+)/g)) {
    const name = m[1].replace(/-/g, "_");
    counts[name] = (counts[name] || 0) + 1;
  }
  return counts;
}

/** cargo check 的 rustc 层告警数（不把 "generated N warnings" 这类汇总行算进去）。 */
function checkWarnings(out) {
  let n = 0;
  for (const line of String(out).split(/\r?\n/)) {
    if (/^warning: /.test(line) && !/generated \d+ warning/.test(line)) n++;
  }
  return n;
}

/** cargo tree --duplicates 报出的重复 crate 名（列首的 "name vX.Y.Z"）。 */
function duplicateCrates(out) {
  const names = new Set();
  for (const line of String(out).split(/\r?\n/)) {
    const m = line.match(/^([A-Za-z0-9_.-]+) v\d/);
    if (m) names.add(m[1]);
  }
  return [...names].sort();
}

/** 汇总 cargo 的 test result 行：一次运行可能有多条（多目标/多测试）。ignored > 0 由调用方判失败。 */
function cargoCounts(out) {
  let passed = 0, failed = 0, ignored = 0, resultLines = 0;
  for (const line of String(out).split(/\r?\n/)) {
    const m = line.match(/^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored/);
    if (m) { resultLines++; passed += Number(m[2]); failed += Number(m[3]); ignored += Number(m[4]); }
  }
  return { passed, failed, ignored, resultLines };
}

/** 只认**行首**的 [探针]（诚实标记），不把测试输出里恰好含这三个字的内容算成环境跳过。 */
function skipsIn(out) {
  return String(out).split(/\r?\n/).map((l) => l.trim()).filter((l) => l.startsWith("[探针]"));
}

/** links.yaml 一行值的规矩（纯判定）：链接必须 https://；.sha256 键必须 64 位十六进制。 */
function linkValueProblem(key, value) {
  if (key.endsWith(".sha256")) {
    return /^[a-f0-9]{64}$/i.test(value) ? null : "links.yaml 的校验值必须是 64 位十六进制（键 " + key + "）";
  }
  return /^https:\/\//.test(value) ? null : "links.yaml 的值必须是 https://（校验值请写成 `<链接键>.sha256`）：" + key;
}

/**
 * 批处理脚本（.cmd / .bat）里第一处非 ASCII 字节所在行号（1 起）；全 ASCII 给 0。
 * 为什么是硬约束：cmd 按控制台代码页解释字节，UTF-8 多字节会被读成行尾的一部分并吞掉下一行。
 */
function batchNonAscii(buf) {
  const bom = buf.length >= 3 && buf[0] === 0xef && buf[1] === 0xbb && buf[2] === 0xbf;
  let line = 1;
  for (const b of buf) {
    if (b === 0x0a) line++;
    else if (b > 0x7f) return { line, bom };
  }
  return { line: 0, bom };
}

/**
 * 门禁解析器自测：固定样例断言。新增解析器时在这里补一条。
 * 返回问题数组（空 = 通过）；失败文案与历史一致，报告里能直接搜到。
 */
function gateParserSelfTest(root) {
  const bad = [];
  const eq = (name, got, want) => {
    const g = JSON.stringify(got);
    const w = JSON.stringify(want);
    if (g !== w) bad.push("门禁解析器自测失败：" + name + "：实际 " + g + "，期望 " + w);
  };
  eq("normPath 收 .. 并统一分隔符", normPath("tests/cross-platform/../helpers/probe.rs"), "tests/helpers/probe.rs");
  eq("normPath 去 \\\\?\\ 前缀并收 ..", normPath("\\\\?\\tests\\..\\src\\main.rs"), "src/main.rs");
  const rootFmt = rootNorm(root) + "src/x.rs";
  eq(
    "fmtDeviations 穿透 ANSI 颜色并按根归一",
    [...fmtDeviations("\u001b[1mDiff in " + rootFmt + ":3:\u001b[0m\nDiff in " + rootFmt + ":9:\n", root)],
    ["src/x.rs"],
  );
  eq(
    "clippyLints 从文档锚点取名并归一连字符",
    clippyLints("...rust-clippy/master/index.html#too_many_arguments...\n...rust-clippy/master/index.html#too-many-arguments..."),
    { too_many_arguments: 2 },
  );
  eq(
    "checkWarnings 跳过 generated N warnings 汇总行",
    checkWarnings("warning: unused import\nwarning: solomni (bin) generated 1 warning\nwarning: dead code\n"),
    2,
  );
  eq("duplicateCrates 去重并按名排序", duplicateCrates("serde v1.0.1\nserde v1.0.2\nfoo v0.1.0\n"), ["foo", "serde"]);
  eq(
    "cargoCounts 多目标求和并拿到 ignored",
    cargoCounts(
      "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n" +
        "test result: FAILED. 1 passed; 2 failed; 4 ignored; 0 measured; 0 filtered out\n",
    ),
    { passed: 4, failed: 2, ignored: 4, resultLines: 2 },
  );
  eq("skipsIn 只认行首的 [探针]", skipsIn("ok\n[探针] 环境不允许\n其它 [探针] 出现在中间\n"), ["[探针] 环境不允许"]);
  eq("linkValueProblem 放过 https 链接", linkValueProblem("winlibs.upstream.asset", "https://x/y.zip"), null);
  eq("linkValueProblem 放过 64 位校验值（大小写都行）", linkValueProblem("a.b.sha256", "D5DBAFC4".repeat(8)), null);
  eq("linkValueProblem 拒非十六进制校验值", /校验值必须/.test(linkValueProblem("a.b.sha256", "d5db")), true);
  eq("linkValueProblem 拒裸 http（并指到 .sha256 写法）", /https/.test(linkValueProblem("a.b", "http://x")), true);
  eq("batchNonAscii 放过纯 ASCII 脚本", batchNonAscii(Buffer.from("@echo off\r\necho OK\r\n")), { line: 0, bom: false });
  eq("batchNonAscii 报出非 ASCII 所在行", batchNonAscii(Buffer.from("@echo off\nrem 中文\necho OK\n")), { line: 2, bom: false });
  eq("batchNonAscii 认 UTF-8 BOM", batchNonAscii(Buffer.from([0xef, 0xbb, 0xbf, 0x40])).bom, true);
  eq("batchNonAscii 非 ASCII 在首行时不误算行号", batchNonAscii(Buffer.from("echo 中\n")).line, 1);
  const rustSource =
    "// 行注释里的 ureq 不是 IO\n" +
    'let s = "std::fs";\n' +
    'let r = r#"ureq "x""#;\n' +
    "/* 外层 /* 内层 ureq */ 仍在注释 */\n" +
    "let c = 'a';\n" +
    "fn f<'t>(x: &'t str) -> &'t str { x }\n";
  const rustMasked = rustMask(rustSource);
  eq("rustMask 抹掉行注释与普通/raw 字符串里的记号", /ureq|std::fs/.test(rustMasked), false);
  eq("rustMask 处理可嵌套块注释且在正确深度收尾", rustMasked.includes("let c ="), true);
  eq("rustMask 抹掉字符字面量", rustMasked.includes("'a'"), false);
  eq("rustMask 不把生命周期 't 当字符字面量吞掉后续代码", rustMasked.includes("-> &'t str { x }"), true);
  eq(
    "rustMask 保留长度与换行数",
    [rustMasked.length, (rustMasked.match(/\n/g) || []).length],
    [rustSource.length, (rustSource.match(/\n/g) || []).length],
  );
  eq("rustMask 跳过字符串里的转义引号", rustMask('let s = "a\\"ureq";').includes("ureq"), false);
  return bad;
}

module.exports = {
  normPath,
  rootNorm,
  fmtDeviations,
  clippyLints,
  checkWarnings,
  duplicateCrates,
  cargoCounts,
  skipsIn,
  linkValueProblem,
  batchNonAscii,
  gateParserSelfTest,
};
