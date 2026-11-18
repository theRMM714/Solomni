/**
 * 文档审查·根文档路由：根目录每个 .md 都必须在 AGENTS.md 的文档路由表里有一行（新增根文档不能只落在盘上）。
 * 判的是"表行"，不是"全文出现过"：正文里顺口提一句不算路由。反方向也查——表行里点名的根文档必须真的在盘上。
 * 白名单只有 AGENTS.md 自己（路由表不列它）；临时材料放 `.tmp/`（`.gitignore` 已忽略），不产生根文档。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const BT = String.fromCharCode(96);

function rootRouter(root) {
  const problems = [];
  const rootDocAllow = ["AGENTS.md"];
  const agentsDoc = path.join(root, "AGENTS.md");
  if (!fs.existsSync(agentsDoc)) return problems;
  const rows = fs
    .readFileSync(agentsDoc, "utf8")
    .split(/\r?\n/)
    .filter((l) => l.startsWith("|") && !/^\|\s*-+/.test(l));
  const rowText = rows.join("\n");
  for (const e of fs.readdirSync(root)) {
    if (!e.endsWith(".md") || rootDocAllow.includes(e)) continue;
    if (!rowText.includes(BT + e + BT) && !rowText.includes("(" + e + ")")) {
      problems.push("根文档 " + e + " 没进 AGENTS.md 的文档路由表");
    }
  }
  for (const row of rows) {
    for (const m of row.matchAll(new RegExp(BT + "([A-Za-z0-9_.-]+\\.md)" + BT, "g"))) {
      const name = m[1];
      if (name === "AGENTS.md" || rootDocAllow.includes(name)) continue;
      if (!fs.existsSync(path.join(root, name))) problems.push("AGENTS.md 的路由表点名了不存在的根文档：" + name);
    }
  }
  return problems;
}

module.exports = { rootRouter };
