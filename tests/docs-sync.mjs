#!/usr/bin/env node
/**
 * 生成段同步：把机器可读事实生成到文档标记之间（文档不手写这些内容）。
 * 来源（唯一）：
 * - platform-status：src/units/vm/domain/exec.rs 的 VERIFIED_HOSTS + tests/<平台>/gaps.yaml 的条目数；
 * - gate-steps：tests/gates/steps.js 的 listModel()（与 `node run-tests.js --list --json` 同源），取 "T0 " 前缀步骤；
 * - success-markers：tests/lib/markers.js 的 MARKERS（标记的唯一声明）；
 * - layer-table：layers.yaml 的分层表（能力分层的唯一权威，文档里的表不手写）；
 * - mutation-scope：各单元 `tests/specs/<unit>.yaml` 的 `mutation` 块（突变范围的唯一声明处）；
 * - dependency-map：单元之间的依赖边，取自 `tests/gates/structure/layers.js` 的 parseLayers + edgesOf（与 Structure.html 同源）。
 * 用法：node tests/docs-sync.mjs [--check|--write]；默认 --check（CI / --docs 用）。
 */
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const HOSTS_FILE = path.join(ROOT, "src", "units", "vm", "domain", "exec.rs");
const PLATFORMS = ["linux", "windows", "macos"];
const NAMES = { linux: "Linux", windows: "Windows", macos: "macOS" };
const NAMES_EN = { linux: "Linux", windows: "Windows", macos: "macOS" };

function verifiedHosts() {
  const text = fs.readFileSync(HOSTS_FILE, "utf8");
  const m = text.match(/const\s+VERIFIED_HOSTS\s*:[^=]*=\s*&\[([^\]]*)\]/);
  if (!m) throw new Error("找不到 VERIFIED_HOSTS：" + HOSTS_FILE);
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}
function gapCount(platform) {
  const f = path.join(ROOT, "tests", platform, "gaps.yaml");
  if (!fs.existsSync(f)) return 0;
  return fs.readFileSync(f, "utf8").split("\n").filter((l) => /^- id:/.test(l)).length;
}
function platformStatus() {
  const verified = verifiedHosts();
  const unverified = PLATFORMS.filter((p) => !verified.includes(p) && gapCount(p) > 0);
  const zh = "已真机验收：" + verified.map((p) => NAMES[p] || p).join("、") +
    (unverified.length ? "；未验：" + unverified.map((p) => NAMES[p] || p).join("、") + "（缺口见 " + unverified.map((p) => "`tests/" + p + "/gaps.yaml`").join("、") + "）" : "") + "。";
  const en = "Verified on real hardware: " + verified.map((p) => NAMES_EN[p] || p).join(", ") +
    (unverified.length ? "; not verified: " + unverified.map((p) => NAMES_EN[p] || p).join(", ") + " (gaps in " + unverified.map((p) => "`tests/" + p + "/gaps.yaml`").join(", ") + ")" : "") + ".";
  return { verified, unverified, zh, en };
}

/** T0 步骤：名称 / 可行状态 / 环境跳过出口，全部取自 steps.listModel()。 */
function t0Steps() {
  const steps = require("./gates/steps.js").listModel();
  return steps.filter((s) => s.name.indexOf("T0 ") === 0);
}
function renderGateSteps() {
  const steps = t0Steps();
  if (!steps.length) throw new Error("steps.listModel() 里没有 T0 步骤");
  const rows = steps.map((s) => "| `" + s.name + "` | " + s.status.join(" / ") + " | " + (s.envSkip ? s.envSkip : "—") + " |");
  const optional = steps.filter((s) => s.status.indexOf("env-skip") >= 0).length;
  const hard = steps.length - optional;
  return [
    "<!-- BEGIN:gate-steps -->",
    "| 步骤（报告 `steps[].step`） | 可行状态（`status[]`） | 环境跳过（`envSkip`） |",
    "| --- | --- | --- |",
    ...rows,
    "",
    "> " + steps.length + " 项：" + hard + " 项没有环境跳过出口（不过即硬失败），" + optional + " 项在工具 / 环境不具备时可记 `env-skip`（不算通过）。",
    "<!-- END:gate-steps -->",
  ].join("\n");
}

/** 成功标记：文本 / 含义 / 产生处，全部取自 markers.MARKERS。 */
function renderSuccessMarkers() {
  const list = require("./lib/markers.js").MARKERS;
  if (!list || !list.length) throw new Error("markers.MARKERS 为空");
  const rows = list.map((m) => "| `" + m.text + "` | " + m.meaning + " | `" + m.owner + "` |");
  return [
    "<!-- BEGIN:success-markers -->",
    "| 标记 | 含义（`tests/lib/markers.js`） | 产生处 |",
    "| --- | --- | --- |",
    ...rows,
    "<!-- END:success-markers -->",
  ].join("\n");
}

/** 环表：环 / 单元 / 可依赖，全部取自 layers.yaml；环数与单元数由它现算，不写死在文档里。 */
function renderLayerTable() {
  const { parseLayers } = require("./gates/structure/layers.js");
  const { tiers, problems } = parseLayers(ROOT);
  if (problems.length) throw new Error(problems.join("；"));
  const byRing = new Map();
  for (const [name, ring] of tiers) {
    if (name === "kernel") continue;
    if (!byRing.has(ring)) byRing.set(ring, []);
    byRing.get(ring).push(name);
  }
  const top = [...byRing.keys()].sort((a, b) => a - b);
  const title = { 1: "1（纯）", 2: "2（状态与机制）", 3: "3（用例）" };
  const rows = ["| 0（地基） | `kernel` | 无 |"];
  for (const t of top) {
    rows.push("| " + (title[t] || String(t)) + " | " + byRing.get(t).sort().map((n) => "`" + n + "`").join("、") + " | " + (t === 1 ? "0" : "0–" + (t - 1)) + " |");
  }
  const caps = [...tiers.keys()].filter((k) => k !== "kernel").length;
  return [
    "<!-- BEGIN:layer-table -->",
    "| 环 | 单元 | 可依赖 |",
    "| --- | --- | --- |",
    ...rows,
    "",
    "> 4 环、" + caps + " 个单元 + `kernel`：本表由 `layers.yaml` 生成（`node tests/docs-sync.mjs --write`），不手写；环 4（`presentation/`、`entry/`）是适配器，不入表。",
    "<!-- END:layer-table -->",
  ].join("\n");
}

/** 突变范围表：单元 / 文件 / 调度，全部取自各单元 spec 的 mutation 块；单元数与文件数现算。 */
function renderMutationScope() {
  const { loadSpecs } = require("./lib/spec.js");
  const { specs, problems } = loadSpecs(ROOT);
  if (problems.length) throw new Error(problems.join("；"));
  const declared = specs.filter((s) => s.mutation);
  const rows = [];
  for (const s of declared) {
    for (const f of s.mutation.files) rows.push("| `" + s.unit + "` | `" + f + "` | " + s.mutation.schedule + " |");
  }
  const weekly = declared.filter((s) => s.mutation.schedule === "weekly");
  const weeklyFiles = weekly.reduce((n, s) => n + s.mutation.files.length, 0);
  return [
    "<!-- BEGIN:mutation-scope -->",
    "| 单元 | 文件 | 调度 |",
    "| --- | --- | --- |",
    ...rows,
    "",
    "> " + weekly.length + " 个单元、周检 " + weeklyFiles + " 个文件；本表由 `tests/specs/*.yaml` 的 `mutation` 块生成（`node tests/docs-sync.mjs --write`），不手写。",
    "<!-- END:mutation-scope -->",
  ].join("\n");
}

/** 依赖边：单元之间的边只有一处来源（layers.js 的 edgesOf），环级聚合与单元级边表都由它推。 */
const BT = String.fromCharCode(96);
function dependencyFacts() {
  const { parseLayers, dirsOf, edgesOf } = require("./gates/structure/layers.js");
  const { tiers, problems } = parseLayers(ROOT);
  if (problems.length) throw new Error(problems.join("；"));
  const same = [];
  const crossPairs = [];
  for (const [key, hit] of edgesOf(ROOT, dirsOf(ROOT))) {
    const parts = key.split(">");
    const from = parts[0];
    const to = parts[1];
    const a = tiers.get(from);
    const b = tiers.get(to);
    if (a === undefined || b === undefined) continue;
    if (a === b) same.push({ ring: a, from: from, to: to, refs: hit.refs });
    else crossPairs.push({ from: from, to: to, refs: hit.refs, fromRing: a, toRing: b });
  }
  const byRing = new Map();
  for (const e of crossPairs) {
    const k = e.fromRing + ">" + e.toRing;
    const got = byRing.get(k) || { from: e.fromRing, to: e.toRing, refs: 0, pairs: 0 };
    got.refs += e.refs;
    got.pairs++;
    byRing.set(k, got);
  }
  return { tiers: tiers, same: same, crossPairs: crossPairs, byRing: [...byRing.values()] };
}

const RING_LABEL = { 0: "环 0 · kernel", 1: "环 1 · 纯", 2: "环 2 · 状态与机制", 3: "环 3 · 用例" };
const ringName = (n) => RING_LABEL[n] || "环 " + n;
const byCount = (a, b) => b.refs - a.refs || (a.from < b.from ? -1 : a.from > b.from ? 1 : 0);
const unitCell = (name) => BT + name + BT;

/** 环级：跨环边只有向内一条路，聚合后的单元对数与引用处数由代码现算。 */
function renderDependencyRings() {
  const { tiers, byRing } = dependencyFacts();
  const rings = [...new Set(tiers.values())].sort((a, b) => a - b);
  const countOf = (r) => [...tiers.values()].filter((v) => v === r).length;
  const links = byRing.slice().sort((a, b) => a.from - b.from || a.to - b.to);
  const fence = BT + BT + BT;
  const rows = links.map((l) => "| " + ringName(l.from) + " | " + ringName(l.to) + " | " + l.pairs + " | " + l.refs + " |");
  const refs = links.reduce((n, l) => n + l.refs, 0);
  const pairs = links.reduce((n, l) => n + l.pairs, 0);
  return [
    "<!-- BEGIN:dependency-rings -->",
    fence + "mermaid",
    "flowchart RL",
    ...rings.map((r) => "  R" + r + '["' + ringName(r) + (r === 0 ? "" : "<br/>" + countOf(r) + " 个单元") + '"]'),
    ...links.map((l) => "  R" + l.from + ' -->|"' + l.refs + " 处 / " + l.pairs + ' 对"| R' + l.to),
    fence,
    "",
    "| 从 | 到 | 单元对 | 引用处 |",
    "| --- | --- | --- | --- |",
    ...rows,
    "",
    "> " + links.length + " 种跨环边、" + pairs + " 对单元、" + refs + " 处引用；本段由 " + unitCell("tests/gates/structure/layers.js") + " 现算（" + unitCell("node tests/docs-sync.mjs --write") + "），不手写。",
    "<!-- END:dependency-rings -->",
  ].join("\n");
}

/** 同环：rank 由同环边现算，边必须指向 rank 更低的单元（同环无环）。 */
function renderDependencySame() {
  const { same } = dependencyFacts();
  const rows = same.slice().sort((a, b) => a.ring - b.ring || byCount(a, b))
    .map((e) => "| " + ringName(e.ring) + " | " + unitCell(e.from) + " | " + unitCell(e.to) + " | " + e.refs + " |");
  return [
    "<!-- BEGIN:dependency-same -->",
    "| 环 | 从 | 到 | 引用处 |",
    "| --- | --- | --- | --- |",
    ...rows,
    "",
    "> " + rows.length + " 对同环边；同环没有层，只有先后（rank 由这些边现算），所以这张表就是 rank 的原料。",
    "<!-- END:dependency-same -->",
  ].join("\n");
}

/** 跨环：每一种「从哪个环到哪个环」都在这里，条数即 H1 的证据面。 */
function renderDependencyCross() {
  const { crossPairs } = dependencyFacts();
  const rows = crossPairs.slice().sort((a, b) => a.fromRing - b.fromRing || byCount(a, b))
    .map((e) => "| " + unitCell(e.from) + " | " + unitCell(e.to) + " | " + ringName(e.fromRing) + " → " + ringName(e.toRing) + " | " + e.refs + " |");
  return [
    "<!-- BEGIN:dependency-cross -->",
    "| 从 | 到 | 方向 | 引用处 |",
    "| --- | --- | --- | --- |",
    ...rows,
    "",
    "> " + rows.length + " 对跨环边；方向一律向内（外层 → 内层），向上与同环逆序由 " + unitCell("tests/gates/structure/layers.js") + " 判红，本表只记现状。",
    "<!-- END:dependency-cross -->",
  ].join("\n");
}
const TARGETS = [
  { file: "README.md", marker: "platform-status", source: "platform-status", lang: "zh" },
  { file: "README_EN.md", marker: "platform-status-en", source: "platform-status", lang: "en" },
  { file: "PRODUCT.md", marker: "platform-status", source: "platform-status", lang: "zh" },
  { file: "docs/vm/README.md", marker: "platform-status", source: "platform-status", lang: "zh" },
  { file: "docs/testing/levels.md", marker: "gate-steps", source: "gate-steps" },
  { file: "docs/testing/execution-ci.md", marker: "success-markers", source: "success-markers" },
  { file: "docs/testing/execution-ci.md", marker: "mutation-scope", source: "mutation-scope" },
  { file: "docs/architecture/layers.md", marker: "layer-table", source: "layer-table" },
  { file: "docs/architecture/dependency-map.md", marker: "dependency-rings", source: "dependency-rings" },
  { file: "docs/architecture/dependency-map.md", marker: "dependency-same", source: "dependency-same" },
  { file: "docs/architecture/dependency-map.md", marker: "dependency-cross", source: "dependency-cross" },
];

function render(target, payload) {
  if (target.source === "platform-status") {
    const text = target.lang === "en" ? payload.en : payload.zh;
    return "<!-- BEGIN:" + target.marker + " -->\n> " + text + "\n<!-- END:" + target.marker + " -->";
  }
  if (target.source === "layer-table") return payload;
  if (target.source === "gate-steps") return payload;
  if (target.source === "success-markers") return payload;
  if (target.source === "mutation-scope") return payload;
  if (target.source === "dependency-rings") return payload;
  if (target.source === "dependency-same") return payload;
  if (target.source === "dependency-cross") return payload;
  throw new Error("未知来源：" + target.source);
}

function main() {
  const write = process.argv.includes("--write");
  const payload = {
    "platform-status": platformStatus(),
    "gate-steps": renderGateSteps(),
    "success-markers": renderSuccessMarkers(),
    "mutation-scope": renderMutationScope(),
    "layer-table": renderLayerTable(),
    "dependency-rings": renderDependencyRings(),
    "dependency-same": renderDependencySame(),
    "dependency-cross": renderDependencyCross(),
  };
  const problems = [];
  let changed = 0;
  for (const t of TARGETS) {
    const abs = path.join(ROOT, t.file);
    if (!fs.existsSync(abs)) { problems.push("缺文件：" + t.file); continue; }
    const text = fs.readFileSync(abs, "utf8");
    const re = new RegExp("<!-- BEGIN:" + t.marker + " -->[\\s\\S]*?<!-- END:" + t.marker + " -->");
    const want = render(t, payload[t.source]);
    if (!re.test(text)) { problems.push(t.file + "：缺生成段标记 " + t.marker); continue; }
    const cur = text.match(re)[0];
    if (cur !== want) {
      changed++;
      if (write) fs.writeFileSync(abs, text.replace(re, want), "utf8");
      else problems.push(t.file + "：生成段与来源不一致（跑 node tests/docs-sync.mjs --write）");
    }
  }
  const st = payload["platform-status"];
  console.log("[docs-sync] platform-status：VERIFIED_HOSTS=[" + st.verified.join(",") + "]，未验=" + (st.unverified.join(",") || "无"));
  console.log("[docs-sync] gate-steps：" + t0Steps().length + " 项；success-markers：" + require("./lib/markers.js").MARKERS.length + " 条");
  console.log("[docs-sync] 目标 " + TARGETS.length + " 份，需更新 " + changed + " 份" + (write ? "（已写入）" : ""));
  for (const p of problems) console.log("  - " + p);
  if (problems.length) process.exit(1);
}
main();