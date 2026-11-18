/**
 * 文档审查·单元索引：docs/ 下一层目录的每份 README.md 都必须在 docs/README.md 里挂上入口。
 * 为什么机器判：新增单元目录时把页写全、忘了在索引里登记，读文档的人就找不到入口——
 *   这类核对是纯事实比对，不该靠人记得回去改索引。
 */
"use strict";
const fs = require("fs");
const path = require("path");

function unitIndex(root) {
  const problems = [];
  const indexPath = path.join(root, "docs", "README.md");
  if (!fs.existsSync(indexPath)) return ["缺文档索引：docs/README.md"];
  const indexText = fs.readFileSync(indexPath, "utf8");
  const linked = new Set();
  for (const m of indexText.matchAll(/\]\(([^)#\s]+\.md)(?:#[^)]*)?\)/g)) {
    if (/^[a-z]+:/i.test(m[1])) continue;
    linked.add(path.posix.normalize(m[1]));
  }
  const docsDir = path.join(root, "docs");
  for (const e of fs.readdirSync(docsDir, { withFileTypes: true })) {
    if (!e.isDirectory()) continue;
    const rel = e.name + "/README.md";
    if (!fs.existsSync(path.join(docsDir, e.name, "README.md"))) continue;
    if (!linked.has(rel)) problems.push("docs/" + rel + " 没在 docs/README.md 的索引表里挂入口");
  }
  return problems;
}

module.exports = { unitIndex };
