#!/usr/bin/env node
/**
 * 覆盖率门禁：把 llvm-cov 文本报告折算成风险区指标，并剔除测试代码。
 *
 * 用法：
 *   node tests/coverage.mjs --report target/coverage/coverage.txt [--json target/coverage/summary.json] [--check]
 *   node tests/coverage.mjs --collect [--check]     # 跑测试 + 插桩构建 + e2e + 报告
 *   node tests/coverage.mjs --report ... --diff-base origin/main   # 加判变更行
 *
 * 为什么要剔除测试代码：内联 #[cfg(test)] 和 src/tests/** 本身就是测试，
 * 把它们算进产品行覆盖率只会让数字好看，不能说明产品行为被测到。
 */

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = path.join(ROOT, "target", "coverage");
const POLICY = JSON.parse(fs.readFileSync(path.join(ROOT, "tests", "coverage-policy.json"), "utf8"));

function arg(name) {
  const hit = process.argv.find((a) => a === name || a.startsWith(name + "="));
  if (!hit) return null;
  if (hit.includes("=")) return hit.slice(hit.indexOf("=") + 1);
  const i = process.argv.indexOf(hit);
  return process.argv[i + 1] && !process.argv[i + 1].startsWith("--") ? process.argv[i + 1] : true;
}

function parseCount(text) {
  const t = text.trim().replace(/,/g, "");
  if (!t) return null;
  const m = t.match(/^([0-9.]+)([kKmM]?)$/);
  if (!m) return null;
  const n = Number(m[1]) * (m[2].toLowerCase() === "k" ? 1000 : m[2].toLowerCase() === "m" ? 1000000 : 1);
  return Number.isFinite(n) ? n : null;
}

function normalizeFile(name) {
  const p = name.replace(/\\/g, "/");
  const marker = p.lastIndexOf("/src/");
  if (marker >= 0) return p.slice(marker + 1);
  const tm = p.lastIndexOf("/tests/");
  if (tm >= 0) return p.slice(tm + 1);
  if (p.startsWith("src/") || p.startsWith("tests/")) return p;
  return path.relative(ROOT, p).replace(/\\/g, "/");
}

function isTestFile(rel) {
  if (rel.startsWith("src/tests/")) return true;
  if (rel.startsWith("tests/")) return true;
  if (/(^|\/)tests\.rs$/.test(rel)) return true;
  if (/(^|\/)tests\//.test(rel)) return true;
  if (/(^|\/)build\.rs$/.test(rel)) return true;
  return false;
}

/** 把 #[cfg(test)] 覆盖的行标出来：从属性行到配对右花括号。 */
function testLineMask(source) {
  const lines = source.split(/\r?\n/);
  const mask = new Array(lines.length).fill(false);
  for (let i = 0; i < lines.length; i++) {
    if (!/#\[cfg\([^\]]*\btest\b/.test(lines[i])) continue;
    let j = i;
    while (j < lines.length && !lines[j].includes("{")) {
      mask[j] = true;
      if (lines[j].includes(";")) break;
      j++;
    }
    if (j >= lines.length) continue;
    let depth = 0;
    let state = "code";
    let raw = 0;
    for (let k = j; k < lines.length; k++) {
      const line = lines[k];
      mask[k] = true;
      for (let x = 0; x < line.length; x++) {
        const c = line[x];
        const n = line[x + 1];
        if (state === "line") break;
        if (state === "block") {
          if (c === "*" && n === "/") { state = "code"; x++; }
          continue;
        }
        if (state === "string") {
          if (c === "\\") { x++; continue; }
          if (c === '"') state = "code";
          continue;
        }
        if (state === "char") {
          if (c === "\\") { x++; continue; }
          if (c === "'") state = "code";
          continue;
        }
        if (c === "/" && n === "/") { state = "line"; break; }
        if (c === "/" && n === "*") { state = "block"; x++; continue; }
        if (raw > 0) continue;
        if (c === '"') { state = "string"; continue; }
        if (c === "'") { state = "char"; continue; }
        if (c === "{") depth++;
        else if (c === "}") {
          depth--;
          if (depth === 0) { k = lines.length; break; }
        }
      }
      if (state === "line") state = "code";
    }
  }
  return mask;
}

function globToRe(glob) {
  let re = "";
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === "*" && glob[i + 1] === "*") { re += ".*"; i++; }
    else if (c === "*") re += "[^/]*";
    else if (c === "?") re += "[^/]";
    else re += c.replace(/[.+^$(){}|[\]\\]/g, "\\$&");
  }
  return new RegExp("^" + re + "$");
}

export function parseReport(text, root = ROOT) {
  const files = [];
  let cur = null;
  for (const line of text.split(/\r?\n/)) {
    if (!line.includes("|") && /:\s*$/.test(line)) {
      cur = { file: normalizeFile(line.slice(0, -1).trim()), lines: [] };
      files.push(cur);
      continue;
    }
    const m = line.match(/^\s*(\d+)\|([^|]*)\|(.*)$/);
    if (!m || !cur) continue;
    const count = parseCount(m[2]);
    if (count === null) continue;
    cur.lines.push({ line: Number(m[1]), count, source: m[3] });
  }
  return files;
}

function exclusionMap() {
  const map = new Map();
  for (const x of POLICY.excludes || []) {
    if (!x.file || !x.reason || !x.reason.trim() || !Number.isInteger(x.from) || !Number.isInteger(x.to) || x.to < x.from) {
      throw new Error("coverage-policy.json 的 excludes 必须带 file/from/to/reason：" + JSON.stringify(x));
    }
    const set = map.get(x.file) || new Set();
    for (let n = x.from; n <= x.to; n++) set.add(n);
    map.set(x.file, set);
  }
  return map;
}

export function evaluate(files, root = ROOT) {
  const exclusions = exclusionMap();
  const buckets = POLICY.buckets.map((b) => ({ ...b, re: b.globs.map(globToRe), covered: 0, total: 0, files: [] }));
  const fileRows = [];
  let excluded = 0;
  for (const f of files) {
    const rel = f.file;
    if (isTestFile(rel)) continue;
    const skip = exclusions.get(rel);
    let mask = null;
    try {
      mask = testLineMask(fs.readFileSync(path.join(root, rel), "utf8"));
    } catch {
      mask = [];
    }
    let covered = 0;
    let total = 0;
    const uncovered = [];
    for (const l of f.lines) {
      if (mask[l.line - 1]) continue;
      if (skip && skip.has(l.line)) { excluded++; continue; }
      total++;
      if (l.count > 0) covered++;
      else if (uncovered.length < 200) uncovered.push(l.line);
    }
    if (!total) continue;
    const row = { file: rel, covered, total, pct: (100 * covered) / total };
    fileRows.push(row);
    for (const b of buckets) {
      if (b.re.some((re) => re.test(rel))) {
        b.covered += covered;
        b.total += total;
        b.files.push(row);
      }
    }
  }
  for (const b of buckets) b.pct = b.total ? (100 * b.covered) / b.total : 0;
  return { buckets, files: fileRows, excluded };
}

function print(result) {
  console.log("[覆盖率] 口径：剔除 src/tests/**、*tests.rs、#[cfg(test)] 区间；E2E 不设行覆盖门。");
  if (result.excluded) {
    console.log("  [口径] 显式 seam 豁免 " + result.excluded + " 行（逐条见 tests/coverage-policy.json 的 reason）");
  }
  for (const b of result.buckets) {
    const ok = b.pct >= b.min;
    console.log(
      "  " + (ok ? "PASS" : "FAIL") + " " + b.id.padEnd(10) +
      " " + b.pct.toFixed(2).padStart(6) + "% (min " + b.min + "%) " +
      b.covered + "/" + b.total + "  " + b.title
    );
  }
  const bad = result.buckets.filter((b) => b.pct < b.min);
  for (const b of bad) {
    console.log("  [" + b.id + "] 缺口最大文件：");
    for (const f of b.files.slice().sort((a, z) => a.pct - z.pct).slice(0, 8)) {
      console.log("    " + f.pct.toFixed(1).padStart(5) + "% " + f.covered + "/" + f.total + " " + f.file);
    }
  }
}

/**
 * 搬运行识别：纯搬运（换文件、换路径限定、rustfmt 重排）不是新代码，不该被变更行门禁审判——
 * 它没有引入新逻辑，覆盖率也不会因为搬一次家就变。
 * 两层判据：函数级（fn 体抹掉注释与字符串、去掉路径限定后 token 相似度 ≥0.98 → 整函数算搬运）
 * 与行键兜底（归一化后逐字相同的行；给被改写了外壳的入口分派用）。
 * 兜底只从本次 PR 触及文件的基线版本里取行，且每个基线行只抵一次，避免一行抵掉无限新行。
 */
const MOVE_KEY_MIN = 12;
const MOVE_SIMILARITY = 0.98;
const MOVE_KEYWORDS = new Set(
  "as break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while async await".split(" ")
);
// 机械重排噪声：clippy 的 collapsible_if 把嵌套 if 收成 let-chain（edition 2024 触发的批量改写）时，
// 语句块的额外花括号与 && 会改变 token 多重集，让纯搬运看起来像新逻辑。这些 token 不携带逻辑差异。
const MOVE_NOISE = new Set(["{", "}", ";", ",", "&"]);

/** 把注释、字符串、字符字面量抹成空格：保留换行与偏移，花括号配平与分词才不会被内容带偏。 */
function blankSource(src) {
  const out = src.split("");
  const n = src.length;
  const blank = (from, to) => {
    for (let k = from; k < to && k < n; k++) if (out[k] !== "\n") out[k] = " ";
  };
  let i = 0;
  while (i < n) {
    const c = src[i];
    if (c === "/" && src[i + 1] === "/") {
      let j = i;
      while (j < n && src[j] !== "\n") j++;
      blank(i, j);
      i = j;
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      let j = i + 2;
      while (j < n && !(src[j] === "*" && src[j + 1] === "/")) j++;
      j = Math.min(n, j + 2);
      blank(i, j);
      i = j;
      continue;
    }
    if (c === "r" && !/[A-Za-z0-9_]/.test(src[i - 1] || "") && /^r#*"/.test(src.slice(i, i + 8))) {
      const hashes = src.slice(i + 1).match(/^#*/)[0];
      const close = '"' + hashes;
      const found = src.indexOf(close, i + 2 + hashes.length);
      const j = found < 0 ? n : found + close.length;
      blank(i, j);
      i = j;
      continue;
    }
    if (c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') {
        if (src[j] === "\\") j++;
        j++;
      }
      j = Math.min(n, j + 1);
      blank(i, j);
      i = j;
      continue;
    }
    if (c === "'") {
      const m = src.slice(i).match(/^'(\\.|[^'\\\n])'/);
      if (m) {
        blank(i, i + m[0].length);
        i += m[0].length;
        continue;
      }
    }
    i++;
  }
  return out.join("");
}

/** 归一化分词：丢掉路径限定（a::b::f 只剩 f）、Rust 关键字与 return，只留结构。 */
function normTokens(text) {
  const stripped = text.replace(/(?:[A-Za-z_][A-Za-z0-9_]*)::/g, "");
  const words = stripped.match(/[A-Za-z_][A-Za-z0-9_]*|\d+(?:\.\d+)?|[^\sA-Za-z0-9_]/g) || [];
  return words.filter((w) => !MOVE_KEYWORDS.has(w));
}

function bagOf(tokens) {
  const bag = new Map();
  for (const t of tokens) bag.set(t, (bag.get(t) || 0) + 1);
  return bag;
}

/** Dice 系数：两份 token 多重集的重合度，0..1。 */
function dice(a, b) {
  let inter = 0;
  let totalA = 0;
  let totalB = 0;
  for (const [k, v] of a) {
    totalA += v;
    inter += Math.min(v, b.get(k) || 0);
  }
  for (const v of b.values()) totalB += v;
  const all = totalA + totalB;
  return all ? (2 * inter) / all : 1;
}

/** 抽出所有带体的函数：名字、起止行号、归一化 token。 */
function extractFunctions(blanked) {
  const lines = blanked.split(/\r?\n/);
  const starts = [];
  let off = 0;
  for (const l of lines) {
    starts.push(off);
    off += l.length + 1;
  }
  const lineOf = (at) => {
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (starts[mid] <= at) lo = mid;
      else hi = mid - 1;
    }
    return lo + 1;
  };
  const out = [];
  const re = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  let m;
  while ((m = re.exec(blanked))) {
    let i = re.lastIndex;
    let depth = 0;
    let open = -1;
    let bodyless = false;
    while (i < blanked.length) {
      const c = blanked[i];
      if (c === "(") depth++;
      else if (c === ")") depth--;
      else if (c === "{" && depth === 0) {
        open = i;
        break;
      } else if (c === ";" && depth === 0) {
        bodyless = true;
        break;
      }
      i++;
    }
    if (bodyless || open < 0) continue;
    let d = 0;
    let end = -1;
    for (let j = open; j < blanked.length; j++) {
      if (blanked[j] === "{") d++;
      else if (blanked[j] === "}") {
        d--;
        if (d === 0) {
          end = j;
          break;
        }
      }
    }
    if (end < 0) continue;
    out.push({
      name: m[1],
      first: lineOf(m.index),
      last: lineOf(end),
      bag: bagOf(normTokens(blanked.slice(open, end + 1)).filter((t) => !MOVE_NOISE.has(t))),
    });
    re.lastIndex = end;
  }
  return out;
}

/** 行键：归一化后拼接；太短的行（只有一个花括号、else 之类）不参与兜底，免得满屏误判。 */
function lineKey(text) {
  const key = normTokens(text).join("");
  return key.length >= MOVE_KEY_MIN ? key : null;
}

/** 本次 PR 触及的 Rust 文件与它们在基线的旧路径（重命名与修改给旧路径，新增没有）。 */
function changedPairs(base) {
  const r = spawnSync("git", ["diff", "--name-status", "-M", base + "...HEAD", "--", "*.rs"], {
    cwd: ROOT, encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
  });
  if (r.status !== 0) throw new Error("git diff --name-status " + base + "...HEAD 失败：\n" + (r.stderr || r.stdout));
  const pairs = [];
  for (const line of r.stdout.split(/\r?\n/)) {
    if (!line.trim()) continue;
    const parts = line.split("\t");
    const st = parts[0][0];
    if (st === "R" || st === "C") pairs.push({ old: parts[1], new: parts[2] });
    else if (st === "M" || st === "T") pairs.push({ old: parts[1], new: parts[1] });
    else if (st === "A") pairs.push({ old: null, new: parts[1] });
  }
  return pairs;
}

function baseSource(base, file) {
  const r = spawnSync("git", ["show", base + ":" + file], { cwd: ROOT, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  return r.status === 0 ? r.stdout : null;
}

/** 变更行里哪些是纯搬运：Map<文件, Set<行号>>。 */
function movedLines(base, wanted, root = ROOT) {
  const pairs = changedPairs(base).filter((p) => p.new && wanted.has(p.new));
  const baseByName = new Map();
  const keysLeft = new Map();
  for (const p of pairs) {
    const src = p.old ? baseSource(base, p.old) : null;
    if (src == null) continue;
    const blanked = blankSource(src);
    for (const f of extractFunctions(blanked)) {
      if (!baseByName.has(f.name)) baseByName.set(f.name, []);
      baseByName.get(f.name).push(f);
    }
    for (const line of blanked.split(/\r?\n/)) {
      const key = lineKey(line);
      if (key) keysLeft.set(key, (keysLeft.get(key) || 0) + 1);
    }
  }
  const moved = new Map();
  const mark = (file, from, to) => {
    if (!moved.has(file)) moved.set(file, new Set());
    const set = moved.get(file);
    for (let l = from; l <= to; l++) set.add(l);
  };
  for (const p of pairs) {
    let src;
    try {
      src = fs.readFileSync(path.join(root, p.new), "utf8");
    } catch {
      continue;
    }
    const blanked = blankSource(src);
    for (const f of extractFunctions(blanked)) {
      let best = 0;
      for (const c of baseByName.get(f.name) || []) {
        best = Math.max(best, dice(f.bag, c.bag));
        if (best >= 1) break;
      }
      if (best >= MOVE_SIMILARITY) mark(p.new, f.first, f.last);
    }
    const lines = blanked.split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const key = lineKey(lines[i]);
      if (!key) continue;
      const left = keysLeft.get(key) || 0;
      if (!left) continue;
      keysLeft.set(key, left - 1);
      mark(p.new, i + 1, i + 1);
    }
  }
  return moved;
}

/**
 * 变更行：git diff 的新侧行号，只取新增行（删除行没有可执行面）。 */
function changedLines(base) {
  const r = spawnSync("git", ["diff", "--unified=0", "--no-color", base + "...HEAD", "--", "*.rs"], {
    cwd: ROOT, encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
  });
  if (r.status !== 0) throw new Error("git diff " + base + "...HEAD 失败：\n" + (r.stderr || r.stdout));
  const map = new Map();
  let file = null;
  let next = 0;
  for (const line of r.stdout.split(/\r?\n/)) {
    if (line.startsWith("+++ b/")) { file = line.slice(6); continue; }
    if (line.startsWith("+++ /dev/null")) { file = null; continue; }
    const h = line.match(/^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if (h) { next = Number(h[1]); continue; }
    if (!file) continue;
    if (line.startsWith("+")) {
      if (!map.has(file)) map.set(file, new Set());
      map.get(file).add(next);
      next++;
    } else if (line.startsWith(" ")) {
      next++;
    }
  }
  return map;
}

/** 变更行覆盖：只看新增的 Rust 行；测试代码、seam 与无执行面的行不参与。 */
export function diffCoverage(files, base, root = ROOT) {
  const changed = changedLines(base);
  const wanted = new Set([...changed.keys()].filter((f) => !isTestFile(f)));
  const moved = movedLines(base, wanted, root);
  const exclusions = exclusionMap();
  const buckets = POLICY.buckets.map((b) => ({ ...b, re: b.globs.map(globToRe), covered: 0, total: 0, moved: 0 }));
  for (const f of files) {
    const rel = f.file;
    if (isTestFile(rel)) continue;
    const want = changed.get(rel);
    if (!want || !want.size) continue;
    const skip = exclusions.get(rel);
    const here = moved.get(rel);
    let mask = null;
    try {
      mask = testLineMask(fs.readFileSync(path.join(root, rel), "utf8"));
    } catch {
      mask = [];
    }
    for (const l of f.lines) {
      if (!want.has(l.line) || mask[l.line - 1] || (skip && skip.has(l.line))) continue;
      for (const b of buckets) {
        if (!b.re.some((re) => re.test(rel))) continue;
        if (here && here.has(l.line)) {
          b.moved++;
          continue;
        }
        b.total++;
        if (l.count > 0) b.covered++;
      }
    }
  }
  for (const b of buckets) b.pct = b.total ? (100 * b.covered) / b.total : 100;
  return { base, min: POLICY.changeLineMin || 90, moved: buckets.reduce((n, b) => n + b.moved, 0), buckets };
}

function printDiff(diff) {
  console.log("[变更行] base=" + diff.base + "（新增 Rust 行；无执行面的行不计）");
  if (!diff.buckets.some((b) => b.total)) {
    console.log("  SKIP 没有可判定的 Rust 变更行");
    return;
  }
  for (const b of diff.buckets) {
    const min = b.min === 100 ? 100 : diff.min;
    const ok = b.pct >= min;
    const moved = b.moved ? "；纯搬运 " + b.moved + " 行不计" : "";
    console.log(
      "  " + (ok ? "PASS" : "FAIL") + " " + b.id.padEnd(10) +
      " " + b.pct.toFixed(2).padStart(6) + "% (min " + min + "%) " +
      b.covered + "/" + b.total + moved
    );
  }
  if (diff.moved) console.log("  其中 " + diff.moved + " 行是纯搬运（换文件、换路径限定或重排），未计入分母");
}

function collect() {
  fs.mkdirSync(OUT_DIR, { recursive: true });
  const env = { ...process.env };
  const shown = spawnSync("cargo", ["llvm-cov", "show-env", "--sh"], { cwd: ROOT, encoding: "utf8" });
  if (shown.status !== 0) throw new Error("cargo llvm-cov show-env 失败：\n" + (shown.stderr || shown.stdout));
  for (const line of shown.stdout.split(/\r?\n/)) {
    const m = line.match(/^export\s+([A-Za-z_][A-Za-z0-9_]*)=(.*)$/);
    if (!m) continue;
    let v = m[2];
    if ((v.startsWith("'") && v.endsWith("'")) || (v.startsWith('"') && v.endsWith('"'))) v = v.slice(1, -1);
    env[m[1]] = v;
  }
  const targetDir = env.CARGO_LLVM_COV_TARGET_DIR || path.join(ROOT, "target");
  // show-env 的变量只该在「外面再包一层 cargo llvm-cov」时用；直接交给 cargo llvm-cov 自己会
  // 让 RUSTC_WRAPPER 递归进 cargo-llvm-cov（rustc -vV 失败）。这里统一清掉，只留目录与 profile 路径。
  const covEnv = { ...env };
  delete covEnv.RUSTC_WRAPPER;
  delete covEnv.CARGO_LLVM_COV_SHOW_ENV;
  for (const k of Object.keys(covEnv)) if (k.startsWith("__CARGO_LLVM_COV_")) delete covEnv[k];
  covEnv.CARGO_TARGET_DIR = targetDir;
  covEnv.LLVM_PROFILE_FILE = path.join(targetDir, "solomni-%p-%16m.profraw");

  console.log("[覆盖率] 1/4 跑测试并收集进程内 profile（测试失败不中断，最后统一判）");
  const test = spawnSync("cargo", ["llvm-cov", "--workspace", "--all-targets", "--no-report", "--ignore-run-fail"], {
    cwd: ROOT, env: covEnv, encoding: "utf8", maxBuffer: 64 * 1024 * 1024,
  });
  const testOut = (test.stdout || "") + (test.stderr || "");
  const testFailed = test.status !== 0 || /test result: FAILED/.test(testOut);
  if (testFailed) console.log(testOut.trim().split(/\r?\n/).slice(-25).join("\n"));

  console.log("[覆盖率] 2/4 构建插桩产品二进制");
  const bin = path.join(targetDir, "debug", process.platform === "win32" ? "solomni.exe" : "solomni");
  // build 不能沿用 cargo-llvm-cov 注入的 RUSTC_WRAPPER：show-env 那套只给 cargo llvm-cov 自己的
  // 子进程用，直接交给 cargo build 会被当成 rustc 再进一次 cargo-llvm-cov（rustc -vV 失败）。
  // 插桩参数本身写成 RUSTFLAGS 即可，产物与测试进程用同一份 profile 格式。
  const buildEnv = { ...covEnv, RUSTFLAGS: "-C instrument-coverage --cfg=coverage" };
  const build = spawnSync("cargo", ["build", "--bin", "solomni"], { cwd: ROOT, env: buildEnv, encoding: "utf8" });
  if (build.status !== 0) throw new Error("插桩构建失败：\n" + ((build.stdout || "") + (build.stderr || "")).slice(-4000));

  console.log("[覆盖率] 3/4 跑 L4 端到端（产品子进程同一份插桩）");
  const e2e = spawnSync(process.execPath, [path.join(ROOT, "tests", "cross-platform", "e2e", "orchestrator.js")], {
    cwd: ROOT,
    env: { ...covEnv, E2E_COVERAGE: "1", SOLOMNI_BIN: bin, LLVM_PROFILE_FILE: path.join(targetDir, "e2e-%p-%16m.profraw") },
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    timeout: 30 * 60 * 1000,
  });
  const e2eOut = (e2e.stdout || "") + (e2e.stderr || "");
  const e2eFailed = e2e.status !== 0 || !/E2E-OK/.test(e2eOut);
  if (e2eFailed) console.log(e2eOut.trim().split(/\r?\n/).slice(-40).join("\n"));

  console.log("[覆盖率] 4/4 合并并出报告");
  // cargo-llvm-cov 会缓存 .profdata；删掉它才会把 e2e 的 profraw 一起合进来。
  for (const d of [targetDir, path.join(targetDir, "llvm-cov-target")]) {
    try {
      for (const n of fs.readdirSync(d)) if (n.endsWith(".profdata")) fs.rmSync(path.join(d, n), { force: true });
    } catch {}
  }
  const txt = path.join(OUT_DIR, "coverage.txt");
  const report = spawnSync("cargo", ["llvm-cov", "report", "--text", "--output-path", txt], { cwd: ROOT, env: covEnv, encoding: "utf8" });
  if (report.status !== 0) throw new Error("生成覆盖率报告失败：\n" + ((report.stdout || "") + (report.stderr || "")).slice(-4000));

  const files = parseReport(fs.readFileSync(txt, "utf8"));
  const result = evaluate(files);
  const base = arg("--diff-base") || process.env.COVERAGE_DIFF_BASE || null;
  if (base && base !== true) result.diff = diffCoverage(files, String(base));
  result.gates = { tests: testFailed ? "fail" : "pass", e2e: e2eFailed ? "fail" : "pass" };
  fs.writeFileSync(path.join(OUT_DIR, "summary.json"), JSON.stringify(result, null, 2));
  print(result);
  if (result.diff) printDiff(result.diff);
  const thresholdBad = result.buckets.some((b) => b.pct < b.min);
  const diffBad = !!(result.diff && result.diff.buckets.some((b) => b.total && b.pct < (b.min === 100 ? 100 : result.diff.min)));
  if (thresholdBad || diffBad || testFailed || e2eFailed) {
    console.log("[覆盖率] 失败：阈值=" + (thresholdBad ? "未达标" : "达标") + "，变更行=" + (diffBad ? "未达标" : "达标") + "，测试=" + result.gates.tests + "，e2e=" + result.gates.e2e);
    return 1;
  }
  console.log("[覆盖率] 通过。报告：target/coverage/coverage.txt；摘要：target/coverage/summary.json");
  return 0;
}

function main() {
  const reportPath = arg("--report");
  if (reportPath) {
    const files = parseReport(fs.readFileSync(String(reportPath), "utf8"));
    const result = evaluate(files);
    const base = arg("--diff-base");
    if (base && base !== true) result.diff = diffCoverage(files, String(base));
    print(result);
    if (result.diff) printDiff(result.diff);
    if (arg("--json")) fs.writeFileSync(String(arg("--json")), JSON.stringify(result, null, 2));
    const bad = result.buckets.some((b) => b.pct < b.min) ||
      !!(result.diff && result.diff.buckets.some((b) => b.total && b.pct < (b.min === 100 ? 100 : result.diff.min)));
    return arg("--check") && bad ? 1 : 0;
  }
  return collect();
}

process.exitCode = main();
