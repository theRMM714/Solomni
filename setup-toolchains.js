#!/usr/bin/env node
/**
 * 基础工具链安装器：把 Python / Node / C-C++ 固定版本装进项目内 .tools/toolchains。
 * 管：下载、sha256 校验、解包、Zig 的 gcc/g++ 包装、写 active.json（启动期由核心读取）。
 * 不管：启动期 PATH 解析（src/entry/toolchains.rs）、guest 预装、C#、宿主 runtimes 扩展。
 * 用法：node setup-toolchains.js [--check] [--yes] [--only=python,node,cc] [--target=linux-x86_64]
 * 约束：只写项目内；下载前要用户同意；不改系统 PATH / 注册表。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const readline = require("readline");
const { spawnSync } = require("child_process");
const env = require("./env.js");
const TC = require("./toolchains.js");

const ROOT = __dirname;
const TOOLS = env.TOOLS;
const ACTIVE = path.join(TOOLS, "toolchains", "active.json");
const CACHE = path.join(TOOLS, "cache");
const IS_WIN = process.platform === "win32";

const log = (m) => console.log("[toolchains] " + m);
const bad = (m) => console.error("[toolchains] " + m);
const die = (m) => { bad(m); process.exit(1); };

function usage() {
  return [
    "用法：node setup-toolchains.js [--check] [--yes] [--only=python,node,cc] [--target=<目标>]",
    "",
    "默认 profile：" + TC.PROFILE.join(" + "),
    "目标：" + Object.keys(TC.TARGETS).join(" / "),
    "安装位置：.tools/toolchains/<目标>/<工具>/<版本>/（.tools/ 不入库）",
  ].join("\n");
}

function curlExe() {
  if (IS_WIN) {
    const p = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
    if (fs.existsSync(p)) return p;
  }
  return "curl";
}

function hashFile(file) {
  return new Promise((resolve, reject) => {
    const h = crypto.createHash("sha256");
    const s = fs.createReadStream(file);
    s.on("error", reject);
    s.on("data", (b) => h.update(b));
    s.on("end", () => resolve(h.digest("hex")));
  });
}

async function ensureVerified(url, sha256) {
  fs.mkdirSync(CACHE, { recursive: true });
  const name = path.basename(new URL(url).pathname);
  const dest = path.join(CACHE, name);
  if (fs.existsSync(dest)) {
    const got = await hashFile(dest);
    if (got.toLowerCase() === sha256.toLowerCase()) {
      log("复用已下载的 " + name);
      return dest;
    }
    log("缓存校验不符，重新下载：" + name);
    fs.rmSync(dest, { force: true });
  }
  const part = dest + ".part";
  fs.rmSync(part, { force: true });
  log("下载 " + url);
  const r = spawnSync(curlExe(), ["--fail", "--location", "--retry", "2", "--output", part, url], { stdio: "inherit" });
  if (r.error || r.status !== 0) {
    throw new Error("下载失败：" + url + (r.error ? "（" + r.error.message + "）" : ""));
  }
  const got = await hashFile(part);
  if (got.toLowerCase() !== sha256.toLowerCase()) {
    fs.rmSync(part, { force: true });
    throw new Error("sha256 不符：" + name + "\n  台账 " + sha256 + "\n  实际 " + got);
  }
  fs.renameSync(part, dest);
  return dest;
}

function extract(archive, dest) {
  fs.mkdirSync(dest, { recursive: true });
  const tar = spawnSync("tar", ["-xf", archive, "-C", dest], { stdio: "inherit" });
  if (tar.status === 0) return;
  if (IS_WIN && /\.zip$/i.test(archive)) {
    const ps = spawnSync(
      "powershell.exe",
      ["-NoProfile", "-Command", "Expand-Archive -Force -LiteralPath '" + archive.replace(/'/g, "''") + "' -DestinationPath '" + dest.replace(/'/g, "''") + "'"],
      { stdio: "inherit" },
    );
    if (ps.status === 0) return;
  }
  throw new Error("解包失败：" + archive + "（需要可用的 tar；Windows 会再试 Expand-Archive）");
}

function existingBins(dir, candidates) {
  return candidates.filter((rel) => fs.existsSync(path.join(dir, rel)) && fs.statSync(path.join(dir, rel)).isDirectory());
}

function findZig(root) {
  const stack = [root];
  while (stack.length) {
    const d = stack.pop();
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) stack.push(p);
      else if (e.name === "zig" || e.name === "zig.exe") return p;
    }
  }
  return null;
}

function writeZigWrappers(installDir) {
  const zig = findZig(installDir);
  if (!zig) throw new Error("Zig 包里找不到 zig 可执行文件：" + installDir);
  const bin = path.join(installDir, "bin");
  fs.mkdirSync(bin, { recursive: true });
  const relZig = path.relative(bin, zig).split(path.sep).join("/");
  const write = (name, body) => {
    const p = path.join(bin, name);
    fs.writeFileSync(p, body);
    fs.chmodSync(p, 0o755);
  };
  write("zig", "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$DIR/" + relZig + "\" \"$@\"\n");
  write("cc", "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$DIR/zig\" cc \"$@\"\n");
  write("gcc", "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$DIR/zig\" cc \"$@\"\n");
  write("c++", "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$DIR/zig\" c++ \"$@\"\n");
  write("g++", "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$DIR/zig\" c++ \"$@\"\n");
  write("clang", "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$DIR/zig\" cc \"$@\"\n");
  write("clang++", "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$DIR/zig\" c++ \"$@\"\n");
  return ["bin"];
}

function rel(p) {
  return path.relative(ROOT, p).split(path.sep).join("/");
}

async function installOne(id, target) {
  const spec = TC.TOOLS[id] && TC.TOOLS[id].targets[target];
  if (!spec) throw new Error("工具 " + id + " 没有目标 " + target + " 的条目");
  // Windows 的 C/C++ 复用 env.js 已经装好的 winlibs，避免同机两份 MinGW。
  if (id === "cc" && target === "windows-x86_64") {
    const ready = path.join(TOOLS, "mingw64", "bin", "g++.exe");
    if (fs.existsSync(ready)) {
      log("复用已有的 MinGW：" + rel(path.join(TOOLS, "mingw64", "bin")));
      return { id, version: spec.version, bin: [rel(path.join(TOOLS, "mingw64", "bin"))], env: {} };
    }
  }
  const dir = path.join(ROOT, TC.installRel(target, id));
  const bins = existingBins(dir, spec.bins);
  if (bins.length) {
    log("已安装 " + id + " " + TC.versionOf(id, target) + "：" + rel(dir));
    return { id, version: TC.versionOf(id, target), bin: bins.map((b) => rel(path.join(dir, b))), env: {} };
  }
  log("安装 " + id + " " + TC.versionOf(id, target) + " → " + TC.installRel(target, id));
  const archive = await ensureVerified(spec.url, spec.sha256);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  extract(archive, dir);
  let found;
  if (spec.kind === "zig") {
    found = writeZigWrappers(dir);
  } else {
    found = existingBins(dir, spec.bins);
    if (!found.length) throw new Error("解包后找不到 bin 目录：" + dir + "（期望 " + spec.bins.join(" / ") + "）");
  }
  return { id, version: TC.versionOf(id, target), bin: found.map((b) => rel(path.join(dir, b))), env: {} };
}

function readActive() {
  if (!fs.existsSync(ACTIVE)) return null;
  try {
    return JSON.parse(fs.readFileSync(ACTIVE, "utf8"));
  } catch (e) {
    return { error: e.message };
  }
}

function writeActive(target, requested, tools) {
  const dir = path.dirname(ACTIVE);
  fs.mkdirSync(dir, { recursive: true });
  const tmp = ACTIVE + ".tmp";
  fs.writeFileSync(tmp, JSON.stringify({ target, profile: requested, tools }, null, 2) + "\n");
  fs.renameSync(tmp, ACTIVE);
}

/** 目的：只读校验 profile 状态；不打印、不退出，供 CLI 与启动器共用。 */
function profileState(target, requested) {
  const active = readActive();
  if (!active) {
    return { ok: false, reason: "missing", active: null, target, installed: [], missing: requested.slice() };
  }
  if (active.error) {
    return { ok: false, reason: "invalid", why: active.error, active: null, target, installed: [], missing: requested.slice() };
  }
  if (active.target !== target) {
    return {
      ok: false,
      reason: "target",
      why: "active.json 的目标是 " + active.target + "，当前是 " + target,
      active,
      target,
      installed: [],
      missing: requested.slice(),
    };
  }
  const installed = new Map((active.tools || []).map((t) => [t.id, t]));
  const okTools = [];
  const missing = [];
  for (const id of requested) {
    const t = installed.get(id);
    if (!t) {
      missing.push(id);
      continue;
    }
    const bins = t.bin || [];
    const ok = bins.length > 0 && bins.every((b) => fs.existsSync(path.join(ROOT, b)));
    if (!ok) {
      missing.push(id);
      continue;
    }
    okTools.push({ id, version: t.version || "", bin: bins });
  }
  return { ok: missing.length === 0, reason: missing.length ? "missing" : "", active, target, installed: okTools, missing };
}

async function ask(q) {
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  const answer = await new Promise((resolve) => rl.question(q, resolve));
  rl.close();
  return answer;
}

/**
 * 目的：确保 base profile 已安装；CLI、setup-runtime、start.js 共用这一份。
 * 参数：yes = 不问直接装；target = 目标；only = 只装其中几个（缺省整个 profile）。
 * 返回：profileState 的结果 + { installedNow, failed }；不退出进程。
 */
async function ensureProfile({ yes = false, target, only } = {}) {
  const t = target || TC.hostTarget();
  if (!t || !TC.TARGETS[t]) {
    throw new Error("不支持的目标平台：" + (t || process.platform + "/" + process.arch));
  }
  const requested = only && only.length ? only.slice() : TC.PROFILE.slice();
  for (const id of requested) {
    if (!TC.TOOLS[id]) throw new Error("未知工具：" + id + "（可用：" + Object.keys(TC.TOOLS).join(" / ") + "）");
  }
  const before = profileState(t, requested);
  if (before.ok) return { ...before, installedNow: false, failed: [] };
  if (!yes) {
    if (!process.stdin.isTTY) return { ...before, ok: false, reason: "no-tty", installedNow: false, failed: [] };
    const a = await ask("把 " + requested.join(" / ") + " 装进项目内 .tools/toolchains？不写系统 [y/N] ");
    if (a.trim().toLowerCase() !== "y") {
      return { ...before, ok: false, reason: "declined", installedNow: false, failed: [] };
    }
  }
  const done = [];
  const failed = [];
  for (const id of requested) {
    try {
      done.push(await installOne(id, t));
    } catch (e) {
      const why = e && e.message ? e.message : String(e);
      failed.push({ id, why });
      bad(id + " 安装失败：" + why);
    }
  }
  if (done.length) {
    const active = readActive();
    const usable = active && !active.error && active.target === t ? active : null;
    const existing = usable ? usable.tools || [] : [];
    const byId = new Map(existing.map((x) => [x.id, x]));
    for (const d of done) byId.set(d.id, d);
    const profile = Array.from(new Set([...(usable ? usable.profile || [] : []), ...TC.PROFILE, ...requested]));
    writeActive(t, profile, Array.from(byId.values()));
    log("active.json 已写：" + rel(ACTIVE));
    for (const d of done) log("  " + d.id + " " + d.version + " → " + d.bin.join(" / "));
  }
  const after = profileState(t, requested);
  return {
    ...after,
    ok: after.ok && failed.length === 0,
    reason: after.ok && failed.length === 0 ? "" : failed.length ? "failed" : after.reason,
    installedNow: true,
    failed,
  };
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--help") || argv.includes("-h")) {
    console.log(usage());
    return;
  }
  const targetArg = argv.find((a) => a.startsWith("--target="));
  const target = targetArg ? targetArg.slice("--target=".length) : TC.hostTarget();
  if (!target || !TC.TARGETS[target]) die("不支持的目标平台：" + (target || process.platform + "/" + process.arch));
  const onlyArg = argv.find((a) => a.startsWith("--only="));
  const requested = onlyArg ? onlyArg.slice("--only=".length).split(",").map((s) => s.trim()).filter(Boolean) : TC.PROFILE.slice();
  for (const id of requested) if (!TC.TOOLS[id]) die("未知工具：" + id + "（可用：" + Object.keys(TC.TOOLS).join(" / ") + "）");
  if (argv.includes("--check")) {
    const st = profileState(target, requested);
    if (st.why) bad(st.why);
    if (!st.active) {
      bad("未安装：缺少 " + rel(ACTIVE) + "。运行 node setup-toolchains.js --yes");
    } else if (!st.ok) {
      for (const id of st.missing) bad("缺 " + id + (st.active ? "（active.json 未登记或缺 bin）" : ""));
    }
    if (!st.ok) process.exit(1);
    for (const t of st.installed) log(t.id + " " + t.version + " → " + t.bin.join(" / "));
    return;
  }
  const yes = argv.includes("--yes");
  log("目标 " + target + "；profile " + requested.join(" + "));
  if (!yes && !process.stdin.isTTY) {
    console.log(usage());
    die("无交互终端：不擅自下载。确认后重跑并加 --yes。");
  }
  const st = await ensureProfile({ yes, target, only: requested });
  if (!st.ok) {
    if (st.reason === "declined") die("已取消");
    bad("基础工具链未就绪：" + (st.missing.join("、") || st.reason));
    process.exit(1);
  }
}

if (require.main === module) main().catch((e) => die(e && e.stack ? e.stack : String(e)));

module.exports = { ensureProfile, profileState };
