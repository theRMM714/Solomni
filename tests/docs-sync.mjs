#!/usr/bin/env node
/**
 * 生成段同步：把机器可读事实生成到文档标记之间（文档不手写这些内容）。
 * 来源（唯一）：
 * - platform-status：src/capabilities/vm/domain/exec.rs 的 VERIFIED_HOSTS + tests/<平台>/gaps.yaml 的条目数；
 * - gate-steps：tests/gates/steps.js 的 listModel()（与 `node run-tests.js --list --json` 同源），取 "T0 " 前缀步骤；
 * - success-markers：tests/lib/markers.js 的 MARKERS（标记的唯一声明）。
 * 用法：node tests/docs-sync.mjs [--check|--write]；默认 --check（CI / --docs 用）。
 */
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const HOSTS_FILE = path.join(ROOT, "src", "capabilities", "vm", "domain", "exec.rs");
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

const TARGETS = [
  { file: "README.md", marker: "platform-status", source: "platform-status", lang: "zh" },
  { file: "README_EN.md", marker: "platform-status-en", source: "platform-status", lang: "en" },
  { file: "PRODUCT.md", marker: "platform-status", source: "platform-status", lang: "zh" },
  { file: "docs/vm/README.md", marker: "platform-status", source: "platform-status", lang: "zh" },
  { file: "docs/testing/levels.md", marker: "gate-steps", source: "gate-steps" },
  { file: "docs/testing/execution-ci.md", marker: "success-markers", source: "success-markers" },
];

function render(target, payload) {
  if (target.source === "platform-status") {
    const text = target.lang === "en" ? payload.en : payload.zh;
    return "<!-- BEGIN:" + target.marker + " -->\n> " + text + "\n<!-- END:" + target.marker + " -->";
  }
  if (target.source === "gate-steps") return payload;
  if (target.source === "success-markers") return payload;
  throw new Error("未知来源：" + target.source);
}

function main() {
  const write = process.argv.includes("--write");
  const payload = {
    "platform-status": platformStatus(),
    "gate-steps": renderGateSteps(),
    "success-markers": renderSuccessMarkers(),
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