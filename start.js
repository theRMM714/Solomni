#!/usr/bin/env node
/**
 * Solomni 启动层序：平台自检 → 依赖就绪（无二进制则构建）→ 运行。
 * 默认 CLI；-webUI 进 Web 转录中心。本脚本不做任何业务。
 * 跨平台（Windows/Linux/macOS）：node start.js [-webUI] [--release] [--root <目录>] [--web-port <端口>]
 */
"use strict";
const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const ROOT = __dirname;
const IS_WIN = process.platform === "win32";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const RELEASE = process.argv.includes("--release");
const PROFILE = RELEASE ? "release" : "debug";
const BIN = path.join(ROOT, "target", PROFILE, EXE);
const BUNDLED_CARGO = path.join(ROOT, "platform", "linux", "cargo", "bin", IS_WIN ? "cargo.exe" : "cargo");

const log = (m) => console.log("[启动] " + m);

function cargoEnv(cargo) {
  // 内置工具链需要显式 HOME；系统 cargo 原样。
  const env = Object.assign({}, process.env);
  if (path.resolve(cargo) === path.resolve(BUNDLED_CARGO)) {
    env.RUSTUP_HOME = path.join(ROOT, "platform", "linux", "rustup");
    env.CARGO_HOME = path.join(ROOT, "platform", "linux", "cargo");
  }
  env.PATH = path.dirname(cargo) + path.delimiter + (env.PATH || "");
  return env;
}

function findCargo() {
  // 优先仓库内置工具链，其次 PATH（Windows 开发机常见 rustup 全局安装）。
  if (fs.existsSync(BUNDLED_CARGO)) return BUNDLED_CARGO;
  for (const d of (process.env.PATH || "").split(path.delimiter)) {
    const c = path.join(d, IS_WIN ? "cargo.exe" : "cargo");
    if (fs.existsSync(c)) return c;
  }
  console.error("[启动] 未找到 cargo：请安装 Rust（https://rustup.rs）");
  process.exit(1);
}

function ensureReady(cargo) {
  log("平台 " + process.platform + " " + process.arch);
  const v = spawnSync(cargo, ["--version"], { cwd: ROOT, env: cargoEnv(cargo), encoding: "utf8" });
  if (v.error || v.status !== 0) { console.error("[启动] cargo 不可用：" + (v.error && v.error.message)); process.exit(1); }
  log(v.stdout.trim());
  if (!fs.existsSync(BIN)) {
    log("未发现二进制，开始构建（首次较慢）…");
    const args = RELEASE ? ["build", "--release"] : ["build"];
    const b = spawnSync(cargo, args, { cwd: ROOT, env: cargoEnv(cargo), stdio: "inherit" });
    if (b.status !== 0) { console.error("[启动] 构建失败"); process.exit(b.status || 1); }
  } else {
    log("已有二进制：" + path.relative(ROOT, BIN));
  }
}

function run() {
  const pass = process.argv.slice(2).filter((a) => a !== "--release");
  const argv = [BIN, "."].concat(pass);
  log(process.argv.includes("-webUI")
    ? "启动 Web 转录中心（默认 127.0.0.1:3081，浏览器打开 http://127.0.0.1:3081）"
    : "启动 CLI（-webUI 可进 Web 转录中心）");
  const r = spawnSync(argv[0], argv.slice(1), { cwd: ROOT, stdio: "inherit" });
  if (r.error) { console.error("[启动] 运行失败：" + r.error.message); process.exit(1); }
  process.exitCode = r.status || 0;
}

ensureReady(findCargo());
run();
