#!/usr/bin/env node
/**
 * 生成段同步：把「平台验收状态」从代码常量与缺口账生成到文档标记之间。
 * 来源：src/capabilities/vm/domain/exec.rs 的 VERIFIED_HOSTS + tests/<平台>/gaps.yaml 的条目数。
 * 用法：node tests/docs-sync.mjs [--check|--write]；默认 --check（CI 用）。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
function status() {
  const verified = verifiedHosts();
  const unverified = PLATFORMS.filter((p) => !verified.includes(p) && gapCount(p) > 0);
  const zh = "已真机验收：" + verified.map((p) => NAMES[p] || p).join("、") +
    (unverified.length ? "；未验：" + unverified.map((p) => NAMES[p] || p).join("、") + "（缺口见 " + unverified.map((p) => "`tests/" + p + "/gaps.yaml`").join("、") + "）" : "") + "。";
  const en = "Verified on real hardware: " + verified.map((p) => NAMES_EN[p] || p).join(", ") +
    (unverified.length ? "; not verified: " + unverified.map((p) => NAMES_EN[p] || p).join(", ") + " (gaps in " + unverified.map((p) => "`tests/" + p + "/gaps.yaml`").join(", ") + ")" : "") + ".";
  return { verified, unverified, zh, en };
}
const TARGETS = [
  { file: "README.md", marker: "platform-status", lang: "zh" },
  { file: "README_EN.md", marker: "platform-status-en", lang: "en" },
  { file: "PRODUCT.md", marker: "platform-status", lang: "zh" },
  { file: "docs/vm/README.md", marker: "platform-status", lang: "zh" }
];
function render(target, st) {
  const text = target.lang === "en" ? st.en : st.zh;
  return "<!-- BEGIN:" + target.marker + " -->\n> " + text + "\n<!-- END:" + target.marker + " -->";
}
function main() {
  const write = process.argv.includes("--write");
  const st = status();
  const problems = [];
  let changed = 0;
  for (const t of TARGETS) {
    const abs = path.join(ROOT, t.file);
    if (!fs.existsSync(abs)) { problems.push("缺文件：" + t.file); continue; }
    const text = fs.readFileSync(abs, "utf8");
    const re = new RegExp("<!-- BEGIN:" + t.marker + " -->[\\s\\S]*?<!-- END:" + t.marker + " -->");
    const want = render(t, st);
    if (!re.test(text)) { problems.push(t.file + "：缺生成段标记 " + t.marker); continue; }
    const cur = text.match(re)[0];
    if (cur !== want) {
      changed++;
      if (write) fs.writeFileSync(abs, text.replace(re, want), "utf8");
      else problems.push(t.file + "：生成段与来源不一致（跑 node tests/docs-sync.mjs --write）");
    }
  }
  console.log("[docs-sync] 来源：VERIFIED_HOSTS=[" + st.verified.join(",") + "]，未验=" + (st.unverified.join(",") || "无"));
  console.log("[docs-sync] 目标 " + TARGETS.length + " 份，需更新 " + changed + " 份" + (write ? "（已写入）" : ""));
  for (const p of problems) console.log("  - " + p);
  if (problems.length) process.exit(1);
}
main();
