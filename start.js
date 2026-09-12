#!/usr/bin/env node
/**
 * Solomni launcher: platform check -> toolchain/build readiness -> run.
 * Default CLI; -webUI starts the Web UI. No business logic here.
 * Cross-platform: node start.js [-webUI] [--release] [--root <dir>] [--web-port <port>]
 * Rust missing? Ask, then install via rustup (GNU toolchain on Windows: no MSVC needed).
 */
"use strict";
const { spawnSync } = require("child_process");
const readline = require("readline");
const path = require("path");
const fs = require("fs");

const ROOT = __dirname;
const IS_WIN = process.platform === "win32";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const RELEASE = process.argv.includes("--release");
const PROFILE = RELEASE ? "release" : "debug";
const BIN = path.join(ROOT, "target", PROFILE, EXE);
const BUNDLED_CARGO = path.join(ROOT, "platform", "linux", "cargo", "bin", IS_WIN ? "cargo.exe" : "cargo");

const log = (m) => console.log("[start] " + m);
const die = (m) => { console.error("[start] " + m); process.exit(1); };

function cargoEnv(cargo) {
  // bundled toolchain needs explicit HOME; system cargo used as-is.
  const env = Object.assign({}, process.env);
  if (path.resolve(cargo) === path.resolve(BUNDLED_CARGO)) {
    env.RUSTUP_HOME = path.join(ROOT, "platform", "linux", "rustup");
    env.CARGO_HOME = path.join(ROOT, "platform", "linux", "cargo");
  }
  env.PATH = path.dirname(cargo) + path.delimiter + (env.PATH || "");
  return env;
}

function findCargo() {
  // bundled toolchain first, then PATH; also probe ~/.cargo/bin (PATH not refreshed).
  if (fs.existsSync(BUNDLED_CARGO)) return BUNDLED_CARGO;
  const exe = IS_WIN ? "cargo.exe" : "cargo";
  for (const d of (process.env.PATH || "").split(path.delimiter)) {
    const c = path.join(d, exe);
    if (fs.existsSync(c)) return c;
  }
  const home = process.env.USERPROFILE || process.env.HOME || "";
  const ru = path.join(home, ".cargo", "bin", exe);
  if (fs.existsSync(ru)) return ru;
  return null;
}

function ask(question) {
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(question, (a) => { rl.close(); resolve(a); });
  });
}

async function installRust() {
  log("Rust toolchain not found (required to build Solomni).");
  const ans = await ask("Download and install Rust now via rustup? [y/N] ");
  if (ans.trim().toLowerCase() !== "y") {
    die("aborted. Install Rust manually: https://rustup.rs then re-run.");
  }
  log("installing rustup (downloads a few hundred MB, once)...");
  if (IS_WIN) {
    // GNU toolchain: self-contained linker, no Visual Studio Build Tools needed.
    const dl = spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command",
      "Invoke-WebRequest -UseBasicParsing https://win.rustup.rs/x86_64 -OutFile $env:TEMP\\rustup-init.exe"],
      { stdio: "inherit" });
    if (dl.status !== 0) die("rustup download failed. Check network / proxy.");
    const init = spawnSync(path.join(process.env.TEMP || "", "rustup-init.exe"),
      ["-y", "--default-toolchain", "stable-x86_64-pc-windows-gnu"],
      { stdio: "inherit" });
    if (init.status !== 0) die("rustup install failed.");
  } else {
    const r = spawnSync("sh", ["-c", "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"],
      { stdio: "inherit" });
    if (r.status !== 0) die("rustup install failed. Check network / curl availability.");
  }
  const cargo = findCargo();
  if (!cargo) die("installed, but cargo still not visible. Reopen the terminal and re-run.");
  log("Rust installed: " + cargo);
  return cargo;
}

function ensureReady(cargo) {
  log("platform " + process.platform + " " + process.arch);
  const v = spawnSync(cargo, ["--version"], { cwd: ROOT, env: cargoEnv(cargo), encoding: "utf8" });
  if (v.error || v.status !== 0) die("cargo not runnable: " + (v.error && v.error.message));
  log(v.stdout.trim());
  if (!fs.existsSync(BIN)) {
    log("binary not found, building (first run is slow)...");
    const args = RELEASE ? ["build", "--release"] : ["build"];
    const b = spawnSync(cargo, args, { cwd: ROOT, env: cargoEnv(cargo), stdio: "inherit" });
    if (b.status !== 0) die("build failed");
  } else {
    log("using binary: " + path.relative(ROOT, BIN));
  }
}

function run(cargo) {
  const pass = process.argv.slice(2).filter((a) => a !== "--release");
  const argv = [BIN, "."].concat(pass);
  log(process.argv.includes("-webUI")
    ? "starting Web UI (default 127.0.0.1:3081, open http://127.0.0.1:3081)"
    : "starting CLI (use -webUI for the Web UI)");
  const r = spawnSync(argv[0], argv.slice(1), { cwd: ROOT, stdio: "inherit", env: cargoEnv(cargo) });
  if (r.error) die("run failed: " + r.error.message);
  process.exitCode = r.status || 0;
}

(async () => {
  let cargo = findCargo();
  if (!cargo) cargo = await installRust();
  ensureReady(cargo);
  run(cargo);
})();
