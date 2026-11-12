/**
 * 结构审查·测试目标：Cargo.toml 的 [[test]] 登记与"孤儿测试文件"。
 * 目的：新增测试文件必须挂在某个目标上，否则它永远不会被入口跑到（结构质量问题，不是覆盖率问题）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");

/** 登记：每个 [[test]] 都要有 name/path，入口必须存在；至少一个目标。 */
function registration(root, ctx) {
  const problems = [];
  const targets = [];
  const toml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
  for (const block of toml.split(/\[\[test\]\]/).slice(1)) {
    const name = (block.match(/^\s*name\s*=\s*"([^"]+)"/m) || [])[1];
    const p = (block.match(/^\s*path\s*=\s*"([^"]+)"/m) || [])[1];
    if (!name || !p) { problems.push("Cargo.toml 有 [[test]] 缺 name 或 path"); continue; }
    if (!fs.existsSync(path.join(root, p))) problems.push("测试目标 " + name + " 的入口不存在：" + p);
    targets.push({ name, entry: path.join(root, p) });
  }
  if (!targets.length) problems.push("Cargo.toml 没有登记任何 [[test]] 目标");
  if (ctx && ctx.stats) ctx.stats.targets = targets.map((t) => t.name);
  return problems;
}

/** 从目标入口出发按 mod 声明走一遍，得到"Cargo 真会编译的测试文件"集合。 */
function reachableFiles(root) {
  const reachable = new Set();
  const walk = (file) => {
    const abs = path.resolve(file);
    if (reachable.has(abs) || !fs.existsSync(abs)) return;
    reachable.add(abs);
    const dir = path.dirname(abs);
    // 子模块目录：a/b.rs 的子女在 a/b/ 下；crate 根（src/main.rs、tests/<目标>/main.rs）的在自己目录下。
    const bases = path.basename(abs) === "main.rs" ? [dir] : [abs.replace(/\.rs$/, "")];
    const re = /(?:#\[path\s*=\s*"([^"]+)"\]\s*)?(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/g;
    let m;
    while ((m = re.exec(fs.readFileSync(abs, "utf8")))) {
      const mod = m[2];
      if (!mod) continue;
      const cands = m[1]
        ? [path.resolve(dir, m[1])]
        : bases.map((b) => path.join(b, mod + ".rs"));
      for (const c of cands) if (fs.existsSync(c)) { walk(c); break; }
    }
  };
  // 只从 [[test]] 入口出发；入口缺失时跳过（registration 已经报过）。
  const toml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
  for (const block of toml.split(/\[\[test\]\]/).slice(1)) {
    const p = (block.match(/^\s*path\s*=\s*"([^"]+)"/m) || [])[1];
    if (p) walk(path.join(root, p));
  }
  return reachable;
}

/** 孤儿：tests/ 下没有任何目标引用的 .rs（含平台专属文件）。 */
function orphans(root, ctx) {
  const problems = [];
  const reachable = reachableFiles(root);
  const allTestFiles = [];
  const collect = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) collect(p);
      else if (e.name.endsWith(".rs")) allTestFiles.push(p);
    }
  };
  collect(path.join(root, "tests"));
  for (const f of allTestFiles) {
    if (!reachable.has(path.resolve(f))) problems.push("孤儿测试文件（没有任何测试目标引用它）：" + rel(root, f));
  }
  if (ctx && ctx.stats) ctx.stats.testFiles = allTestFiles.length;
  return problems;
}

module.exports = { registration, orphans, reachableFiles };
