/**
 * 结构审查·同名导入：一个文件里同时导入两个**同名不同义**的类型，`as` 只是把歧义藏进别名。
 * 目的：两个同名类型出现在同一文件时，读代码的人必须先查别名才知道说的是哪一个；
 *   治理办法是改名（让名字自己说清是谁），不是起别名。
 * 判据：同一文件的 `use` 里，某个类型的末段名（`as` 之前）出现 ≥2 次且来自**不同路径**即失败。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { mask } = require("../../lib/rust-mask.js");

function rsFilesOf(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name === "tests") continue;
      rsFilesOf(p, out);
    } else if (e.name.endsWith(".rs") && !/(^|_)tests\.rs$/.test(e.name)) out.push(p);
  }
  return out;
}

/** 一条 `use` 语句里的导入项：[{ name, full }]；跳过 self / super / 通配 / 分组前缀。 */
function itemsOf(stmt, base) {
  const out = [];
  const m = stmt.match(/^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+([\s\S]+?);\s*$/);
  if (!m) return out;
  let body = m[1].replace(/\s+/g, " ").trim();
  let prefix = "";
  const brace = body.indexOf("{");
  if (brace >= 0) {
    prefix = body.slice(0, brace).replace(/::$/, "");
    if (/^(super|self|crate::super|crate::self)$/.test(prefix)) return out;
    body = body.slice(brace + 1, body.lastIndexOf("}"));
  }
  for (const raw of body.split(",")) {
    const item = raw.trim();
    // `super::` / `self::` 是同一个类型换一种写法（模块内重导），不是同名不同义。
    if (!item || item === "*" || item === "self" || /^(super|self)::/.test(item)) continue;
    const noAlias = item.replace(/\s+as\s+[A-Za-z_][A-Za-z0-9_]*$/, "");
    const last = noAlias.split("::").pop().trim();
    if (!/^[A-Z][A-Za-z0-9_]*$/.test(last)) continue;
    out.push({ name: last, full: (prefix ? prefix + "::" : "") + noAlias, line: base });
  }
  return out;
}

/** 同名导入：返回问题列表（空 = 没有文件同时导入两个同名类型）。 */
function importNames(root) {
  const problems = [];
  const base = path.join(root, "src");
  if (!fs.existsSync(base)) return problems;
  for (const file of rsFilesOf(base)) {
    const text = mask(fs.readFileSync(file, "utf8"));
    const starts = [];
    let at = 0;
    for (const line of text.split(/\r?\n/)) {
      starts.push(at);
      at += line.length + 1;
    }
    const lineOf = (pos) => {
      let lo = 0;
      let hi = starts.length - 1;
      while (lo < hi) {
        const mid = (lo + hi + 1) >> 1;
        if (starts[mid] <= pos) lo = mid;
        else hi = mid - 1;
      }
      return lo + 1;
    };
    const byName = new Map();
    for (const m of text.matchAll(/^[ \t]*(?:pub(?:\([^)]*\))?\s+)?use\s+[^;]+;/gm)) {
      for (const item of itemsOf(m[0], lineOf(m.index))) {
        if (!byName.has(item.name)) byName.set(item.name, []);
        byName.get(item.name).push(item);
      }
    }
    for (const [name, list] of byName) {
      const fulls = [...new Set(list.map((i) => i.full))];
      if (fulls.length < 2) continue;
      const where = list.map((i) => i.full + "（第 " + i.line + " 行）").join(" 与 ");
      problems.push(
        "同名导入：" + rel(root, file) + " 同时导入两个 " + name + "——" + where + "。改名让名字自证身份，别用 as 遮住歧义"
      );
    }
  }
  return problems;
}

module.exports = { importNames };
