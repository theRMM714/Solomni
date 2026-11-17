/**
 * 结构审查·死参（只回执）：具体实现里带下划线的参数，几乎都只是"为了签名好看"而留下的死参数。
 * 目的：把 `decision_for(..., _default)` 这类「传进来就被丢掉」的参数晾出来——它会让调用方以为某个值有作用。
 * 范围：只扫 `impl` 块里**有函数体**的 `fn`；trait 声明与 trait 的默认方法不扫（那里的 `_x` 是实现方可以忽略的参数，是契约）。
 * 效力：只回执（不进失败集），因为替身与平台空实现里确实存在"接口要求但不关心"的参数。
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

/** 参数表里带下划线的形参名（跳过 self 与类型里的下划线）。 */
function deadParams(params) {
  const out = [];
  let depth = 0;
  let cur = "";
  for (const ch of params + ",") {
    if ("<([".includes(ch)) depth++;
    else if (">)]".includes(ch)) depth--;
    if (ch === "," && depth === 0) {
      const m = cur.trim().match(/^_([a-zA-Z][A-Za-z0-9_]*)\s*:/);
      if (m) out.push("_" + m[1]);
      cur = "";
      continue;
    }
    cur += ch;
  }
  return out;
}

/** 找出 `impl` 块里有函数体、且形参带下划线的 fn；空实现与平台 / 替身后端不回执。 */
function deadParamsIn(root, file) {
  const text = mask(fs.readFileSync(file, "utf8"));
  const hits = [];
  // 设计上"不做事"的文件：平台后端、空后端、替身——它们的 `_x` 是契约要求，不是死参。
  const stubFile = /(^|\/)(fake[^/]*|[^/]*_none|none|unix|other|windows)\.rs$/.test(rel(root, file));
  const stack = [];
  let line = 1;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "\n") { line++; continue; }
    if (c === "}") { stack.pop(); continue; }
    if (c !== "{") continue;
    let j = i - 1;
    while (j >= 0 && !/[{};]/.test(text[j])) j--;
    const header = text.slice(j + 1, i);
    const kind = /^\s*(?:pub(?:\([^)]*\))?\s+)?impl\b/.test(header)
      ? "impl"
      : /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\b/.test(header)
        ? "trait"
        : /\bfn\s+[A-Za-z_]/.test(header)
          ? "fn"
          : "other";
    const top = stack[stack.length - 1];
    const inImplBody = top && top.kind === "impl";
    const empty = stubFile || (top && top.empty) || /\bfor\s+No[A-Za-z0-9_]*\s*$/.test(header);
    stack.push({ kind: kind, empty: empty });
    if (kind !== "fn" || !inImplBody || empty) continue;
    const open = header.indexOf("(");
    const close = header.lastIndexOf(")");
    if (open < 0 || close < open) continue;
    const name = (header.slice(0, open).match(/\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/) || [])[1] || "?";
    const params = deadParams(header.slice(open + 1, close));
    if (params.length) hits.push(rel(root, file) + ":" + line + " " + name + " 的 " + params.join("、"));
  }
  return hits;
}

/** 死参回执：返回 notes（不进失败集）。 */
function deadParamsNotes(root) {
  const base = path.join(root, "src");
  if (!fs.existsSync(base)) return [];
  const hits = [];
  for (const file of rsFilesOf(base)) hits.push(...deadParamsIn(root, file));
  if (!hits.length) return [];
  return ["死参（只回执）：impl 里有函数体却没人用的形参——传进来就被丢掉会骗调用方：" + hits.join("；")];
}

module.exports = { deadParamsNotes };