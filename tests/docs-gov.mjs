#!/usr/bin/env node
/**
 * 文档模板检查器：类型映射在 docs/templates/index.json，模板在 docs/templates/*.md。
 * 模型：固定骨架 + 重复块；块内只允许列表 / 表格 / 代码 / 引用，每块最多 1 行 ≤150 字引导语。
 * 用法：node tests/docs-gov.mjs [--summary] [--json] [--strict] [--verbose] [--self-test]
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TPL_DIR = path.join(ROOT, "docs", "templates");
const CFG = JSON.parse(fs.readFileSync(path.join(TPL_DIR, "index.json"), "utf8"));
const LEAD_MAX = 150;
const MODAL = /(必须|不得|禁止|只允许|只能|一律|零容忍)/;
const SKIP_DIRS = new Set(["target", ".git", "node_modules", ".tools", ".home", ".vm", "dependencies", "logs", "session", "runtimes", "platform", "demo", "modules", "tests"]);

const esc = (s) => s.replace(/[.*+?^$()|[\]\\]/g, "\\$&");
const stripNum = (s) => s.trim().replace(/^[一二三四五六七八九十百零〇]+[、.．]\s*/, "").replace(/^\d+[、.．]\s*/, "");
const norm = (s) => stripNum(s).replace(/\s+/g, " ").trim();

function globRe(g) {
  let out = esc(g);
  out = out.replace(/\\\*\\\*/g, ".*").replace(/\\\*/g, "[^/]*");
  return new RegExp("^" + out + "$");
}
function headingRe(tplText) {
  const raw = tplText.trim();
  const hasNum = raw.indexOf("{{编号}}") >= 0;
  let out = "", i = 0, m;
  const re = /\{\{([^}]*)\}\}/g;
  while ((m = re.exec(raw))) {
    out += esc(raw.slice(i, m.index));
    out += m[1] === "编号" ? "(?:[一二三四五六七八九十百零〇]+|\\d+)" : "(.+?)";
    i = m.index + m[0].length;
  }
  out += esc(raw.slice(i));
  const prefix = hasNum ? "" : "(?:[一二三四五六七八九十百零〇]+、|\\d+[、.．]\\s*)?";
  return new RegExp("^" + prefix + out + "$");
}
function resolveType(rel) {
  for (const type of Object.keys(CFG.templates)) {
    for (const g of CFG.templates[type].globs) if (globRe(g).test(rel)) return type;
  }
  for (const e of CFG.exceptions) if (globRe(e.path).test(rel)) return "exception";
  return null;
}

const tplCache = {};
function firstTableHeader(body) {
  let fence = false;
  for (const raw of body) {
    const t = raw.trim();
    if (/^```/.test(t)) { fence = !fence; continue; }
    if (fence || !/^\|/.test(t)) continue;
    if (/^\|[\s:|-]+\|$/.test(t)) continue;
    return t.replace(/^\|/, "").replace(/\|$/, "").split("|").map((x) => x.trim());
  }
  return null;
}
function loadTemplate(type) {
  if (tplCache[type]) return tplCache[type];
  const lines = fs.readFileSync(path.join(TPL_DIR, CFG.templates[type].file), "utf8").split("\n");
  const sections = [];
  let cur = null, pending = null, inFence = false;
  for (const raw of lines) {
    const t = raw.trim();
    if (/^```/.test(t)) { inFence = !inFence; if (cur) cur.body.push(raw); continue; }
    const rm = t.match(/^<!--\s*repeat:([\w-]+)\s+min=(\d+)\s+max=(\d+)\s*-->\s*$/);
    if (rm && !inFence) { pending = { id: rm[1], min: Number(rm[2]), max: Number(rm[3]) }; continue; }
    if (/^<!--\s*\/repeat\s*-->\s*$/.test(t) && !inFence) { pending = null; continue; }
    const hm = raw.match(/^(#{1,6})\s+(.+)$/);
    if (hm && !inFence) {
      if (cur) sections.push(cur);
      cur = { level: hm[1].length, text: hm[2].trim(), repeat: pending, body: [] };
      pending = null;
      continue;
    }
    if (cur) cur.body.push(raw);
  }
  if (cur) sections.push(cur);
  for (const s of sections) s.tableHeader = firstTableHeader(s.body);
  const first = sections.find((s) => s.tableHeader);
  tplCache[type] = { file: CFG.templates[type].file, sections, tableHeader: first ? first.tableHeader : null };
  return tplCache[type];
}

function parseDoc(content) {
  const lines = content.split("\n");
  const sections = [], headerLines = [];
  let h1 = null, cur = null, inFence = false;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const t = raw.trim();
    if (!inFence) {
      const hm = raw.match(/^(#{1,6})\s+(.+)$/);
      if (hm) {
        if (hm[1].length === 1 && !h1) { h1 = { text: hm[2].trim(), no: i + 1 }; continue; }
        if (hm[1].length === 2) {
          if (cur) sections.push(cur);
          cur = { heading: hm[2].trim(), no: i + 1, body: [] };
          continue;
        }
        if (!cur) headerLines.push({ line: raw, no: i + 1 }); else cur.body.push({ line: raw, no: i + 1 });
        continue;
      }
    }
    if (/^```/.test(t)) inFence = !inFence;
    const item = { line: raw, no: i + 1 };
    if (cur) cur.body.push(item); else headerLines.push(item);
  }
  if (cur) sections.push(cur);
  return { h1, headerLines, sections };
}

function validateHeader(rel, parsed, problems) {
  if (!parsed.h1) problems.push(rel + "：缺少一级标题");
  const text = parsed.headerLines.map((x) => x.line).join("\n");
  if (!/^>\s*定位[：:]/.test(text)) {
    if (!/^>\s*定位[：:]/m.test(text)) problems.push(rel + "：头部缺少「> 定位：…」");
  }
  const em = text.match(/^>\s*效力[：:]\s*([^\n｜|]+)/m);
  if (!em) { problems.push(rel + "：头部缺少「> 效力：…」"); return; }
  const eff = em[1].trim();
  if (!/^(索引|规范|契约|表)/.test(eff)) problems.push(rel + "：效力值必须是 索引 / 规范 / 契约 / 表，实际：" + eff);
  if (/^(规范|契约)/.test(eff)) {
    if (!/核对[：:]\s*\d{4}-\d{2}-\d{2}/.test(text)) problems.push(rel + "：规范 / 契约需要「核对：YYYY-MM-DD」");
    if (!/依据[：:]\s*\S/.test(text)) problems.push(rel + "：规范 / 契约需要「依据：…」");
  }
}

function hasAnchor(line) {
  if (/gaps\.yaml#/.test(line)) return true;
  if (/<!--\s*BEGIN:/.test(line)) return true;
  if (/\]\([^)]+\.md/.test(line)) return true;
  for (const m of line.matchAll(/`([^`]+)`/g)) {
    const t = m[1];
    if (t.includes("/") || t.includes("::") || /\.(rs|js|mjs|cjs|md|ya?ml|toml|json)$/.test(t)) return true;
  }
  return false;
}

function validateBlock(rel, type, tpl, sec, problems) {
  const at = rel + "：" + sec.no + " 行「" + stripNum(sec.heading) + "」";
  const body = sec.body;
  let inFence = false, fenceCount = 0, bullets = 0, paras = [];
  const tables = [];
  let curTable = null;
  for (const it of body) {
    const t = it.line.trim();
    if (/^```/.test(t)) { inFence = !inFence; if (inFence) fenceCount++; curTable = null; continue; }
    if (inFence) continue;
    if (!t) { curTable = null; continue; }
    if (/^\|/.test(t)) {
      if (/^\|[\s:|-]+\|$/.test(t)) continue;
      const cells = t.replace(/^\|/, "").replace(/\|$/, "").split("|").map((x) => x.trim());
      if (!curTable) { curTable = { cells, no: it.no }; tables.push(curTable); }
      continue;
    }
    curTable = null;
    if (/^[-*]\s+/.test(t) || /^\d+[.)]\s+/.test(t)) { bullets++; continue; }
    if (/^>/.test(t) || /^#{3,6}\s/.test(t)) continue;
    paras.push({ t, no: it.no });
  }
  if (paras.length > 1) problems.push(at + "：块内纯段落 " + paras.length + " 行（只允许 1 行引导语），行 " + paras.map((p) => p.no).join(","));
  for (const p of paras) {
    const n = [...p.t].length;
    if (n > LEAD_MAX) problems.push(at + "：引导语超过 " + LEAD_MAX + " 字（" + n + "），行 " + p.no);
  }
  if (type === "portal" && fenceCount > 0) problems.push(at + "：门户不允许代码块");
  if (type === "reference") {
    let f = false;
    for (const it of body) {
      if (/^```/.test(it.line.trim())) { f = !f; continue; }
      if (f) continue;
      if (/(^|[^A-Za-z0-9_])(src|capabilities)\//.test(it.line) || /::[A-Za-z]/.test(it.line) || /\.rs\b/.test(it.line) || /当前状态|已实现|未验/.test(it.line)) {
        problems.push(at + "：参考文档不允许实现细节 / 运行状态（行 " + it.no + "）");
        break;
      }
    }
  }
  if (tpl.repeat) {
    if (tpl.repeat.id === "route-table" && tables.length === 0) problems.push(at + "：路由块缺少表格");
    if (tpl.repeat.id === "rule-group" && bullets === 0) problems.push(at + "：规则组缺少条目");
    if (tpl.repeat.id === "topic" && bullets === 0) problems.push(at + "：专题缺少条目");
    if (tpl.repeat.id === "field-group" && tables.length === 0) problems.push(at + "：字段组缺少字段表");
  }
  if (tpl.tableHeader) {
    for (const tb of tables) {
      if (JSON.stringify(tb.cells) !== JSON.stringify(tpl.tableHeader)) {
        problems.push(at + "：" + tb.no + " 行表头应为「" + tpl.tableHeader.join(" | ") + "」，实际「" + tb.cells.join(" | ") + "」");
      }
    }
  }
}

function validateMap(rel, parsed, tpl, problems) {
  const tables = [];
  let cur = null;
  for (const it of parsed.headerLines) {
    const t = it.line.trim();
    if (/^\|/.test(t)) {
      if (/^\|[\s:|-]+\|$/.test(t)) continue;
      const cells = t.replace(/^\|/, "").replace(/\|$/, "").split("|").map((x) => x.trim());
      if (!cur) { cur = { cells, no: it.no }; tables.push(cur); }
      continue;
    }
    cur = null;
  }
  if (tables.length !== 1) problems.push(rel + "：单元地图必须有且只有一张表，实际 " + tables.length);
  else if (JSON.stringify(tables[0].cells) !== JSON.stringify(tpl.tableHeader)) {
    problems.push(rel + "：" + tables[0].no + " 行表头应为「文件 | 职责」，实际「" + tables[0].cells.join(" | ") + "」");
  }
}

function validateSections(rel, type, tpl, parsed, problems) {
  const doc = parsed.sections;
  let si = 0;
  for (let ti = 0; ti < tpl.sections.length; ti++) {
    const t = tpl.sections[ti];
    if (t.level === 1) continue;
    const re = headingRe(t.text);
    if (t.repeat) {
      const next = tpl.sections[ti + 1];
      const nextRe = next && next.level >= 2 ? headingRe(next.text) : null;
      let count = 0;
      while (si < doc.length) {
        const h = doc[si].heading.trim();
        if (nextRe && nextRe.test(h)) break;
        if (!re.test(h)) break;
        validateBlock(rel, type, t, doc[si], problems);
        si++; count++;
      }
      if (count < t.repeat.min) problems.push(rel + "：缺少重复块「" + stripNum(t.text) + "」至少 " + t.repeat.min + " 个，实际 " + count);
      continue;
    }
    if (si >= doc.length) { problems.push(rel + "：缺少章节「" + stripNum(t.text) + "」"); continue; }
    if (re.test(doc[si].heading.trim())) { validateBlock(rel, type, t, doc[si], problems); si++; continue; }
    let k = si;
    while (k < doc.length && !re.test(doc[k].heading.trim())) k++;
    if (k < doc.length) {
      for (let x = si; x < k; x++) problems.push(rel + "：" + doc[x].no + " 行多余章节「" + stripNum(doc[x].heading) + "」");
      validateBlock(rel, type, t, doc[k], problems);
      si = k + 1;
    } else {
      problems.push(rel + "：" + doc[si].no + " 行章节顺序不符：期望「" + stripNum(t.text) + "」，实际「" + stripNum(doc[si].heading) + "」");
      si++;
    }
  }
  while (si < doc.length) { problems.push(rel + "：" + doc[si].no + " 行多余章节「" + stripNum(doc[si].heading) + "」"); si++; }
}

function checkClaims(rel, content, problems) {
  const lines = content.split("\n");
  let inFence = false;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    if (/^```/.test(raw.trim())) { inFence = !inFence; continue; }
    if (inFence) continue;
    const tt = raw.trim();
    if (!/^[#>|]/.test(tt) && MODAL.test(raw) && !hasAnchor(raw)) problems.push(rel + "：" + (i + 1) + " 行强约束缺少锚点");
    for (const m of raw.matchAll(/`([^`]+)`/g)) {
      const token = m[1];
      if (!token.includes("/")) continue;
      if (/[<>*{}…]/.test(token)) continue;
      if (/^(node|cargo|gh|git|solomni|npm|npx|rustup)\b/.test(token.trim())) continue;
      const clean = token.replace(/:\d+(-\d+)?$/, "").replace(/#.*$/, "");
      if (!/\.(rs|js|mjs|cjs|md|ya?ml|toml|json)$/.test(clean)) continue;
      if (!fs.existsSync(path.join(ROOT, clean))) problems.push(rel + "：" + (i + 1) + " 行锚点不存在：" + clean);
    }
  }
}

function checkDoc(rel, content) {
  const problems = [];
  const type = resolveType(rel);
  if (type === "exception") return { type, problems };
  if (!type) { problems.push(rel + "：未登记文档类型（docs/templates/index.json）"); return { type: null, problems }; }
  const tpl = loadTemplate(type);
  const parsed = parseDoc(content);
  validateHeader(rel, parsed, problems);
  if (type === "map") validateMap(rel, parsed, tpl, problems);
  else validateSections(rel, type, tpl, parsed, problems);
  checkClaims(rel, content, problems);
  return { type, problems };
}

function collectDocs() {
  const out = [];
  const walk = (abs, rel) => {
    for (const e of fs.readdirSync(abs, { withFileTypes: true })) {
      const a = path.join(abs, e.name);
      const r = rel ? rel + "/" + e.name : e.name;
      if (e.isDirectory()) {
        const always = e.name === "target" || e.name === ".git" || e.name === "node_modules";
        const rootData = rel === "" && SKIP_DIRS.has(e.name);
        if (!always && !rootData) walk(a, r);
      } else if (e.name.endsWith(".md")) out.push(r);
    }
  };
  walk(ROOT, "");
  return out.sort();
}

function runAll() {
  const docs = collectDocs();
  const results = [];
  for (const rel of docs) {
    const content = fs.readFileSync(path.join(ROOT, rel), "utf8");
    const r = checkDoc(rel, content);
    results.push({ rel, type: r.type, problems: r.problems });
  }
  return results;
}

function summary(results, opts) {
  const byType = {};
  for (const r of results) {
    const k = r.type || "unmatched";
    byType[k] = byType[k] || { docs: 0, problems: 0, sample: [] };
    byType[k].docs++;
    byType[k].problems += r.problems.length;
    if (r.problems.length && byType[k].sample.length < (opts.verbose ? 999 : 4)) byType[k].sample.push(...r.problems.slice(0, opts.verbose ? 999 : 4));
  }
  console.log("[docs-gov] 扫描 " + results.length + " 份 Markdown；强制类型：" + (opts.strict ? "全部（--strict）" : (CFG.enforce.length ? CFG.enforce.join(",") : "无（报告模式）")));
  for (const k of Object.keys(byType).sort()) {
    console.log("[docs-gov] " + k.padEnd(10) + " 文档 " + String(byType[k].docs).padStart(3) + " 份，问题 " + String(byType[k].problems).padStart(4) + " 条");
    for (const p of byType[k].sample) console.log("           - " + p);
  }
}

function selfTest() {
  const BT = String.fromCharCode(96);
  const F = (n) => BT.repeat(n);
  const unit = [
    "# demo（测试）",
    "> 定位：测试单元",
    "> 效力：规范｜核对：2026-11-04｜依据：" + BT + "src/main.rs" + BT,
    "",
    "## 管什么 / 不管什么",
    "- 管：demo",
    "- 不管：别的",
    "",
    "## 入站契约与状态归属",
    "- " + BT + "api::Demo" + BT + " 是入站面",
    "",
    "## 依赖图位置（由源码的 " + BT + "::api" + BT + " 引用推导）",
    "- 经 " + BT + "::api" + BT + " 用到：" + BT + "kernel" + BT,
    "- 谁在用我：" + BT + "cli" + BT,
    "",
    "## 改动本单元时必须同步",
    "- " + BT + "src/main.rs" + BT + " → 同步",
    "",
    "## 本目录",
    "| 文件 | 内容 |",
    "| --- | --- |",
    "| " + BT + "unit-map.md" + BT + " | 文件职责 |"
  ];
  const portal = [
    "# 门户",
    "> 定位：测试门户",
    "> 效力：索引｜核对：2026-11-04",
    "",
    "## 路由",
    "| 要找什么 | 读哪一份 |",
    "| --- | --- |",
    "| x | " + BT + "docs/README.md" + BT + " |",
    "",
    "## 改动时必须同步",
    "- " + BT + "src/main.rs" + BT + " → x"
  ];
  const spec = [
    "# 规范",
    "> 定位：测试",
    "> 效力：规范｜核对：2026-11-04｜依据：" + BT + "src/main.rs" + BT,
    "",
    "## 适用与判据",
    "- 所有人",
    "",
    "## 一、基础",
    "- 必须保持单一权限（" + BT + "src/main.rs" + BT + "）",
    "",
    "## 检查",
    "- 基础 → T0",
    "",
    "## 路由",
    "- " + BT + "docs/README.md" + BT
  ];
  const reference = [
    "# 参考",
    "> 定位：格式",
    "> 效力：契约｜核对：2026-11-04｜依据：" + BT + "Cargo.toml" + BT,
    "",
    "## 结构",
    F(3) + "text",
    "x",
    F(3),
    "",
    "## 字段",
    "| 字段 | 类型 | 必填 | 默认 | 说明 |",
    "| --- | --- | --- | --- | --- |",
    "| a | string | 是 | - | a |",
    "",
    "## 示例",
    F(3) + "yaml",
    "a: b",
    F(3),
    "",
    "## 校验与拒收",
    "- 缺 a 拒收",
    "",
    "## 路由",
    "- " + BT + "docs/README.md" + BT
  ];
  const map = [
    "# 地图",
    "> 定位：测试",
    "> 效力：契约｜核对：2026-11-04｜依据：" + BT + "src/main.rs" + BT,
    "",
    "| 文件 | 职责 |",
    "| --- | --- |",
    "| " + BT + "src/main.rs" + BT + " | x |"
  ];
  const cases = [
    { name: "unit 合法", rel: "docs/demo/README.md", body: unit, expect: [] },
    { name: "unit 缺效力", rel: "docs/demo/README.md", body: unit.map((l) => l.replace(/^> 效力.*$/, "")), expect: ["效力"] },
    { name: "unit 纯段落两行", rel: "docs/demo/README.md", body: unit.map((l) => l === "- " + BT + "api::Demo" + BT + " 是入站面" ? "第一段\n第二段" : l), expect: ["引导语"] },
    { name: "unit 强约束缺锚点", rel: "docs/demo/README.md", body: unit.map((l) => l === "- " + BT + "src/main.rs" + BT + " → 同步" ? "- 必须同步唯一实现" : l), expect: ["锚点"] },
    { name: "unit 锚点不存在", rel: "docs/demo/README.md", body: unit.map((l) => l === "- " + BT + "src/main.rs" + BT + " → 同步" ? "- " + BT + "src/nope_123.rs" + BT + " → 同步" : l), expect: ["锚点不存在"] },
    { name: "unit 多余章节", rel: "docs/demo/README.md", body: unit.flatMap((l) => l === "## 本目录" ? ["## 多余", "- x", "", "## 本目录"] : [l]), expect: ["多余"] },
    { name: "portal 代码块", rel: "docs/README.md", body: portal.flatMap((l) => l === "## 改动时必须同步" ? [F(3) + "text", "x", F(3), "", "## 改动时必须同步"] : [l]), expect: ["代码块"] },
    { name: "reference 实现细节", rel: "RUNTIME_SPEC.md", body: reference.map((l) => l === "- " + BT + "docs/README.md" + BT ? "- " + BT + "src/main.rs" + BT : l), expect: ["实现细节"] },
    { name: "map 表头错误", rel: "docs/demo/unit-map.md", body: map.map((l) => l === "| 文件 | 职责 |" ? "| 路径 | 职责 |" : l), expect: ["表头"] },
    { name: "spec 缺规则组", rel: "ARCHITECTURE.md", body: spec.filter((l) => !/^## 一、基础$/.test(l) && !/必须保持/.test(l)), expect: ["规则组"] },
    { name: "未登记文档", rel: "foo/bar.md", body: unit, expect: ["未登记"] },
    { name: "引导语超长", rel: "docs/demo/README.md", body: unit.map((l) => l === "- " + BT + "api::Demo" + BT + " 是入站面" ? "长".repeat(151) : l), expect: ["150"] }
  ];
  let failed = 0;
  for (const c of cases) {
    const got = checkDoc(c.rel, c.body.join("\n")).problems;
    const ok = c.expect.length === 0 ? got.length === 0 : c.expect.some((e) => got.some((g) => g.includes(e)));
    if (!ok) { failed++; console.log("[self-test] FAIL " + c.name + " => " + JSON.stringify(got.slice(0, 3))); }
    else console.log("[self-test] ok " + c.name);
  }
  if (failed) { console.log("[self-test] 失败 " + failed + " 条"); process.exit(1); }
  console.log("[self-test] 全部通过（" + cases.length + " 条）");
}

const args = process.argv.slice(2);
if (args[0] && args[0].indexOf("--dump=") === 0) { console.log(JSON.stringify(loadTemplate(args[0].slice(7)), null, 1)); process.exit(0); }
if (args.includes("--self-test")) { selfTest(); process.exit(0); }
const strict = args.includes("--strict");
const onlyArg = args.find((a) => a.indexOf("--only=") === 0);
const typeArg = args.find((a) => a.indexOf("--type=") === 0);
const exArg = args.find((a) => a.indexOf("--exclude=") === 0);
const only = onlyArg ? onlyArg.slice(7).split(",").map((s) => s.trim()).filter(Boolean) : null;
const types = typeArg ? typeArg.slice(7).split(",").map((s) => s.trim()).filter(Boolean) : null;
const excl = exArg ? exArg.slice(10).split(",").map((s) => s.trim()).filter(Boolean) : null;
let results = runAll();
if (types) results = results.filter((r) => types.includes(r.type));
if (only) results = results.filter((r) => only.some((p) => r.rel === p || r.rel.indexOf(p) === 0));
if (excl) results = results.filter((r) => !excl.some((p) => r.rel === p || r.rel.indexOf(p) === 0));
const fatal = results.filter((r) => (strict || CFG.enforce.includes(r.type)) && r.problems.length);
if (args.includes("--json")) {
  console.log(JSON.stringify({ results, fatal: fatal.length }, null, 1));
} else {
  summary(results, { strict, verbose: args.includes("--verbose") });
}
if (fatal.length) { console.log("[docs-gov] 强制类型有 " + fatal.length + " 份文档不合规"); process.exit(1); }
process.exit(0);
