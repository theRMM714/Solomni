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

/** 本机可用的解释器名（探不到也照写，让工具失败时如实报错，而不是静默跳过）。 */
function interpreter() {
  const { spawnSync } = require("child_process");
  for (const name of ["python", "python3"]) {
    const r = spawnSync(name, ["-c", "print(1)"], { stdio: "ignore" });
    if (!r.error && r.status === 0) return name;
  }
  return "python";
}

/** 夹具模块 toolbox 的清单：工具的启动命令在这里定（夹具模块源码在 root/modules 下，运行期产物不入库）。 */
function writeToolbox(py) {
  const dir = path.join(FIXTURE, "modules", "toolbox");
  fs.mkdirSync(path.join(dir, "tools"), { recursive: true });
  const text = [
    "id: toolbox",
    "brief: e2e 夹具：声明一个外部工具，用来验证工具进程的工作目录是它自己的模块目录。",
    "system: >-",
    "  你负责读文本。工具用法（参数以 JSON 对象经信封 args 传入）：",
    "  - read_txt：读文本并带行号输出，参数 {\"path\":\"…\"}（相对路径以本模块目录为基准）。",
    "  读用户投喂的材料请优先用内置 read（路径用 agent 提示词里列出的真实根目录）。",
    "tools:",
    "  read_txt: " + py + " tools/read_txt.py",
    "",
  ].join("\n");
  fs.writeFileSync(path.join(dir, "module.yaml"), text);
}

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
  // 工具命令行按平台生成：Linux / macOS 上解释器通常叫 python3，Windows 上叫 python（写死一个必然在另一个平台挂）。
  writeToolbox(interpreter());
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
