#!/usr/bin/env node
/**
 * Solomni 启动器：环境就绪 -> 构建 -> 运行（默认 CLI；-webUI 进 Web）。业务逻辑不在这里。
 * 环境（工具链在哪、环境怎么拼、缺了怎么装）全部交给 env.js —— 本文件只做编排。
 * 跨平台：node start.js [-webUI] [--release] [--root <dir>] [--web-port <port>]；-test 转测试入口。
 */
"use strict";
const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");
const env = require("./env.js");

const ROOT = __dirname;
const IS_WIN = process.platform === "win32";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const RELEASE = process.argv.includes("--release");
const PROFILE = RELEASE ? "release" : "debug";
const BIN = path.join(ROOT, "target", PROFILE, EXE);

const log = (m) => console.log("[start] " + m);
const die = (m) => { console.error("[start] " + m); process.exit(1); };

function binLocked(p) {
  // Windows 不允许打开正在运行的可执行文件写：这个探针判断是否还有另一个实例占着二进制。
  // POSIX 没有这种锁，直接报 false。
  if (!fs.existsSync(p)) return false;
  try {
    const fd = fs.openSync(p, "r+");
    fs.closeSync(fd);
    return false;
  } catch (e) {
    return true;
  }
}

function runTests(cargoEnv) {
  // 测试总入口（见 TESTING.md）：环境已备好，交给 run-tests.js 逐层跑。
  log("running the test battery (node run-tests.js)");
  const r = spawnSync(process.execPath, [path.join(ROOT, "run-tests.js")], {
    cwd: ROOT,
    stdio: "inherit",
    env: cargoEnv,
  });
  if (r.error) die("tests failed to start: " + r.error.message);
  process.exitCode = r.status === null ? 1 : r.status;
}

function run(cargoEnv) {
  const pass = process.argv.slice(2).filter((a) => a !== "--release");
  const argv = [BIN, "."].concat(pass);
  log(process.argv.includes("-webUI")
    ? "starting Web UI (default 127.0.0.1:3081, open http://127.0.0.1:3081)"
    : "starting CLI (type webui at the prompt for the Web UI, or start with -webUI)");
  const r = spawnSync(argv[0], argv.slice(1), { cwd: ROOT, stdio: "inherit", env: cargoEnv });
  if (r.error) die("run failed: " + r.error.message);
  process.exitCode = r.status || 0;
}

(async () => {
  const ready = await env.ensure();
  log("platform " + process.platform + " " + process.arch);
  const cargo = ready.cargo;
  const cargoEnv = ready.env;
  env.pinTemp(cargoEnv); // 临时文件不出项目
  const v = spawnSync(cargo, ["--version"], { cwd: ROOT, env: cargoEnv, encoding: "utf8" });
  if (v.error || v.status !== 0) die("cargo not runnable: " + (v.error && v.error.message));
  log(v.stdout.trim());
  if (process.argv.includes("-test")) {
    runTests(cargoEnv);
    return;
  }
  // 始终交给 cargo 判断哪些是陈旧的（约一秒）；"已有二进制就跳过构建"会让改了源码后仍跑旧二进制。
  if (binLocked(BIN)) {
    log("binary is held by a running Solomni instance - cannot rebuild, starting it as-is.");
    log("close the other window/session to pick up source changes.");
    run(cargoEnv);
    return;
  }
  log(fs.existsSync(BIN) ? "checking build..." : "binary not found, building (first run is slow)...");
  const args = RELEASE ? ["build", "--release"] : ["build"];
  const b = spawnSync(cargo, args, { cwd: ROOT, env: cargoEnv, stdio: "inherit" });
  if (b.status !== 0) die("build failed (is solomni running in another window? close it and retry)");

  // 构建可能替换了锁探针刚检查过的文件；用之前再确认一次。
  if (!fs.existsSync(BIN)) die("build reported success but no binary at " + path.relative(ROOT, BIN));
  run(cargoEnv);
})();
