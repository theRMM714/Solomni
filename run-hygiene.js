#!/usr/bin/env node
/**
 * 仓库卫生审查——**不是门禁**：门禁（node run-tests.js 的 T0 结构审查）回答"这次改动有没有把代码弄坏"，
 * 卫生回答"仓库里还欠多少、挂在谁身上"。存量是历史累积，必然存在，所以它只报，不拦；唯一会写文件的入口是 --tighten。
 *
 * 三类：
 * - 格式存量（注释契约，ARCHITECTURE.md 十）：按 文件 × 规则 **计数**，与 tests/comment-baseline.json 比。
 *   棘轮只减不增：新文件零容忍（快照里没有 = 从 0 起算），老文件同类变多也算新增；变少要求销账。
 *   --tighten 只许往下收束：出现新增就拒绝写盘（收紧后连同改动一起提交）。
 * - 内容卫生（只报）：悬挂的缺口 id（已删 id 从 git 历史取）、文档里指向不存在的 src 路径、引用的根文档不存在。
 * - 语义卫生（只报）：全仓 Markdown 正文与源码 / 配置 / 脚本注释里的历史或待办措辞；不建基线、不进 T0，
 *   只把可疑行列出来让人判断。`hygiene-ignore` 可显式跳过确认过的行。
 *
 * 用法：node run-hygiene.js [--by-file] [--dir <模式>]… [--tighten] [--strict]
 *                      [--semantic] [--semantic-only] [--strict-semantic]
 *   出口码：默认 0；--strict 有契约 / 内容发现即 1；--strict-semantic 有**高置信**语义发现即 1。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const ROOT = __dirname;
const BASELINE = path.join(ROOT, "tests", "comment-baseline.json");
const HEADER_SLOTS = ["目的", "管", "不管", "联动"];
const ITEM_SLOTS = ["目的", "参数", "返回", "错误", "约束"];
/* 词表只收**叙事性**词（讲来历、显式待办）。领域词不进：`临时 agent`、`后续片段`、`本地遗留`、
 * `暂未暴露` 都是描述当前状态的说法，收进来只会制造误报。 */
const COMMENT_FORBIDDEN = ["曾经", "原来", "旧版", "旧实现", "改成", "先是", "后来", "以前", "与旧逻辑", "TODO", "FIXME", "待补"];
/* 语义卫生（只报）的词表：**与上面的门禁词表分开**。上面的词表只作用于 Rust `//` 行内注释，
 * 改动它会动 tests/comment-baseline.json；这里扩的是全仓报告，不建基线、不进 T0。
 * 强词 = 大概率是旧实现 / 旧格式叙述；弱词 = 也可能描述当前操作或当前效果，必须人看。hygiene-ignore */
const STRONG_WORDS = ["曾经", "旧版", "旧实现", "旧格式", "旧会话", "旧字段", "旧形态", "旧上限", "旧输出", "旧名字", "旧记录", "旧逻辑", "迁移期", "承接原", "已不再"];
const WEAK_WORDS = ["旧资产", "原来", "改成", "先是", "后来", "以前", "不再有", "TODO", "FIXME", "待补"];
const SEMANTIC_EXTS = new Set(["md", "rs", "js", "mjs", "cjs", "yaml", "yml", "toml", "sh", "py", "bat", "cmd", "cpp", "css", "html"]);
const LEDGERS = ["tests/gaps.yaml", "tests/cross-platform/gaps.yaml", "tests/windows/gaps.yaml", "tests/linux/gaps.yaml", "tests/macos/gaps.yaml"];

const RULES = {
  headerMissing: "头块：缺文件头",
  headerOrder: "头块：槽不齐或顺序不对",
  headerStray: "头块：槽外有行或自造槽",
  itemFirst: "条目：首行不是目的",
  itemStray: "条目：槽外有行或自造槽",
  inlineBlank: "行内：注释串后面必须是代码（不能是空行）",
  inlineRun: "行内：连续超过 2 行",
  inlineLong: "行内：超过 150 字",
  inlineWord: "行内：历史或待办措辞",
  inlineBlock: "行内：块注释",
};

function rsFiles(dir, out) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (!["target", ".git", "node_modules"].includes(e.name)) rsFiles(p, out); }
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
}

/** 注释契约（ARCHITECTURE.md 十）：返回 { findings: [{file, line, rule}], byFileRule: {file: [rule]} }。
 *  门禁与卫生工具共用这一份判定——判定只有一处。 */
/** 目的：按行扫 Rust 源码的「字面量 / 注释」状态，供注释契约判定用。
 *  管：真块注释出现在哪一行、哪些行是真正的行注释（行首 // 且不在多行字面量里）。
 *  不管：语法正确性——识别不出的极限情况按代码对待（宁可漏报，不误报）。
 */
function scanRustLines(text) {
  const lines = text.split(/\r?\n/);
  const blockAt = [];
  const isComment = new Array(lines.length).fill(false);
  const comments = [];
  const blockTexts = [];
  let mode = "code";
  let hashes = 0;
  let blockStart = 0;
  let blockBuf = "";
  for (let n = 0; n < lines.length; n++) {
    const s = lines[n];
    if (mode === "code" && /^\s*\/\//.test(s)) isComment[n] = true;
    let i = 0;
    while (i < s.length) {
      if (mode === "block") {
        const close = s.indexOf("*/", i);
        if (close >= 0) {
          blockBuf += s.slice(i, close);
          blockTexts.push({ line: blockStart, text: blockBuf });
          mode = "code";
          i = close + 2;
          blockBuf = "";
        } else {
          blockBuf += s.slice(i) + "\n";
          i = s.length;
        }
        continue;
      }
      if (mode === "str" || mode === "char") {
        const q = mode === "str" ? '"' : "'";
        if (s[i] === "\\") { i += 2; continue; }
        if (s[i] === q) { mode = "code"; i++; continue; }
        i++;
        continue;
      }
      if (mode === "raw") {
        if (s[i] === '"' && s.slice(i + 1, i + 1 + hashes) === "#".repeat(hashes)) { mode = "code"; i += 1 + hashes; continue; }
        i++;
        continue;
      }
      if (s.startsWith("//", i)) {
        comments.push({ line: n + 1, text: s.slice(i) });
        break;
      }
      if (s.startsWith("/*", i)) {
        blockAt.push(n + 1);
        blockStart = n + 1;
        blockBuf = "";
        mode = "block";
        i += 2;
        continue;
      }
      if (s[i] === '"') { mode = "str"; i++; continue; }
      if (s[i] === "r") {
        let j = i + 1;
        let h = 0;
        while (s[j] === "#") { h++; j++; }
        if (s[j] === '"') { mode = "raw"; hashes = h; i = j + 1; continue; }
        i++;
        continue;
      }
      if (s[i] === "b" && (s[i + 1] === '"' || s[i + 1] === "r" || s[i + 1] === "'")) { i++; continue; }
      if (s[i] === "'") {
        const m = /^'(?:\\.|[^\\'])'/.exec(s.slice(i));
        if (m) { i += m[0].length; continue; }
        i++;
        continue;
      }
      i++;
    }
  }
  if (mode === "block") blockTexts.push({ line: blockStart, text: blockBuf });
  return { blockAt: blockAt, isComment: isComment, comments: comments, blockTexts: blockTexts };
}

function scanCommentContract() {
  const findings = [];
  const files = rsFiles(path.join(ROOT, "src"), []).concat(rsFiles(path.join(ROOT, "tests"), []));
  for (const f of files) {
    const rel = path.relative(ROOT, f).replace(/\\/g, "/");
    const text = fs.readFileSync(f, "utf8");
    const lines = text.split(/\r?\n/);
    const at = (n, rule) => findings.push({ file: rel, line: n, rule });
    // ① 文件头
    let i = 0;
    while (i < lines.length && (lines[i].trim() === "" || /^\s*#!\[/.test(lines[i]))) i++;
    const slots = [];
    while (i < lines.length && lines[i].startsWith("//!")) {
      const raw = lines[i].slice(3);
      const body = raw.trim();
      const hit = HEADER_SLOTS.find((s) => body.startsWith(s + "："));
      if (hit) { slots.push(hit); i++; continue; }
      if (/^ {2,}/.test(raw) && slots.length) { i++; continue; }
      at(i + 1, RULES.headerStray);
      i++;
    }
    if (!slots.length) at(1, RULES.headerMissing);
    else if (slots.join("|") !== HEADER_SLOTS.join("|")) at(1, RULES.headerOrder);
    const lex = scanRustLines(text);
    for (const ln of lex.blockAt) at(ln, RULES.inlineBlock);
    // ② 附着在 pub 项上的文档块
    let d = 0;
    while (d < lines.length) {
      if (!lines[d].trim().startsWith("///")) { d++; continue; }
      const start = d;
      while (d < lines.length && lines[d].trim().startsWith("///")) d++;
      const doc = lines.slice(start, d);
      let m = d;
      while (m < lines.length && (lines[m].trim().startsWith("#[") || lines[m].trim() === "")) m++;
      if (!/^pub(\s|\()/.test((lines[m] || "").trim())) continue;
      const seen = [];
      doc.forEach((rawDoc, n) => {
        const body = rawDoc.trim().slice(3);
        const t = body.trim();
        const hit = ITEM_SLOTS.find((s) => t.startsWith(s + "："));
        if (hit) { seen.push(hit); return; }
        if (/^ {2,}/.test(body) && seen.length) return;
        at(start + n + 1, RULES.itemStray);
      });
      if (seen[0] !== "目的") at(start + 1, RULES.itemFirst);
    }
    // ③ 行内注释：只认真正的行注释行（字面量 / 块注释里的不算）；违规报**注释串首行**。
    let run = 0;
    let runStart = -1;
    const closeRun = (followedByBlank) => {
      if (run > 2) at(runStart + 1, RULES.inlineRun);
      if (followedByBlank) at(runStart + 1, RULES.inlineBlank);
      run = 0;
      runStart = -1;
    };
    lines.forEach((rawLine, n) => {
      const t = rawLine.trim();
      if (lex.isComment[n] && t.startsWith("//") && !t.startsWith("///") && !t.startsWith("//!")) {
        const body = t.replace(/^\/\/ ?/, "");
        if (body.length > 150) at(n + 1, RULES.inlineLong);
        if (COMMENT_FORBIDDEN.some((w) => body.includes(w))) at(n + 1, RULES.inlineWord);
        if (!run) runStart = n;
        run++;
        return;
      }
      if (run) closeRun(t === "");
    });
    if (run) closeRun(false);
  }
  return { findings: findings, byFileRule: toByFileRule(findings) };
}

/** 文件扩展名（小写，不含点）；没有扩展名 = 空串。 */
function extOf(file) {
  const m = String(file).match(/\.([A-Za-z0-9]+)$/);
  return m ? m[1].toLowerCase() : "";
}

/** 语义卫生的取文本规则：Markdown 取正文（跳围栏），源码 / 配置 / 脚本取注释。
 *  返回 [{line, text}]；line 从 1 起。 */
function semanticTexts(text, ext) {
  const lines = String(text).split(/\r?\n/);
  if (ext === "md") {
    const out = [];
    let fence = false;
    lines.forEach((line, n) => {
      const t = line.trim();
      if (/^(```|~~~)/.test(t)) { fence = !fence; return; }
      if (!fence) out.push({ line: n + 1, text: line });
    });
    return out;
  }
  if (ext === "rs") {
    const lex = scanRustLines(text);
    return lex.comments.concat(lex.blockTexts).sort((a, b) => a.line - b.line);
  }
  if (["js", "mjs", "cjs", "cpp", "css"].includes(ext)) return jsCommentTexts(text);
  if (["yaml", "yml", "toml", "sh", "py"].includes(ext)) {
    const out = [];
    lines.forEach((line, n) => {
      const t = line.trim();
      if (t.startsWith("#")) out.push({ line: n + 1, text: t });
    });
    return out;
  }
  if (["bat", "cmd"].includes(ext)) {
    const out = [];
    lines.forEach((line, n) => {
      const t = line.trim();
      if (/^(rem\b|::)/i.test(t)) out.push({ line: n + 1, text: t });
    });
    return out;
  }
  if (ext === "html") {
    const out = [];
    let inComment = false;
    lines.forEach((line, n) => {
      const t = line.trim();
      if (t.includes("<!--")) inComment = true;
      if (inComment) out.push({ line: n + 1, text: line });
      if (t.includes("-->")) inComment = false;
    });
    return out;
  }
  return [];
}

/** JS / TS / C++ / CSS 的注释文本：字符串（含模板串）里的 // 不算注释；块注释跨行累积。 */
function jsCommentTexts(text) {
  const lines = String(text).split(/\r?\n/);
  const out = [];
  let mode = "code";
  let quote = "";
  let blockLine = 0;
  let blockBuf = "";
  for (let n = 0; n < lines.length; n++) {
    const s = lines[n];
    let i = 0;
    while (i < s.length) {
      if (mode === "block") {
        const close = s.indexOf("*/", i);
        if (close >= 0) {
          blockBuf += s.slice(i, close);
          out.push({ line: blockLine, text: blockBuf });
          mode = "code";
          i = close + 2;
          blockBuf = "";
        } else {
          blockBuf += s.slice(i) + "\n";
          i = s.length;
        }
        continue;
      }
      if (mode === "str") {
        if (s[i] === "\\") { i += 2; continue; }
        if (s[i] === quote) { mode = "code"; i++; continue; }
        i++;
        continue;
      }
      if (s.startsWith("//", i)) { out.push({ line: n + 1, text: s.slice(i) }); break; }
      if (s.startsWith("/*", i)) { blockLine = n + 1; blockBuf = ""; mode = "block"; i += 2; continue; }
      if (s[i] === '"' || s[i] === "'" || s[i] === "`") { quote = s[i]; mode = "str"; i++; continue; }
      i++;
    }
  }
  if (mode === "block") out.push({ line: blockLine, text: blockBuf });
  return out;
}

/** 一行文本里的语义词：强词 / 弱词 / 批次 20x。返回 null = 没有可疑词。 */
function semanticLineHits(text) {
  const flat = String(text).replace(/\s+/g, " ");
  const strong = STRONG_WORDS.filter((w) => flat.includes(w));
  const batches = flat.match(/批次\s*20[ab]/g) || [];
  const weak = WEAK_WORDS.filter((w) => flat.includes(w));
  const words = [...new Set([...strong, ...batches, ...weak])];
  if (!words.length) return null;
  return { level: strong.length || batches.length ? "strong" : "weak", words: words };
}

/** 一段文本的语义发现：同一行多个词合成一条；hygiene-ignore 的整行 / 整块跳过。 */
function semanticHits(text, ext) {
  const byLine = new Map();
  let ignored = 0;
  for (const item of semanticTexts(text, ext)) {
    if (item.text.includes("hygiene-ignore")) { ignored++; continue; }
    const hit = semanticLineHits(item.text);
    if (!hit) continue;
    const prev = byLine.get(item.line);
    if (prev) {
      prev.words = [...new Set([...prev.words, ...hit.words])];
      if (hit.level === "strong") prev.level = "strong";
      prev.text += " " + item.text;
    } else {
      byLine.set(item.line, {
        line: item.line,
        level: hit.level,
        words: hit.words,
        text: String(item.text).replace(/\s+/g, " ").trim(),
      });
    }
  }
  return { findings: [...byLine.values()].sort((a, b) => a.line - b.line), ignored: ignored };
}

const SEMANTIC_SKIP_DIRS = new Set(["target", ".git", "node_modules", "platform", ".tools", ".home", "session", "work", "dist", "userdata"]);

/** 受控文件清单；取不到 git 时返回 null（调用方降级到目录遍历）。 */
function trackedTextFiles() {
  try {
    return git(["ls-files"]).split(/\r?\n/).filter((f) => f && SEMANTIC_EXTS.has(extOf(f)));
  } catch (e) {
    return null;
  }
}

/** git 不可用时的降级：从仓库根遍历文本文件，跳过产物、用户数据与依赖目录。 */
function fallbackTextFiles() {
  const out = [];
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) {
        if (!SEMANTIC_SKIP_DIRS.has(e.name)) walk(p);
        continue;
      }
      if (SEMANTIC_EXTS.has(extOf(e.name))) out.push(path.relative(ROOT, p).replace(/\\/g, "/"));
    }
  };
  walk(ROOT);
  return out;
}

/** 语义卫生（只报）：全仓 Markdown 正文 + 源码 / 配置 / 脚本注释里的历史或待办措辞。
 *  不建基线、不进 T0；hygiene-ignore 是显式的「已确认」出口。 */
function semanticFindings() {
  let files = trackedTextFiles();
  let degraded = null;
  if (files === null) {
    files = fallbackTextFiles();
    degraded = "取不到 git 文件清单，只扫源码树（target / .git / 用户数据已跳过）";
  }
  const findings = [];
  let ignored = 0;
  let unreadable = 0;
  for (const rel of files) {
    let text;
    try {
      text = fs.readFileSync(path.join(ROOT, rel), "utf8");
    } catch (e) {
      unreadable++;
      continue;
    }
    const res = semanticHits(text, extOf(rel));
    ignored += res.ignored;
    for (const f of res.findings) findings.push({ file: rel, ...f });
  }
  findings.sort((a, b) => {
    if (a.level !== b.level) return a.level === "strong" ? -1 : 1;
    if (a.file !== b.file) return a.file < b.file ? -1 : 1;
    return a.line - b.line;
  });
  return { findings: findings, ignored: ignored, unreadable: unreadable, degraded: degraded };
}

/** 语义卫生的可执行自测（纯文本，不读盘）：围栏 / 字符串 / 注释 / 抑制 / 去重 / 分级。 */
function semanticSelfTest() {
  const bad = [];
  const eq = (name, got, want) => {
    if (JSON.stringify(got) !== JSON.stringify(want)) bad.push(name + "：期望 " + JSON.stringify(want) + "，实际 " + JSON.stringify(got));
  };
  const hits = (text, ext) => semanticHits(text, ext).findings;
  eq("MD 代码围栏跳过", hits("```\n曾经\n```\n", "md").length, 0);
  eq("MD 正文命中", hits("正文曾经", "md")[0] && hits("正文曾经", "md")[0].words, ["曾经"]);
  eq("Rust /// 命中", hits("/// 曾经\n", "rs")[0] && hits("/// 曾经\n", "rs")[0].words, ["曾经"]);
  eq("Rust 字符串不命中", hits('let s = "曾经";', "rs").length, 0);
  eq("Rust 行内命中", hits("let s = 1; // 旧实现", "rs")[0] && hits("let s = 1; // 旧实现", "rs")[0].words, ["旧实现"]);
  eq("Rust 块注释命中", hits("/* 旧版 */", "rs")[0] && hits("/* 旧版 */", "rs")[0].words, ["旧版"]);
  eq("JS 字符串不命中", hits('const u = "http://x/曾经";', "js").length, 0);
  eq("JS 行注释命中", hits("// 旧格式\n", "js")[0] && hits("// 旧格式\n", "js")[0].words, ["旧格式"]);
  eq("YAML 值不命中", hits("desc: 曾经\n", "yaml").length, 0);
  eq("YAML 注释命中", hits("# 旧版\n", "yaml")[0] && hits("# 旧版\n", "yaml")[0].words, ["旧版"]);
  eq("hygiene-ignore 跳过", hits("曾经 hygiene-ignore", "md").length, 0);
  eq("同行去重", hits("曾经 旧版", "md").length, 1);
  eq("弱词分级", hits("改成", "md")[0] && hits("改成", "md")[0].level, "weak");
  eq("批次 20a 识别", hits("批次 20a", "md")[0] && hits("批次 20a", "md")[0].words, ["批次 20a"]);
  return bad;
}

/** 语义小节的文本行；findings 已按 --dir 过滤。 */
function semanticReportLines(sem, findings, all) {
  const out = [];
  const strong = findings.filter((f) => f.level === "strong");
  const weak = findings.filter((f) => f.level === "weak");
  out.push("== 语义卫生（只报，不进 T0、不进棘轮） ==");
  if (sem.degraded) out.push("  [降级] " + sem.degraded);
  const extra = [];
  if (sem.ignored) extra.push("已按 hygiene-ignore 跳过 " + sem.ignored + " 行");
  if (sem.unreadable) extra.push(sem.unreadable + " 个文件读不了");
  out.push("  高置信 " + strong.length + " 处 / 需判断 " + weak.length + " 处" + (extra.length ? "；" + extra.join("；") : ""));
  if (!findings.length) {
    out.push("  没有发现");
    return out;
  }
  const wordCount = (arr) => {
    const m = new Map();
    for (const f of arr) for (const w of f.words) m.set(w, (m.get(w) || 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1)).map(([w, n]) => w + " " + n).join(" | ");
  };
  if (strong.length) out.push("  高置信词：" + wordCount(strong));
  if (weak.length) out.push("  需判断词：" + wordCount(weak));
  out.push("");
  const emit = (arr, cap, label) => {
    for (const f of arr.slice(0, cap)) out.push("  " + f.file + ":" + f.line + "  [" + f.words.join("、") + "]  " + f.text);
    if (arr.length > cap) out.push("  …（" + label + "其余 " + (arr.length - cap) + " 处，--semantic 展开）");
  };
  emit(strong, all ? strong.length : 30, "高置信");
  if (weak.length) emit(weak, all ? weak.length : 5, "需判断");
  out.push("");
  out.push("  提示：这些是「可疑」不是「违规」；确认是当前语义的行加 hygiene-ignore，其余按当前状态改写。");
  return out;
}

function toByFileRule(findings) {
  const by = {};
  for (const f of findings) {
    if (!by[f.file]) by[f.file] = {};
    by[f.file][f.rule] = (by[f.file][f.rule] || 0) + 1;
  }
  return by;
}

/** 棘轮比对（门禁与卫生工具共用这一份判定）：返回 { added, stale }，元素是给人看的字符串（带计数）。
 *  **added** = 现在比快照多——新文件从 0 起算（零容忍），老文件同类变多也不许；门禁硬失败。
 *  **stale** = 现在比快照少——同样硬失败，必须收紧；收紧只能往下（见 --tighten）。 */
function compareContract(now, baseline) {
  const added = [];
  const stale = [];
  for (const f of Object.keys(now)) {
    for (const rule of Object.keys(now[f])) {
      const was = (baseline[f] || {})[rule] || 0;
      const is = now[f][rule];
      if (is > was) added.push(f + " :: " + rule + "（" + was + " → " + is + "）");
    }
  }
  for (const f of Object.keys(baseline)) {
    for (const rule of Object.keys(baseline[f] || {})) {
      const was = baseline[f][rule];
      const is = (now[f] || {})[rule] || 0;
      if (is < was) stale.push(f + " :: " + rule + "（" + was + " → " + is + "）");
    }
  }
  return { added: added, stale: stale };
}

/** 棘轮语义的自测（T0 调用）：把「新文件零容忍 / 只减不增 / 收束只能往下」钉成可执行的判据。 */
function ratchetSelfTest() {
  const bad = [];
  const eq = (name, got, want) => {
    if (JSON.stringify(got) !== JSON.stringify(want)) bad.push(name + "：期望 " + JSON.stringify(want) + "，实际 " + JSON.stringify(got));
  };
  eq("新文件零容忍", compareContract({ "a.rs": { R: 1 } }, {}).added.length, 1);
  eq("老文件同类变多 = 新增", compareContract({ "a.rs": { R: 2 } }, { "a.rs": { R: 1 } }).added.length, 1);
  const flat = compareContract({ "a.rs": { R: 1 } }, { "a.rs": { R: 1 } });
  eq("持平通过", flat.added.length + flat.stale.length, 0);
  eq("变少要销账", compareContract({ "a.rs": { R: 1 } }, { "a.rs": { R: 3 } }).stale.length, 1);
  eq("条目整条消失要销账", compareContract({}, { "a.rs": { R: 1 } }).stale.length, 1);
  return bad;
}

/** 注释词法判定的自测（T0 调用）：字符串与散文里的 /* 不算块注释，真块注释要报对行；
 *  行注释要在字面量之外才算。 */
function commentLexerSelfTest() {
  const bad = [];
  const eq = (name, got, want) => {
    if (JSON.stringify(got) !== JSON.stringify(want)) bad.push(name + "：期望 " + JSON.stringify(want) + "，实际 " + JSON.stringify(got));
  };
  eq("字符串里的 /* 不算块注释", scanRustLines('let s = "/var/lib/apt/lists/*";').blockAt, []);
  eq("散文里的 work/** 不算块注释", scanRustLines("/// 共用顶层那一个 work/**").blockAt, []);
  eq("真块注释要报行号", scanRustLines("let a = 1;\n/* 说明 */\nlet b = 2;").blockAt, [2]);
  eq("raw 串里的 // 不算行注释", scanRustLines('let s = r#"\n// 不是注释\n"#;').isComment.filter(Boolean).length, 0);
  eq("普通 // 是行注释", scanRustLines("let a = 1;\n// 说明\nlet b = 2;").isComment.filter(Boolean).length, 1);
  return bad;
}

const GIT_TMP = path.join(ROOT, "target", "scratch", "hygiene-git.tmp");

/** 跑 git 并把 stdout 重定向到**文件**再读回。
 *  受限会话里管道捕获会被拒（spawnSync EPERM），重定向到文件这条路在普通与受限 shell 里都走得通；
 *  走不通时抛错，由调用方如实降级——不把「没跑」当「没问题」。 */
function git(args) {
  fs.mkdirSync(path.dirname(GIT_TMP), { recursive: true });
  const fd = fs.openSync(GIT_TMP, "w");
  let r;
  try {
    r = spawnSync("git", args, { cwd: ROOT, stdio: ["ignore", fd, "ignore"] });
  } finally {
    fs.closeSync(fd);
  }
  if (r.error || r.status !== 0) throw new Error(r.error ? r.error.message : "git 退出码 " + r.status);
  const text = fs.readFileSync(GIT_TMP, "utf8");
  try {
    fs.unlinkSync(GIT_TMP);
  } catch (e) {
    // 留在 target/scratch 里也无妨（那棵树不入库）
  }
  return text;
}

/** 当前四本缺口账里的 id + 非账本文件里引用的 id。 */
function gapIds() {
  const live = new Set();
  for (const f of LEDGERS) {
    const abs = path.join(ROOT, f);
    if (!fs.existsSync(abs)) continue;
    for (const line of fs.readFileSync(abs, "utf8").split(/\r?\n/)) {
      const m = line.match(/^-\s*id:\s*(\S+)/);
      if (m) live.add(m[1]);
    }
  }
  return live;
}

/** 已删的缺口 id：git 历史里出现在账本上、现在不在了的那些。取不到 git 就返回 null（如实降级）。 */
function deletedGapIds() {
  // 一次 git log -p 拿全历史（输出重定向到文件再读：管道会被拒，文件不会，也不会被截断）。
  let text;
  try {
    text = git(["log", "-p", "--unified=0", "--"].concat(LEDGERS));
  } catch (e) {
    return null;
  }
  const live = gapIds();
  const ever = new Set();
  for (const line of text.split(/\r?\n/)) {
    const m = line.match(/^--\s*id:\s*(\S+)/); // diff 里被删掉的那一行形如：-- id: xxx
    if (m) ever.add(m[1]);
  }
  return [...ever].filter((id) => !live.has(id));
}

/** 内容卫生要读的文件：源码、测试、入口脚本、docs 的 md、根 md。读一遍就好。 */
function candidateFiles() {
  return rsFiles(path.join(ROOT, "src"), [])
    .concat(rsFiles(path.join(ROOT, "tests"), []))
    .concat(["run-tests.js", "run-hygiene.js", "start.js"].map((f) => path.join(ROOT, f)))
    .concat(mdFiles(path.join(ROOT, "docs")))
    .concat(mdFiles(ROOT, true))
    .filter((f) => f && fs.existsSync(f));
}

/** 内容卫生（只报）：悬挂的缺口 id、文档里不存在的 src 路径、引用的根文档不存在。 */
function contentFindings() {
  const out = [];
  const files = candidateFiles().map((f) => ({
    rel: path.relative(ROOT, f).replace(/\\/g, "/"),
    lines: fs.readFileSync(f, "utf8").split(/\r?\n/),
  }));
  const deleted = deletedGapIds();
  if (deleted === null) out.push({ kind: "跳过", text: "取不到 git 历史（受限环境里 git 可能被拦）：已删缺口 id 的悬挂检查没跑——请在普通 shell 里跑一次" });
  else {
    for (const f of files) {
      if (LEDGERS.includes(f.rel)) continue;
      f.lines.forEach((line, n) => {
        for (const id of deleted) {
          if (line.includes(id)) out.push({ kind: "悬挂缺口 id", text: f.rel + ":" + (n + 1) + " 指向已删的 " + id });
        }
      });
    }
  }
  for (const f of files) {
    if (!f.rel.startsWith("docs/") && !/^[^/]+\.md$/.test(f.rel)) continue;
    f.lines.forEach((line, n) => {
      for (const m of line.matchAll(/src\/[A-Za-z0-9_/-]+\.rs/g)) {
        if (!fs.existsSync(path.join(ROOT, m[0]))) out.push({ kind: "悬空代码路径", text: f.rel + ":" + (n + 1) + " 引用的 " + m[0] + " 不存在" });
      }
    });
  }
  return out;
}

function mdFiles(dir, rootOnly) {
  const out = [];
  if (!fs.existsSync(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (!rootOnly && !["target", ".git", "node_modules"].includes(e.name)) out.push(...mdFiles(p, false)); continue; }
    if (!e.name.endsWith(".md")) continue;
    if (rootOnly && path.dirname(p) !== ROOT) continue;
    out.push(p);
  }
  return out;
}

function readBaseline() {
  if (!fs.existsSync(BASELINE)) return null;
  const raw = JSON.parse(fs.readFileSync(BASELINE, "utf8"));
  delete raw._comment;
  return raw;
}

/** `--dir <模式>`：模式里的 `*` 表示一段任意字符（不含分隔符）；匹配文件路径，边界必须是 `/` 或结尾。
 *  不写正则——模式来自命令行，正则转义最容易在这里出错；按片段顺序比对同样准确。 */
function dirMatcher(patterns) {
  return (rel) => {
    if (!patterns.length) return true;
    for (const p of patterns) {
      const pieces = p.split("*");
      let i = 0;
      let ok = true;
      for (let k = 0; k < pieces.length; k++) {
        const piece = pieces[k];
        if (!piece) continue; // 空片段 = 那个 *，交给下一片自己找位置
        const at = k === 0 ? (rel.startsWith(piece) ? 0 : -1) : rel.indexOf(piece, i);
        if (at < 0) { ok = false; break; }
        i = at + piece.length;
      }
      if (!ok) continue;
      if (i === rel.length || rel[i] === "/") return true;
    }
    return false;
  };
}

function main() {
  const argv = process.argv.slice(2);
  const dirs = [];
  for (let i = 0; i < argv.length; i++) if (argv[i] === "--dir" && argv[i + 1]) dirs.push(argv[i + 1]);
  const byFileView = argv.includes("--by-file");
  const semanticAll = argv.includes("--semantic");
  const semanticOnly = argv.includes("--semantic-only");
  const strictSemantic = argv.includes("--strict-semantic");
  const scan = scanCommentContract();
  const inDirs = dirMatcher(dirs);
  const findings = scan.findings.filter((f) => inDirs(f.file));
  const by = toByFileRule(findings);
  const scope = dirs.length ? "（--dir " + dirs.join(" / ") + "）" : "";
  const baselineAll = readBaseline();
  const baseline = baselineAll === null
    ? null
    : Object.fromEntries(Object.entries(baselineAll).filter(([f]) => inDirs(f)));
  const lines = [];

  if (argv.includes("--tighten")) {
    /* 收紧只允许全仓一次做完：带 --dir 收紧会把没过滤到的存量从快照里抹掉，那是静默丢账。 */
    if (dirs.length) {
      console.log("--tighten 不接受 --dir：收紧是全仓一次的动作，带过滤收紧会丢掉没扫到的存量。");
      return 2;
    }
    /* 收紧是棘轮唯一的下调入口：出现任何「新增」就拒绝写盘——否则先违规再收紧等于把违规洗成存量。 */
    if (baselineAll) {
      const { added } = compareContract(by, baselineAll);
      if (added.length) {
        console.log("收紧被拒：棘轮只减不增，这次有 " + added.length + " 处新增——先把它们清掉再收紧。");
        for (const a of added.slice(0, 12)) console.log("  [新增] " + a);
        if (added.length > 12) console.log("  …（其余省略）");
        return 2;
      }
    }
    const out = { _comment: "注释契约（ARCHITECTURE.md 十）的存量快照：键是文件，值是「规则 → 计数」。棘轮只减不增：新文件零容忍，老文件同类变多也算新增；变少必须销账。收紧只能往下：node run-hygiene.js --tighten。" };
    for (const f of Object.keys(by).sort()) out[f] = by[f];
    fs.writeFileSync(BASELINE, JSON.stringify(out, null, 2) + "\n");
    lines.push("已收紧快照 tests/comment-baseline.json：" + Object.keys(by).length + " 个文件、"
      + findings.length + " 处。" + (baselineAll ? "（收紧前 " + Object.keys(baselineAll).length + " 个文件）" : ""));
    console.log(lines.join("\n"));
    return 0;
  }

  if (semanticOnly) {
    const sem = semanticFindings();
    const semFiltered = sem.findings.filter((f) => inDirs(f.file));
    console.log(semanticReportLines(sem, semFiltered, true).join("\n"));
    return strictSemantic && semFiltered.some((f) => f.level === "strong") ? 1 : 0;
  }

  const byRule = new Map();
  for (const f of findings) {
    const cur = byRule.get(f.rule) || { count: 0, files: new Set() };
    cur.count++;
    cur.files.add(f.file);
    byRule.set(f.rule, cur);
  }
  lines.push("== 注释契约存量（ARCHITECTURE.md 十）" + scope + " ==");
  lines.push("合计 " + findings.length + " 处，涉及 " + Object.keys(by).length + " 个文件：");
  for (const [rule, v] of [...byRule.entries()].sort((a, b) => b[1].count - a[1].count)) {
    lines.push("  " + String(v.count).padStart(5) + " 处  " + String(v.files.size).padStart(4) + " 个文件  " + rule);
  }
  if (byFileView) {
    const counts = new Map();
    for (const f of findings) counts.set(f.file, (counts.get(f.file) || 0) + 1);
    lines.push("");
    lines.push("== 逐文件（欠得最多的在前） ==");
    for (const [file, n] of [...counts.entries()].sort((a, b) => b[1] - a[1])) {
      lines.push("  " + String(n).padStart(5) + " 处  " + file + "  :: " + Object.keys(by[file]).join("、"));
    }
  }
  if (baseline === null) {
    lines.push("");
    lines.push("还没有快照：跑 node run-hygiene.js --tighten 建一份（门禁拿它当棘轮基线）。");
  } else {
    const { added, stale } = compareContract(by, baseline);
    lines.push("");
    lines.push("与快照比" + scope + "：新增 " + added.length + " 处（门禁会报）、可销账 " + stale.length + " 处（门禁要求销账）");
    for (const a of added.slice(0, 12)) lines.push("  [新增] " + a);
    for (const s of stale.slice(0, 12)) lines.push("  [可销账] " + s);
    if (added.length + stale.length > 24) lines.push("  …（其余省略）");
    if (!added.length && !stale.length) lines.push("  快照与现状一致（棘轮已对齐）");
    else lines.push("  收紧：node run-hygiene.js --tighten" + (dirs.length ? "（收紧必须全仓，别带 --dir）" : ""));
  }

  const content = contentFindings();
  lines.push("");
  lines.push("== 内容卫生（只报，不进 T0） ==");
  if (!content.length) lines.push("  没有发现");
  for (const c of content) lines.push("  [" + c.kind + "] " + c.text);

  const sem = semanticFindings();
  const semFiltered = sem.findings.filter((f) => inDirs(f.file));
  lines.push("");
  lines.push(...semanticReportLines(sem, semFiltered, semanticAll));

  console.log(lines.join("\n"));
  if (strictSemantic && semFiltered.some((f) => f.level === "strong")) return 1;
  if (argv.includes("--strict") && (findings.length || content.length)) return 1;
  return 0;
}

if (require.main === module) process.exit(main());
module.exports = { scanCommentContract, compareContract, ratchetSelfTest, commentLexerSelfTest, semanticSelfTest, semanticFindings, semanticHits, deletedGapIds, contentFindings, RULES, LEDGERS };
