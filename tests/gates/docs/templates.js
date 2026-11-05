/**
 * 文档审查·模板合规：模板与类型映射在 docs/templates/，强制类型为空时只报不拦。
 * 判定只在 tests/docs-gov.mjs 一处，这里只负责调用并把它的输出转成问题（不复制检查逻辑）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

function templates(root) {
  const problems = [];
  const gov = path.join(root, "tests", "docs-gov.mjs");
  if (!fs.existsSync(gov)) return problems;
  const r = spawnSync(process.execPath, [gov, "--summary"], { cwd: root, encoding: "utf8" });
  if (r.stdout) process.stdout.write(r.stdout);
  if (r.status !== 0) problems.push("文档模板不合规：见上方 docs-gov 输出");
  return problems;
}

module.exports = { templates };
