#!/usr/bin/env node
/**
 * 季度审计候选清单（只报不拦，不进门禁）：
 * - stale：效力行的「依据」在「核对」日之后又变过；
 * - 状态断言缺证据：已验 / 未验 / 实现中 / 缺口 附近没有可核对锚点；
 * - 重复事实：同一句实质内容出现在多份文档；
 * - 失效锚点：链接里的 #标题 在目标文档不存在；
 * - 孤儿：受治理文档没有任何入链；
 * - 异常：模板 exceptions 清单（人工定期复核）；
 * - 原则句：没有锚点、只能靠判断的句子（人工/AI 复核候选）。
 * 用法：node tests/docs-audit.mjs [--json] [--top N]
 */
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TPL_DIR = path.join(ROOT, "docs", "templates");
const CFG = JSON.parse(fs.readFileSync(path.join(TPL_DIR, "index.json"), "utf8"));
const SKIP_DIRS = new Set(["target", ".git", "node_modules", ".tools", ".home", ".vm", "dependencies", "logs", "session", "runtimes", "platform", "demo", "modules", "tests"]);

const esc = (s) => s.replace(/[.*+?^$()|[\]\\]/g, "\\$&");
const globRe = (g) => new RegExp("^" + esc(g).replace(/\\\*\\\*/g, ".*").replace(/\\\*/g, "[^/]*") + "$");
function resolveType(rel) {
  for (const t of Object.keys(CFG.templates)) for (const g of CFG.templates[t].globs) if (globRe(g).test(rel)) return t;
  for (const e of CFG.exceptions) if (globRe(e.path).test(rel)) return "exception";
  return null;
}
function collect() {
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
const dateCache = {};
function lastCommitDate(p) {
  if (p in dateCache) return dateCache[p];
  try {
    dateCache[p] = execFileSync("git", ["log", "-1", "--format=%cs", "--", p], { cwd: ROOT, encoding: "utf8" }).trim();
  } catch { dateCache[p] = ""; }
  return dateCache[p];
}
const slug = (s) => s.toLowerCase().replace(/\[([^\]]*)\]\([^)]*\)/g, "$1").replace(/[\x60*_~]/g, "").replace(/[^\p{L}\p{N}\s-]/gu, "").trim().replace(/\s+/g, "-");

const docs = collect();
const parsed = {};
const inbound = {};
const normLines = new Map();

for (const rel of docs) {
  const text = fs.readFileSync(path.join(ROOT, rel), "utf8");
  const lines = text.split("\n");
  const type = resolveType(rel);
  const head = lines.slice(0, 10).join("\n");
  const verify = (head.match(/核对[：:]\s*(\d{4}-\d{2}-\d{2})/) || [])[1] || "";
  const basisLine = (head.match(/依据[：:]([^\n]+)/) || [])[1] || "";
  const basis = [...basisLine.matchAll(/`([^`]+)`/g)].map((m) => m[1].replace(/:\d+(-\d+)?$/, "").replace(/#.*$/, ""));
  const headings = [];
  const status = [];
  const principles = [];
  let inFence = false;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const t = raw.trim();
    if (/^```/.test(t)) { inFence = !inFence; continue; }
    if (inFence) continue;
    const hm = raw.match(/^(#{1,6})\s+(.+)$/);
    if (hm) { headings.push(hm[2].trim()); continue; }
    if (/^>/.test(t) || /^\|/.test(t)) continue;
    if (/(已验|未验|实现中)/.test(t) && !/(`[^`]*\/[^`]*`|::|gaps\.yaml|run-tests|CI|tests\/)/.test(t)) {
      status.push({ line: i + 1, text: t.slice(0, 100) });
    }
    if (/原则[：:]/.test(t)) principles.push({ line: i + 1, text: t.slice(0, 100) });
    if (t.length >= 20 && t.replace(/\[[^\]]*\]\([^)]*\)/g, "").replace(/[`*_>\s，。；、：|]/g, "").length >= 10) {
      const key = t.replace(/\[([^\]]*)\]\([^)]*\)/g, "$1").replace(/[\x60*_>]/g, "").replace(/[\s，。；、：]/g, "").replace(/[""'']/g, "");
      if (key.length >= 16) {
        if (!normLines.has(key)) normLines.set(key, []);
        normLines.get(key).push({ rel, line: i + 1, text: t.slice(0, 110) });
      }
    }
  }
  parsed[rel] = { type, verify, basis, headings, status, principles };
  if (!inbound[rel]) inbound[rel] = 0;
}
// inbound links + broken anchors
const brokenAnchors = [];
for (const rel of docs) {
  const text = fs.readFileSync(path.join(ROOT, rel), "utf8");
  for (const m of text.matchAll(/\]\(([^)#\s]+\.md)(?:#([^)]+))?\)/g)) {
    const target = path.relative(ROOT, path.resolve(path.dirname(path.join(ROOT, rel)), m[1])).split(path.sep).join("/");
    if (inbound[target] !== undefined) inbound[target]++;
    if (!m[2] || !parsed[target]) continue;
    const frag = decodeURIComponent(m[2]).trim();
    const hs = parsed[target].headings.map(slug);
    if (!hs.includes(slug(frag))) brokenAnchors.push(rel + " -> " + target + "#" + frag);
  }
}
// stale
const stale = [];
for (const rel of docs) {
  const p = parsed[rel];
  if (!p.verify) continue;
  for (const b of p.basis) {
    if (!fs.existsSync(path.join(ROOT, b))) continue;
    const d = lastCommitDate(b);
    if (d && d > p.verify) stale.push({ rel, basis: b, verified: p.verify, changed: d });
  }
}
// duplicates across docs
const dupGroups = [];
for (const [key, arr] of normLines) {
  const rels = [...new Set(arr.map((x) => x.rel))];
  if (rels.length >= 2) dupGroups.push({ key: arr[0].text, hits: arr.slice(0, 4) });
}
// orphans (governed only)
const orphans = docs.filter((d) => parsed[d].type !== "exception" && inbound[d] === 0);
const exceptions = CFG.exceptions.map((e) => e.path);
const statusAll = [];
for (const rel of docs) for (const s of parsed[rel].status) statusAll.push({ rel, line: s.line, text: s.text });
const principlesAll = [];
for (const rel of docs) for (const p of parsed[rel].principles) principlesAll.push({ rel, line: p.line, text: p.text });
const report = { scanned: docs.length, stale, status: statusAll, brokenAnchors, duplicates: dupGroups, orphans, exceptions, principles: principlesAll };

const TOPICS = [
  { name: "平台验收状态", re: /(已真机验收|未验|VERIFIED_HOSTS|virtiofsd 时退 9p|缺 ?smbd)/ },
  { name: "CI 必填检查", re: /(验收闸门|必填检查|required context)/ },
  { name: "工具链版本", re: /(3\.12\.4|24\.11\.0|0\.15\.2|1\.98)/ }
];
const topicHits = TOPICS.map((t) => ({ name: t.name, hits: [] }));
for (const rel of docs) {
  const lines = fs.readFileSync(path.join(ROOT, rel), "utf8").split("\n");
  let f = false;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    if (/^```/.test(raw.trim())) { f = !f; continue; }
    if (f) continue;
    for (const t of TOPICS) if (t.re.test(raw)) topicHits[TOPICS.indexOf(t)].hits.push({ rel, line: i + 1, text: raw.trim().slice(0, 90) });
  }
}
const multiSource = topicHits.filter((t) => new Set(t.hits.map((h) => h.rel)).size >= 2);
report.multiSource = multiSource;
const top = Number((process.argv.find((a) => a.startsWith("--top=")) || "--top=5").slice(6));
const jsonOut = path.join(ROOT, "target", "docs-audit-report.json");
fs.mkdirSync(path.dirname(jsonOut), { recursive: true });
fs.writeFileSync(jsonOut, JSON.stringify(report, null, 1));
if (process.argv.includes("--json")) { console.log(JSON.stringify(report, null, 1)); process.exit(0); }
console.log("[docs-audit] 候选清单（只报不拦）：扫描 " + docs.length + " 份");
console.log("- 过期（依据在核对日之后变过）：" + stale.length);
console.log("- 状态断言缺证据：" + statusAll.length);
console.log("- 重复事实（≥2 份文档同一句）：" + dupGroups.length);
console.log("- 失效锚点：" + brokenAnchors.length);
console.log("- 孤儿文档：" + orphans.length);
console.log("- 例外（人工复核）：" + exceptions.length);
console.log("- 原则句（无锚点判断）：" + report.principles.length);
console.log("- 多处同源事实：" + multiSource.length);
for (const t of multiSource) console.log("  topic " + t.name + "：" + [...new Set(t.hits.map((h) => h.rel))].join(", "));
for (const [i, s] of stale.slice(0, top).entries()) console.log("  stale " + (i + 1) + ") " + s.rel + " 依据 " + s.basis + " 在 " + s.changed + " 变过（核对 " + s.verified + "）");
for (const s of statusAll.slice(0, top)) console.log("  status " + s.rel + ":" + s.line + " " + s.text);
for (const g of dupGroups.slice(0, top)) console.log("  dup " + g.hits.map((h) => h.rel + ":" + h.line).join(" | ") + " :: " + g.key.slice(0, 80));
for (const o of orphans.slice(0, top)) console.log("  orphan " + o);
for (const b of brokenAnchors.slice(0, top)) console.log("  anchor " + b);
console.log("报告：" + path.relative(ROOT, jsonOut));
