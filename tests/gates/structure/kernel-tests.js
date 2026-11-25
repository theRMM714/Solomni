/**
 * 结构审查·内核测试边界：src/kernel 的内联测试必须用**真实实例**（临时目录、真实进程、真文件系统），
 * 不得引用 crate::tests 的业务替身——内核是稳定地基，替身在这里会把"替身的行为"当成内核的事实。
 * 目的：守住 TESTING.md 的「内核用真实实例」；业务替身只服务单元层。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { mask } = require("../../lib/rust-mask.js");

// 真实实例夹具（`scratch` 临时目录、`fence_acl_guard` ACL 串行锁）在内核里合理；
// 要拦的是业务替身：doubles / builders / prelude / fakes——它们按单元装配，会把"替身的行为"写成内核的事实。
const DOUBLE_USE = /\bcrate::tests::(doubles|builders|prelude|fakes)\b/;

function kernelTests(root) {
  const problems = [];
  const base = path.join(root, "src", "kernel");
  if (!fs.existsSync(base)) return problems;
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) {
        walk(p);
        continue;
      }
      if (!e.name.endsWith(".rs")) continue;
      const r = rel(root, p);
      const lines = mask(fs.readFileSync(p, "utf8")).split(/\r?\n/);
      for (let i = 0; i < lines.length; i++) {
        if (!DOUBLE_USE.test(lines[i])) continue;
        problems.push(
          `${r}:${i + 1} 内核测试必须用真实实例，不得引用 crate::tests 的业务替身（doubles/builders/prelude/fakes）：${lines[i].trim()}`
        );
      }
    }
  };
  walk(base);
  return problems;
}

module.exports = { kernelTests };
