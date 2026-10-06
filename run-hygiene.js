#!/usr/bin/env node
/**
 * 仓库卫生审查——**不是门禁**：门禁（node run-tests.js 的 T0 结构审查）回答"这次改动有没有把代码弄坏"，
 * 卫生回答"仓库里还欠多少、挂在谁身上"。存量在迁移期必然存在，所以它只报，不拦；唯一会写文件的入口是 --tighten。
 *
 * 两类：
 * - 格式存量（注释契约，ARCHITECTURE.md 十）：按 文件 × 规则 统计，与 tests/comment-baseline.json 比。
 *   门禁只报**新增**与**应销账**；--tighten 把快照收紧（收紧后要连同改动一起提交）。
 * - 内容卫生（只报）：悬挂的缺口 id（已删 id 从 git 历史取）、文档里指向不存在的 src 路径、引用的根文档不存在。
 *
 * 用法：node run-hygiene.js [--tighten] [--strict]   出口码：默认 0；--strict 有发现即 1。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const ROOT = __dirname;
const BASELINE = path.join(ROOT, "tests", "comment-baseline.json");
const HEADER_SLOTS = ["目的", "管", "不管", "联动"];
const ITEM_SLOTS = ["目的", "参数", "返回", "错误", "约束"];
const COMMENT_FORBIDDEN = ["曾经", "原来", "旧版", "旧实现", "改成", "先是", "后来", "遗留", "以前", "与旧逻辑", "TODO", "FIXME", "待补", "临时", "后续", "暂不"];
const LEDGERS = ["tests/gaps.yaml", "tests/cross-platform/gaps.yaml", "tests/windows/gaps.yaml", "tests/linux/gaps.yaml", "tests/macos/gaps.yaml"];

const RULES = {
  headerMissing: "头块：缺文件头",
  headerOrder: "头块：槽不齐或顺序不对",
  headerStray: "头块：槽外有行或自造槽",
  itemFirst: "条目：首行不是目的",
  itemStray: "条目：槽外有行或自造槽",
  inlineBlank: "行内：后面是空行",
  inlineRun: "行内：连续超过 2 行",
  inlineLong: "行内：超过 100 字",
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
    if (text.includes("/*")) at(1, RULES.inlineBlock);
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
    // ③ 行内注释
    let run = 0;
    lines.forEach((rawLine, n) => {
      const t = rawLine.trim();
      if (t.startsWith("//") && !t.startsWith("///") && !t.startsWith("//!")) {
        const body = t.replace(/^\/\/ ?/, "");
        if (body.length > 100) at(n + 1, RULES.inlineLong);
        if (COMMENT_FORBIDDEN.some((w) => body.includes(w))) at(n + 1, RULES.inlineWord);
        run++;
        return;
      }
      if (run) {
        if (run > 2) at(n + 1, RULES.inlineRun);
        if (t === "") at(n + 1, RULES.inlineBlank);
        run = 0;
      }
    });
  }
  return { findings: findings, byFileRule: toByFileRule(findings) };
}

function toByFileRule(findings) {
  const by = {};
  for (const f of findings) {
    if (!by[f.file]) by[f.file] = [];
    if (!by[f.file].includes(f.rule)) by[f.file].push(f.rule);
  }
  for (const k of Object.keys(by)) by[k].sort();
  return by;
}

const GIT_TMP = path.join(ROOT, "target", "hygiene-git.tmp");

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
    // 留在 target/ 里也无妨（那棵树不入库）
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

function main() {
  const argv = process.argv.slice(2);
  const scan = scanCommentContract();
  const by = scan.byFileRule;
  const baseline = readBaseline();
  const lines = [];

  if (argv.includes("--tighten")) {
    const out = { _comment: "注释契约（ARCHITECTURE.md 十）的存量快照：键是文件，值是它当前还欠的规则。门禁只报新增；条目不再成立时必须销账——收紧用 node run-hygiene.js --tighten。" };
    for (const f of Object.keys(by).sort()) out[f] = by[f];
    fs.writeFileSync(BASELINE, JSON.stringify(out, null, 2) + "\n");
    lines.push("已收紧快照 tests/comment-baseline.json：" + Object.keys(by).length + " 个文件、"
      + scan.findings.length + " 处。" + (baseline ? "（收紧前 " + Object.keys(baseline).length + " 个文件）" : ""));
    console.log(lines.join("\n"));
    return 0;
  }

  // 按规则汇总
  const byRule = new Map();
  for (const f of scan.findings) {
    const cur = byRule.get(f.rule) || { count: 0, files: new Set() };
    cur.count++;
    cur.files.add(f.file);
    byRule.set(f.rule, cur);
  }
  lines.push("== 注释契约存量（ARCHITECTURE.md 十） ==");
  lines.push("合计 " + scan.findings.length + " 处，涉及 " + Object.keys(by).length + " 个文件：");
  for (const [rule, v] of [...byRule.entries()].sort((a, b) => b[1].count - a[1].count)) {
    lines.push("  " + String(v.count).padStart(5) + " 处  " + String(v.files.size).padStart(4) + " 个文件  " + rule);
  }
  if (baseline === null) {
    lines.push("");
    lines.push("还没有快照：跑 node run-hygiene.js --tighten 建一份（门禁拿它当棘轮基线）。");
  } else {
    const added = [], stale = [];
    for (const f of Object.keys(by)) {
      for (const rule of by[f]) if (!(baseline[f] || []).includes(rule)) added.push(f + " :: " + rule);
    }
    for (const f of Object.keys(baseline)) {
      for (const rule of baseline[f]) if (!(by[f] || []).includes(rule)) stale.push(f + " :: " + rule);
    }
    lines.push("");
    lines.push("与快照比：新增 " + added.length + " 处（门禁会报）、可销账 " + stale.length + " 处（门禁要求销账）");
    for (const a of added.slice(0, 12)) lines.push("  [新增] " + a);
    for (const s of stale.slice(0, 12)) lines.push("  [可销账] " + s);
    if (added.length + stale.length > 24) lines.push("  …（其余省略）");
    if (!added.length && !stale.length) lines.push("  快照与现状一致（棘轮已对齐）");
    else lines.push("  收紧：node run-hygiene.js --tighten");
  }

  const content = contentFindings();
  lines.push("");
  lines.push("== 内容卫生（只报，不进 T0） ==");
  if (!content.length) lines.push("  没有发现");
  for (const c of content) lines.push("  [" + c.kind + "] " + c.text);

  console.log(lines.join("\n"));
  if (argv.includes("--strict") && (scan.findings.length || content.length)) return 1;
  return 0;
}

if (require.main === module) process.exit(main());
module.exports = { scanCommentContract, deletedGapIds, contentFindings, RULES, LEDGERS };
