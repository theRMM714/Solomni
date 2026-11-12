/**
 * 结构审查·模块入口文件：Rust 2018 起模块入口可用自命名文件 X.rs 与 X/ 目录并列，替代 X/mod.rs。
 * 本仓只保留一种形态：src/ 与 tests/ 下出现任何 mod.rs 都算布局问题（规则与判据见 docs/rust-dev.md）。
 */
"use strict";
const fs = require("fs");
const path = require("path");

function walk(dir, out) {
  if (!fs.existsSync(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name === "mod.rs") out.push(p);
  }
  return out;
}

function layout(root) {
  const problems = [];
  for (const sub of ["src", "tests"]) {
    for (const abs of walk(path.join(root, sub), [])) {
      const rel = path.relative(root, abs).replace(/\\/g, "/");
      const dir = rel.slice(0, -"/mod.rs".length);
      problems.push("模块入口请用自命名文件：" + rel + " → 改为 " + dir + ".rs，与 " + dir + "/ 目录并列（docs/rust-dev.md）");
    }
  }
  return problems;
}

module.exports = { layout };
