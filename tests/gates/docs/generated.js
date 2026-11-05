/**
 * 文档审查·生成段同步：平台验收状态等生成段必须与代码常量 / 缺口账一致。
 * 生成与比对只在 tests/docs-sync.mjs 一处，这里只负责调用。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

function generated(root) {
  const problems = [];
  const sync = path.join(root, "tests", "docs-sync.mjs");
  if (!fs.existsSync(sync)) return problems;
  const r = spawnSync(process.execPath, [sync, "--check"], { cwd: root, encoding: "utf8" });
  if (r.stdout) process.stdout.write(r.stdout);
  if (r.status !== 0) problems.push("生成段与来源不一致：跑 node tests/docs-sync.mjs --write");
  return problems;
}

module.exports = { generated };
