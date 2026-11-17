/**
 * 结构审查·相似函数（只回执）：两段代码 3-gram 包含度过高，就可能是同一条知识写了两遍。
 * 目的：给「同一事实两处回答」一张自动化的候选清单；判定权在人和评审，不在脚本——
 *   形状像不等于语义同（误报率实测不低），所以只回执、不进失败集。
 * 判据：token（标识符序列）3-gram 包含度：跨文件 ≥ 0.72、同文件 ≥ 0.88，函数体 ≥ 12 行才比。
 * 豁免：平台双实现（windows / unix / other 三兄弟之间不比）——那是平台差异，不是重复。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { mask } = require("../../lib/rust-mask.js");

const CROSS = 0.72;
const SAME = 0.88;
const MIN_LINES = 12;
const MIN_GRAMS = 30;
const MIN_SHARED = 6;
const BUCKET_CAP = 30;
const REPORT_CAP = 8;

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

/** 抹掉 `#[cfg(test)]` 模块：内联用例不是生产实现，比出"相似"只会是测试写法像。 */
function withoutTestMods(text) {
  let out = text;
  let at = out.indexOf("#[cfg(test)]");
  while (at >= 0) {
    const open = out.indexOf("{", at);
    if (open < 0) break;
    let depth = 0;
    let end = out.length;
    for (let i = open; i < out.length; i++) {
      if (out[i] === "{") depth++;
      else if (out[i] === "}" && --depth === 0) {
        end = i + 1;
        break;
      }
    }
    out = out.slice(0, at) + " ".repeat(end - at) + out.slice(end);
    at = out.indexOf("#[cfg(test)]", end);
  }
  return out;
}

/** 3-gram 集合：标识符序列上滑三位（够粗，形状像不等语义同）。 */
function gramsOf(body) {
  const ids = body.match(/[A-Za-z_][A-Za-z0-9_]*/g) || [];
  const out = new Set();
  for (let i = 0; i + 2 < ids.length; i++) out.add(ids[i] + " " + ids[i + 1] + " " + ids[i + 2]);
  return out;
}

/** 文件里的全部有体函数：[{ file, name, line, grams, lines }]。 */
function functionsOf(root, file) {
  const text = withoutTestMods(mask(fs.readFileSync(file, "utf8")));
  const out = [];
  const re = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^<>]*>)?\s*\(/g;
  let m;
  while ((m = re.exec(text))) {
    // 形参括号配对，再找函数体。
    let i = text.indexOf("(", m.index);
    let depth = 0;
    for (; i < text.length; i++) {
      if (text[i] === "(") depth++;
      else if (text[i] === ")" && --depth === 0) break;
    }
    // 形参之后可能还有返回类型与 where 子句：往上找到 `{`（有体）或 `;`（声明）。
    let k = i + 1;
    while (k < text.length && text[k] !== "{" && text[k] !== ";") k++;
    if (text[k] !== "{") continue;
    const bodyStart = k + 1;
    let braces = 1;
    for (k = bodyStart; k < text.length && braces > 0; k++) {
      if (text[k] === "{") braces++;
      else if (text[k] === "}") braces--;
    }
    const body = text.slice(bodyStart, k);
    const lines = (body.match(/\n/g) || []).length;
    if (lines < MIN_LINES) continue;
    const grams = gramsOf(body);
    // 太短的函数 3-gram 少，任何两段同构骨架都会"很像"；只在体量相当的对之间比。
    if (grams.size < MIN_GRAMS) continue;
    const line = (text.slice(0, m.index).match(/\n/g) || []).length + 1;
    out.push({ file: rel(root, file), name: m[1], line: line, grams: grams, lines: lines, start: m.index, end: k });
  }
  return out;
}

/** 平台双实现之间不比：同一件事在三个后端各写一遍是有意的。 */
const BACKEND = /(windows|unix|other)\.rs$/;
function samePlatform(a, b) {
  const x = a.match(BACKEND);
  const y = b.match(BACKEND);
  return Boolean(x && y && x[1] !== y[1]);
}

/** 相似函数回执：返回 notes（不进失败集）。 */
function similarNotes(root) {
  const base = path.join(root, "src");
  if (!fs.existsSync(base)) return [];
  const fns = [];
  for (const file of rsFilesOf(base)) fns.push(...functionsOf(root, file));
  const index = new Map();
  for (let i = 0; i < fns.length; i++) {
    for (const g of fns[i].grams) {
      if (!index.has(g)) index.set(g, []);
      index.get(g).push(i);
    }
  }
  const shared = new Map();
  for (const bucket of index.values()) {
    if (bucket.length < 2 || bucket.length > BUCKET_CAP) continue;
    for (let a = 0; a < bucket.length; a++) {
      for (let b = a + 1; b < bucket.length; b++) {
        const key = bucket[a] + ":" + bucket[b];
        shared.set(key, (shared.get(key) || 0) + 1);
      }
    }
  }
  const hits = [];
  for (const [key, count] of shared) {
    if (count < MIN_SHARED) continue;
    const [ia, ib] = key.split(":").map(Number);
    const a = fns[ia];
    const b = fns[ib];
    if (samePlatform(a.file, b.file)) continue;
    // 内嵌函数：一个的体在另一个的体里（`fn a() { fn b() {…} }`），不算两处回答。
    if (a.file === b.file && ((a.start <= b.start && b.end <= a.end) || (b.start <= a.start && a.end <= b.end))) continue;
    // 一边是另一边的"子集"不算重复：那更像同一个骨架的两段不同用途。
    const small = Math.min(a.grams.size, b.grams.size);
    if (small * 2 < Math.max(a.grams.size, b.grams.size)) continue;
    const ratio = count / small;
    const bar = a.file === b.file ? SAME : CROSS;
    if (ratio < bar) continue;
    hits.push({ ratio: ratio, a: a, b: b });
  }
  if (!hits.length) return [];
  hits.sort((x, y) => y.ratio - x.ratio);
  const shown = hits.slice(0, REPORT_CAP).map((h) => {
    return h.a.file + ":" + h.a.line + " " + h.a.name + " ≈ " + h.b.file + ":" + h.b.line + " " + h.b.name + "（" + Math.round(h.ratio * 100) + "%）";
  });
  const more = hits.length > shown.length ? "；另有 " + (hits.length - shown.length) + " 对未列出" : "";
  return ["相似函数（只回执）：token 3-gram 包含度高的函数对，先判是不是同一条知识写了两遍：" + shown.join("；") + more];
}

module.exports = { similarNotes };
