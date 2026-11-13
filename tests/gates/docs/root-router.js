/**
 * 文档审查·根文档路由：根目录每个 .md 都必须进 AGENTS.md 的文档路由表（新增根文档不能只落在盘上）。
 * 白名单只有 AGENTS.md 自己（路由表不列它）与 plan.md（根目录临时计划文档，已进 .gitignore；用完即删）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const BT = String.fromCharCode(96);

function rootRouter(root) {
  const problems = [];
  // plan.md = 根目录临时计划：.gitignore 忽略、用完即删，不参与路由表。
  const rootDocAllow = ["AGENTS.md", "plan.md"];
  const agentsDoc = path.join(root, "AGENTS.md");
  if (!fs.existsSync(agentsDoc)) return problems;
  const agentsText = fs.readFileSync(agentsDoc, "utf8");
  for (const e of fs.readdirSync(root)) {
    if (!e.endsWith(".md") || rootDocAllow.includes(e)) continue;
    if (!agentsText.includes(BT + e + BT)) problems.push("根文档 " + e + " 没进 AGENTS.md 的文档路由表");
  }
  return problems;
}

module.exports = { rootRouter };
