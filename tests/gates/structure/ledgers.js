/**
 * 结构审查·账本：缺口账格式、外部连接台账（links.yaml）、依赖清单校验值格式。
 * 目的：这些是"某份清单守护另一处事实"的段落，人一定会忘记同步，所以只能机器比对。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { linkValueProblem } = require("../../lib/parsers.js");

const IS_WIN = process.platform === "win32";
const OS_KEY = IS_WIN ? "windows" : process.platform === "darwin" ? "macos" : "linux";

/** 缺口账文件清单：唯一的四处账本（平台层 + 跨平台层 + 全局）。 */
function gapFiles(root) {
  return [
    path.join(root, "tests", "gaps.yaml"),
    path.join(root, "tests", "cross-platform", "gaps.yaml"),
    ...(IS_WIN ? ["windows", OS_KEY, "linux", "macos"] : [OS_KEY, "windows", "linux", "macos"]).map((p) => path.join(root, "tests", p, "gaps.yaml")),
  ];
}

/** 每条缺口必须有 scope/level/why/how/accept/blocked_by（缺一个就无法验收）。 */
function gapFormat(root) {
  const problems = [];
  for (const f of gapFiles(root)) {
    if (!fs.existsSync(f)) { problems.push("缺缺口账：" + rel(root, f)); continue; }
    const blocks = fs.readFileSync(f, "utf8").split(/^\s*-\s*id:/m).slice(1);
    blocks.forEach((b, i) => {
      for (const key of ["scope", "level", "why", "how", "accept", "blocked_by"]) {
        if (!new RegExp("^\\s+" + key + ":", "m").test(b)) problems.push(rel(root, f) + " 第 " + (i + 1) + " 条缺口缺 " + key);
      }
    });
  }
  return problems;
}

/** 外部连接台账：值必须 https、域在白名单、键不重复、校验值与链接成对、键必须有人消费（死数据也算违例）。 */
function links(root) {
  const problems = [];
  const linksFile = path.join(root, "links.yaml");
  const linkDomains = ["rustup", "winlibs", "release", "guest", "smbd", "toolchain"];
  if (!fs.existsSync(linksFile)) {
    problems.push("缺外部连接台账：links.yaml");
    return problems;
  }
  const declared = new Map();
  for (const raw of fs.readFileSync(linksFile, "utf8").split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const m = line.match(/^([A-Za-z0-9_.-]+):\s*(\S+)\s*$/);
    if (!m) { problems.push("links.yaml 不是扁平的 键: 值：" + raw); continue; }
    const key = m[1];
    const bad = linkValueProblem(key, m[2]);
    if (bad) problems.push(bad);
    if (!linkDomains.includes(key.split(".")[0])) problems.push("links.yaml 的域不在白名单（" + linkDomains.join(" / ") + "）：" + key);
    if (declared.has(key)) problems.push("links.yaml 重复键：" + key);
    declared.set(key, m[2]);
  }
  // 校验值必须与真实存在的链接成对：写成孤儿键 = 它守护的那条连接根本不存在。
  for (const [k] of declared) {
    if (!k.endsWith(".sha256")) continue;
    const base = k.slice(0, -".sha256".length);
    if (!declared.has(base) || !/^https:\/\//.test(declared.get(base) || "")) {
      problems.push("links.yaml 的校验值没有对应的链接键：" + k + "（应为 " + base + " 的 .sha256）");
    }
  }
  const used = new Set();
  const scanLinkUses = (abs) => {
    if (!fs.existsSync(abs)) return;
    for (const m of fs.readFileSync(abs, "utf8").matchAll(/\blink\("([A-Za-z0-9_.-]+)"\)/g)) used.add(m[1]);
  };
  // 消费点：环境层（rustup 安装）与依赖清单的 source（产品侧下载源都在 dependencies.yaml）。
  scanLinkUses(path.join(root, "env.js"));
  scanLinkUses(path.join(root, "toolchains.js"));
  const depsFile = path.join(root, "dependencies.yaml");
  if (fs.existsSync(depsFile)) {
    for (const m of fs.readFileSync(depsFile, "utf8").matchAll(/^\s*source:\s*([A-Za-z0-9_.:/-]+)\s*$/gm)) {
      if (!m[1].includes(":")) used.add(m[1]);
    }
  }
  const walkSrc = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkSrc(p);
      else if (e.name.endsWith(".rs")) scanLinkUses(p);
    }
  };
  const srcDir = path.join(root, "src");
  if (fs.existsSync(srcDir)) walkSrc(srcDir);
  // 校验值跟着它守护的那条链接一起被消费：代码按 源名 加 .sha256 取，所以"基础链接有人用"就等于验过。
  const consumed = new Set(used);
  for (const k of declared.keys()) {
    if (!k.endsWith(".sha256")) continue;
    if (used.has(k.slice(0, -".sha256".length))) consumed.add(k);
  }
  for (const k of used) if (!declared.has(k)) problems.push("links.yaml 缺被引用的键：" + k);
  for (const k of declared.keys()) if (!consumed.has(k)) problems.push("links.yaml 的键没有消费点（死数据）：" + k);
  return problems;
}

/** 依赖清单的校验值：写了就必须是 64 位十六进制（与 links.yaml 同一口径）；不写 = 没 pin。 */
function depsSha256(root) {
  const problems = [];
  const depsLedger = path.join(root, "dependencies.yaml");
  if (!fs.existsSync(depsLedger)) return problems;
  fs.readFileSync(depsLedger, "utf8").split(/\r?\n/).forEach((line, i) => {
    const m = line.match(/^\s+sha256:\s*(\S+)\s*$/);
    if (m && !/^[a-f0-9]{64}$/i.test(m[1])) problems.push("dependencies.yaml:" + (i + 1) + " 的 sha256 必须是 64 位十六进制");
  });
  return problems;
}

module.exports = { gapFiles, gapFormat, links, depsSha256 };
