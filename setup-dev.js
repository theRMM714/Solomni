#!/usr/bin/env node
/**
 * 开发环境安装 / 同步：把开发与门禁需要的东西装到**项目内**（platform/<os>/ 与 .tools/）。
 * 管：项目内 Rust 工具链、Rust 组件、交叉 target、供应链工具；--check 只读校验、--offline 不下载。
 * 不管：运行环境（setup-runtime.js）；条目清单（dev-tools.js，唯一真相）。
 * 用法：node setup-dev.js [--check] [--yes] [--offline]
 * 约束：只写项目内（RUSTUP_HOME / CARGO_HOME / .tools）；不碰系统 ~/.rustup、~/.cargo。
 */
"use strict";
const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const env = require("./env.js");
const DEV = require("./dev-tools.js");

const ROOT = __dirname;
const CHECK = process.argv.includes("--check");
const YES = process.argv.includes("--yes");
const OFFLINE = process.argv.includes("--offline");
const TOOLS_ROOT = path.join(ROOT, ".tools");

const log = (m) => console.log("[setup-dev] " + m);
const bad = (m) => console.error("[setup-dev] " + m);
const die = (m) => { bad(m); process.exit(1); };

/** 跑一条命令（继承输出）。 */
function run(cmd, args, e) {
  const r = spawnSync(cmd, args, { cwd: ROOT, env: e, stdio: "inherit" });
  if (r.error) die(cmd + " 无法执行：" + r.error.message);
  return r.status === 0;
}
/** 跑一条命令并收输出（只读探测用）。 */
function probe(cmd, args, e) {
  const r = spawnSync(cmd, args, { cwd: ROOT, env: e, encoding: "utf8" });
  return { ok: !r.error && r.status === 0, text: ((r.stdout || "") + (r.stderr || "")).trim() };
}
function lines(text) {
  return text.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
}
function installedComponents(e) {
  return new Set(lines(probe("rustup", ["component", "list", "--installed"], e).text));
}
function componentPresent(set, name) {
  return [...set].some((line) => line === name || line.startsWith(name + "-"));
}
function installedTargets(e) {
  return new Set(lines(probe("rustup", ["target", "list", "--installed"], e).text));
}
function crateBin(name) {
  return path.join(TOOLS_ROOT, "bin", process.platform === "win32" ? name + ".exe" : name);
}
function probeRunnable(name, e) {
  const candidates = name === "python" ? ["python", "python3"] : [name];
  for (const c of candidates) {
    const args = name === "python" ? ["-c", "print(1)"] : ["--version"];
    if (probe(c, args, e).ok) return c;
  }
  return null;
}

(async () => {
  if (CHECK && !env.findProjectCargo()) {
    bad("项目内工具链缺失：platform/<os>/cargo/bin/cargo 不在。跑 node setup-dev.js 装进项目内。");
    process.exit(1);
  }
  const ready = CHECK ? env.resolve({ requireProject: true }) : await env.ensureProject({ yes: YES });
  if (!ready) die("项目内工具链不可用（platform/<os>/）：先 node setup-dev.js --yes");
  const e = ready.env;
  log("项目内工具链：" + ready.cargo);

  const comps = installedComponents(e);
  const targets = installedTargets(e);
  const missing = [];
  for (const c of DEV.components) {
    if (!componentPresent(comps, c.name)) missing.push({ kind: "component", id: c.name, why: c.why });
  }
  for (const t of DEV.targets) {
    if (!targets.has(t.triple)) missing.push({ kind: "target", id: t.triple, why: t.why });
  }
  for (const c of DEV.crates) {
    if (!fs.existsSync(crateBin(c.name))) missing.push({ kind: "crate", id: c.name, why: c.why });
  }
  for (const p of DEV.probe) {
    const found = probeRunnable(p.name, e);
    log((found ? "探测到 " : "缺少（如实提示，相关测试会 env-skip）：") + p.name + (found ? " → " + found : "") + "（" + p.why + "）");
  }

  if (!missing.length) {
    log("开发环境齐备（组件 / target / 工具都在项目内）。");
    process.exit(0);
  }
  log("缺 " + missing.length + " 项：");
  for (const m of missing) log("  - " + m.kind + " " + m.id + "（" + m.why + "）");

  if (CHECK) {
    bad("开发环境不齐：跑 node setup-dev.js 补齐（或加 --yes 免询问）。");
    process.exit(1);
  }
  if (OFFLINE) {
    bad("--offline：缺项需要下载，按上面清单手动装（都落项目内）。");
    process.exit(1);
  }
  if (!YES && !process.stdin.isTTY) {
    die("无交互终端：不擅自下载。重跑加 --yes，或按上面清单手动装。");
  }

  for (const m of missing) {
    log("装 " + m.kind + " " + m.id + " ...");
    const ok =
      m.kind === "component" ? run("rustup", ["component", "add", m.id], e)
      : m.kind === "target" ? run("rustup", ["target", "add", m.id], e)
      : run("cargo", ["install", "--locked", "--root", TOOLS_ROOT, m.id], e);
    if (!ok) die("装 " + m.id + " 失败（可单独重跑，项目内不改系统）。");
  }
  log("开发环境同步完成。");
})();
