/**
 * 结构审查·代码↔文档：单元地图（docs 下各 unit-map.md）双向一致，以及代码注释里的文档引用。
 * 目的：unit-map 是各单元逐文件职责的唯一权威；人手维护必然漂移（曾出现表错位与整族文件漏记）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const BT = String.fromCharCode(96);

/** 单元地图与磁盘双向一致：每行第一格是仓库根相对路径且存在；src 下每个 .rs 都要在某一张地图里有一行。 */
function unitMap(root) {
  const problems = [];
  const mapFiles = [];
  (function collectMaps(d) {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) collectMaps(p);
      else if (e.name === "unit-map.md") mapFiles.push(p);
    }
  })(path.join(root, "docs"));
  if (!mapFiles.length) problems.push("找不到任何单元地图（docs 下的 unit-map.md）");
  const mapRows = [];
  for (const f of mapFiles) {
    for (const m of fs.readFileSync(f, "utf8").matchAll(new RegExp("^\\| " + BT + "([^" + BT + "]+)" + BT + " \\|", "gm"))) mapRows.push(m[1]);
  }
  const seenRows = new Set();
  for (const r of mapRows) {
    if (seenRows.has(r)) problems.push("单元地图有重复行：" + r);
    seenRows.add(r);
    if (!fs.existsSync(path.join(root, r))) problems.push("单元地图引用的文件不存在：" + r);
  }
  // 目录入口：只声明模块 / 重导出（不定义任何条目）的文件不必逐行入册。
  const isBarrel = (abs) => !/^(pub(\([^)]*\))? )?(fn|struct|enum|impl|trait|const|static|type|macro_rules!) /m.test(fs.readFileSync(abs, "utf8"));
  const collectSrc = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) { collectSrc(p); continue; }
      if (!e.name.endsWith(".rs")) continue;
      const r = rel(root, p);
      if (r === "src/tests.rs" || r.startsWith("src/tests/")) continue;
      if (isBarrel(p)) continue;
      if (!seenRows.has(r)) problems.push("src 下的文件没进单元地图：" + r);
    }
  };
  collectSrc(path.join(root, "src"));
  return problems;
}

/** 代码注释里引用的文档必须存在：注释也是长期文档的一部分。 */
function docRefs(root) {
  const problems = [];
  const check = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) { check(p); continue; }
      if (!e.name.endsWith(".rs")) continue;
      for (const m of fs.readFileSync(p, "utf8").matchAll(/docs\/[A-Za-z0-9_./-]+\.md/g)) {
        if (!fs.existsSync(path.join(root, m[0]))) problems.push(rel(root, p) + " 注释引用的文档不存在：" + m[0]);
      }
    }
  };
  check(path.join(root, "src"));
  return problems;
}

module.exports = { unitMap, docRefs };
