/**
 * 结构审查·真相源比对：提示词键表 ↔ prompts 册子、systools 表 ↔ 门户。
 * 为什么机器查：这些"某份文档列出另一处清单"的段落，人一定会忘记同步。
 */
"use strict";
const fs = require("fs");
const path = require("path");

/** 提示词键表：docs/prompt/prompts.md 的「结构与键」是"每份文件里有什么键"的唯一权威，双向比对。 */
function promptKeys(root) {
  const problems = [];
  const promptsDoc = path.join(root, "docs", "prompt", "prompts.md");
  const promptsRoot = path.join(root, "prompts");
  if (!fs.existsSync(promptsDoc) || !fs.existsSync(promptsRoot)) return problems;
  const stated = new Map(); // 文件（相对 prompts/）→ 表里提到的顶层键
  let tableFile = null;
  for (const line of fs.readFileSync(promptsDoc, "utf8").split(/\r?\n/)) {
    if (!line.trim().startsWith("|")) continue;
    const cells = line.split("|").map((c) => c.trim());
    if (cells.length < 4) continue;
    if (/^[-: ]+$/.test(cells[1]) && /^[-: ]+$/.test(cells[2])) continue; // 分隔行
    const cell = cells[1].replace(/`/g, "").trim();
    if (cell) tableFile = cell;
    if (!tableFile || !tableFile.endsWith(".yaml")) continue;
    if (!stated.has(tableFile)) stated.set(tableFile, new Set());
    for (const m of cells[2].matchAll(/`([A-Za-z_][A-Za-z0-9_]*)/g)) stated.get(tableFile).add(m[1]);
  }
  const topKeys = (abs) =>
    new Set(
      fs.readFileSync(abs, "utf8").split(/\r?\n/)
        .map((l) => { const m = l.match(/^([A-Za-z_][A-Za-z0-9_]*):/); return m && m[1]; })
        .filter(Boolean),
    );
  for (const [file, keys] of stated) {
    const abs = path.join(promptsRoot, file);
    if (!fs.existsSync(abs)) { problems.push("提示词键表列了不存在的文件：" + file); continue; }
    const top = topKeys(abs);
    for (const k of keys) if (!top.has(k)) problems.push("提示词键表里的键已不存在：" + file + " 的 " + k);
    for (const k of top) if (!keys.has(k)) problems.push("提示词的键没进键表：" + file + " 的 " + k + "（docs/prompt/prompts.md「结构与键」）");
  }
  const walkPrompts = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) { walkPrompts(p); continue; }
      if (!e.name.endsWith(".yaml")) continue;
      const relf = path.relative(promptsRoot, p).replace(/\\/g, "/");
      if (!stated.has(relf)) problems.push("提示词册的文件没进键表：" + relf);
    }
  };
  walkPrompts(promptsRoot);
  return problems;
}

/** systools 真相源表：每张表都要在门户或细则里被点名（新增表不能只落在盘上）。 */
function systools(root) {
  const problems = [];
  const systoolsDir = path.join(root, "systools");
  if (!fs.existsSync(systoolsDir)) return problems;
  const tablesDoc = ["SYSTOOL.md", "docs/tools/tools-and-roles.md"]
    .map((f) => { const abs = path.join(root, f); return fs.existsSync(abs) ? fs.readFileSync(abs, "utf8") : ""; })
    .join("\n");
  for (const e of fs.readdirSync(systoolsDir)) {
    if (!e.endsWith(".yaml")) continue;
    if (!tablesDoc.includes(e)) problems.push("systools/" + e + " 没被 SYSTOOL.md 或 docs/tools/tools-and-roles.md 提到（新增真相源表要进门户）");
  }
  return problems;
}

module.exports = { promptKeys, systools };
