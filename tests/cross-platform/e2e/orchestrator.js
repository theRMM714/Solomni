#!/usr/bin/env node
/**
 * L4 端到端编排（跨平台）：清隔离根 → 起假供应商 → 起 Web → 跑断言驱动 → 收尾杀掉自己起的进程。
 * 夹具根 = 本目录下的 root/：真实 .home/ 与 session/ 全程不被触碰。
 * 成功打印固定标记 E2E-OK（见 TESTING.md 六）。
 */
"use strict";
const { spawn } = require("child_process");
const fs = require("fs");
const path = require("path");

const HERE = __dirname;
const PRODUCT_ROOT = path.dirname(path.dirname(path.dirname(HERE)));
const FIXTURE = path.join(HERE, "root");
const PORT_SRV = 3099;
const BIN = path.join(PRODUCT_ROOT, "target", "debug", process.platform === "win32" ? "solomni.exe" : "solomni");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitReady(url, tries) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return true; } catch {}
    await sleep(250);
  }
  return false;
}

function stop(child) {
  if (!child || child.exitCode !== null) return;
  try { child.kill(); } catch {}
}

async function main() {
  if (!fs.existsSync(BIN)) {
    console.error("[e2e] 找不到二进制（" + path.relative(PRODUCT_ROOT, BIN) + "）：先 cargo build");
    return 1;
  }
  // 提示词册是产品的一部分，必须用当前那份（夹具里不放副本，否则必然过期）。
  fs.copyFileSync(path.join(PRODUCT_ROOT, "prompts.yaml"), path.join(FIXTURE, "prompts.yaml"));
  // 清运行期痕迹（夹具本身不动）。
  fs.rmSync(path.join(FIXTURE, "session"), { recursive: true, force: true });
  fs.rmSync(path.join(FIXTURE, "logs"), { recursive: true, force: true });
  fs.rmSync(path.join(FIXTURE, "modules", "toolbox", "userdata"), { recursive: true, force: true });

  const mock = spawn(process.execPath, [path.join(HERE, "mock.js")], { cwd: HERE, stdio: "inherit" });
  await sleep(400);
  const srv = spawn(BIN, ["-webUI", "--web-port", String(PORT_SRV), "--root", FIXTURE], { cwd: PRODUCT_ROOT, stdio: "inherit" });
  try {
    const ready = await waitReady("http://127.0.0.1:" + PORT_SRV + "/api/state", 60);
    if (!ready) {
      console.error("[e2e] 服务端没起来（端口 " + PORT_SRV + "）");
      return 1;
    }
    const driver = spawn(process.execPath, [path.join(HERE, "driver.js")], {
      cwd: HERE,
      stdio: "inherit",
      env: Object.assign({}, process.env, { E2E_BASE: "http://127.0.0.1:" + PORT_SRV }),
    });
    const code = await new Promise((resolve) => driver.on("exit", (c) => resolve(c === null ? 1 : c)));
    if (code !== 0) {
      console.log("E2E-FAILED（驱动退出码 " + code + "）");
      return code;
    }
    console.log("E2E-OK");
    return 0;
  } finally {
    stop(mock);
    stop(srv);
    await sleep(300);
  }
}

main().then((code) => process.exit(code));
