#!/usr/bin/env node
/**
 * Solomni launcher: platform check -> toolchain/build readiness -> run.
 * Default CLI; -webUI starts the Web UI. No business logic here.
 * Cross-platform: node start.js [-webUI] [--release] [--root <dir>] [--web-port <port>]
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

const log = (m) => console.log("[start] " + m);

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
  // bundled toolchain first, then PATH (common rustup global install on Windows).
  if (fs.existsSync(BUNDLED_CARGO)) return BUNDLED_CARGO;
  for (const d of (process.env.PATH || "").split(path.delimiter)) {
    const c = path.join(d, IS_WIN ? "cargo.exe" : "cargo");
    if (fs.existsSync(c)) return c;
  }
  console.error("[start] cargo not found. Install Rust: https://rustup.rs");
  process.exit(1);
}

function ensureReady(cargo) {
  log("platform " + process.platform + " " + process.arch);
  const v = spawnSync(cargo, ["--version"], { cwd: ROOT, env: cargoEnv(cargo), encoding: "utf8" });
  if (v.error || v.status !== 0) { console.error("[start] cargo not runnable: " + (v.error && v.error.message)); process.exit(1); }
  log(v.stdout.trim());
  if (!fs.existsSync(BIN)) {
    log("binary not found, building (first run is slow)...");
    const args = RELEASE ? ["build", "--release"] : ["build"];
    const b = spawnSync(cargo, args, { cwd: ROOT, env: cargoEnv(cargo), stdio: "inherit" });
    if (b.status !== 0) { console.error("[start] build failed"); process.exit(b.status || 1); }
  } else {
    log("using binary: " + path.relative(ROOT, BIN));
  }
}

function run() {
  const pass = process.argv.slice(2).filter((a) => a !== "--release");
  const argv = [BIN, "."].concat(pass);
  log(process.argv.includes("-webUI")
    ? "starting Web UI (default 127.0.0.1:3081, open http://127.0.0.1:3081)"
    : "starting CLI (use -webUI for the Web UI)");
  const r = spawnSync(argv[0], argv.slice(1), { cwd: ROOT, stdio: "inherit" });
  if (r.error) { console.error("[start] run failed: " + r.error.message); process.exit(1); }
  process.exitCode = r.status || 0;
}

ensureReady(findCargo());
run();
