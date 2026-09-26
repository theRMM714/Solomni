#!/usr/bin/env node
/**
 * L4 端到端编排（跨平台）：清隔离根 → 起假供应商 → 起 Web → 跑断言驱动 → 收尾杀掉自己起的进程。
 * 夹具根 = 本目录下的 root/：真实 .home/ 与 session/ 全程不被触碰。
 * 成功打印固定标记 E2E-OK（见 docs/testing/execution-ci.md）。
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
    if (!r.error && r.status === 0) {
      // 围栏是按「解释器所在目录」放行的，所以这里如实报出：用哪个名字、真身在哪、安装根在哪。
      // 失败时（例如动态库取不到导致 SIGABRT）这行就是定位依据。
      const which = spawnSync(process.platform === "win32" ? "where" : "which", [name], { encoding: "utf8" });
      const real = (which.stdout || "").trim().split("\n")[0] || "(取不到路径)";
      console.log("[e2e] 夹具解释器：" + name + " → " + real);
      return name;
    }
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
    "  read_txt:",
    "    command: " + py + " tools/read_txt.py",
    "    desc: 读文本并带行号输出",
    "    params:",
    "      path:",
    "        type: string",
    "        required: true",
    "        desc: 要读取的文件路径（相对路径以本模块目录为基准）",
    "",
  ].join("\n");
  fs.writeFileSync(path.join(dir, "module.yaml"), text);
}


/** 把仓库里三个真模块（harvest=python / render=node / indexer=C++）复制进夹具。
 * 只复制源码与清单，不复制 build/ 与 userdata/（产物与私有状态不入夹具）。 */
function copyRealModules() {
  for (const id of ["harvest", "render", "indexer"]) {
    const from = path.join(PRODUCT_ROOT, "modules", id);
    const to = path.join(FIXTURE, "modules", id);
    if (!fs.existsSync(from)) {
      console.error("[e2e] 找不到真模块：" + path.relative(PRODUCT_ROOT, from));
      return;
    }
    fs.rmSync(to, { recursive: true, force: true });
    fs.cpSync(from, to, {
      recursive: true,
      filter: (src) => {
        const rel = path.relative(from, src);
        if (!rel) return true;
        // 构建产物与模块私有区不入夹具：前者由本脚本现编，后者是跨任务私有状态。
        return !rel.split(path.sep).includes("build") && !rel.split(path.sep).includes("userdata");
      },
    });
  }
  console.log("[e2e] 真模块已复制进夹具：harvest / render / indexer");
}

/** 构建 indexer（C++17，单文件，零第三方依赖）。返回是否成功。
 * 编译器：优先环境变量 SOLOMNI_CXX，其次 c++ / g++ / clang++（Windows 上项目自带 .tools/mingw64）。
 * 与 modules/indexer/README.md 同一条命令——本机与 CI 编的是同一份源码，链接方式按平台：
 * Windows 走静态（不带编译器也能在别的机器上跑），Unix 走动态（系统自带运行时）。 */
function buildIndexer() {
  const { spawnSync } = require("child_process");
  const src = path.join(FIXTURE, "modules", "indexer", "tools", "src", "indexer.cpp");
  const outDir = path.join(FIXTURE, "modules", "indexer", "build");
  const out = path.join(outDir, process.platform === "win32" ? "indexer.exe" : "indexer");
  fs.mkdirSync(outDir, { recursive: true });
  // 候选编译器：项目自带工具链（本地）→ PATH 上的 g++ / c++ → Windows 上常见的两处安装位置。
  // 顺序即优先级；每个都先打 --version 进日志，再真编——哪个用上了、什么版本，日志里一眼可见。
  const winCandidates = [
    path.join(PRODUCT_ROOT, ".tools", "mingw64", "bin", "g++.exe"),
    "g++",
    "c++",
    path.join("C:", "msys64", "mingw64", "bin", "g++.exe"),
    path.join("C:", "Program Files", "LLVM", "bin", "clang++.exe"),
  ];
  const candidates = process.env.SOLOMNI_CXX
    ? [process.env.SOLOMNI_CXX]
    : (process.platform === "win32" ? winCandidates : ["c++", "g++", "clang++"]);
  const flags = ["-O2", "-std=c++17", "-Wall", "-Wextra"];
  if (process.platform === "win32") flags.push("-static", "-static-libgcc", "-static-libstdc++");
  // stdio: 'inherit'——不抓子进程的管道输出。受限沙箱里 Node 的管道捕获会 EPERM（这是环境边界，
  // 不是编译器的问题），而编译器版本本来就该进日志：直接让它打进同一个流，两件事一起解决。
  for (const cxx of candidates) {
    console.log("[e2e] 试编译器：" + cxx);
    const version = spawnSync(cxx, ["--version"], { stdio: "inherit" });
    if (version.error || version.status !== 0) continue;
    const r = spawnSync(cxx, flags.concat([src, "-o", out]), { stdio: "inherit" });
    if (r.status === 0) {
      console.log("[e2e] indexer 就绪：" + path.relative(FIXTURE, out) + "（" + fs.statSync(out).size + " 字节）");
      return true;
    }
  }
  console.error("[e2e] 找不到可用的 C++ 编译器（试过：" + candidates.join("、") + "）——indexer 构建是硬失败，不跳过");
  return false;
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
  // 册子是**目录**（按角色分文件）：整份复制过去，夹具里不留第二份真相。
  fs.cpSync(path.join(PRODUCT_ROOT, "prompts"), path.join(FIXTURE, "prompts"), {
    recursive: true,
    force: true,
  });
  // 工具总表也是产品的一部分（工具是什么的唯一真相），同样整份复制。
  fs.cpSync(path.join(PRODUCT_ROOT, "systools"), path.join(FIXTURE, "systools"), {
    recursive: true,
    force: true,
  });
  // 工具命令行按平台生成：Linux / macOS 上解释器通常叫 python3，Windows 上叫 python（写死一个必然在另一个平台挂）。
  writeToolbox(interpreter());
  // 清运行期痕迹（夹具本身不动）。
  fs.rmSync(path.join(FIXTURE, "session"), { recursive: true, force: true });
  fs.rmSync(path.join(FIXTURE, "logs"), { recursive: true, force: true });
  // 登记处也是运行期状态（驱动每次用 API 重新登记）：必须清空，
  // 否则上一次跑出来的结论会漏进这一次（例如原生探测把 tools 写回 native，后面的信封场景就全变了）。
  const home = path.join(FIXTURE, ".home");
  fs.rmSync(home, { recursive: true, force: true });
  fs.mkdirSync(home, { recursive: true });
  fs.rmSync(path.join(FIXTURE, "modules", "toolbox", "userdata"), { recursive: true, force: true });

  // 三个真模块（python / node / C++）按需复制进夹具：**每次跑都从仓库源码重新复制**，
  // 所以不存在"副本与真模块漂移"（副本是生成的，不是入库的第二个真相）。
  // 为什么复制而不是指过去：夹具根要有自己的模块目录（夹具模块与真模块共存），
  // 而且工具产物（编译出来的 indexer）应该落在夹具里，不污染仓库。
  copyRealModules();
  // indexer 是编译出来的：真工具链路要用它，所以这里**必须**构建成功（硬失败，不 env-skip）。
  // 与 modules/indexer/README.md 同一条命令；编译器版本打进日志，出问题时本地复现方便。
  if (!buildIndexer()) {
    return 1;
  }

  // 假供应商端口按当前进程号取：本机可能残留上一次跑掉的进程占着固定端口，
  // 那样新进程起不来（EADDRINUSE），而驱动仍会打到旧的那个——症状是断言莫名其妙地按旧逻辑走。
  const mockPort = 8397 + (process.pid % 200);
  const mock = spawn(process.execPath, [path.join(HERE, "mock.js")], {
    cwd: HERE,
    stdio: "inherit",
    env: Object.assign({}, process.env, { E2E_MOCK_PORT: String(mockPort) }),
  });
  // 等它真的起来（连不上就一直等，等到超时再如实报错——不靠 sleep 猜）。
  if (!(await waitReady("http://127.0.0.1:" + mockPort + "/models", 60))) {
    console.error("[e2e] 假供应商没起来（端口 " + mockPort + "）：可能有残留进程占着端口");
    stop(mock);
    return 1;
  }
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
      env: Object.assign({}, process.env, {
        E2E_BASE: "http://127.0.0.1:" + PORT_SRV,
        E2E_MOCK_BASE: "http://127.0.0.1:" + mockPort,
      }),
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
    // 收尾回收：这次真机跑（--fence-live）写过目录 ACL、建过容器 profile，必须按台账撤干净，
    // 再按名字前缀扫掉整族遗留 profile。CI 机器是一次性的，但本地跑同样不许留痕。
    // 回收失败只如实打印，不改判 e2e 的结论（结论由驱动断言决定）。
    const cleaned = require("child_process").spawnSync(BIN, ["--fence-clean", "--root", FIXTURE], {
      cwd: PRODUCT_ROOT,
      encoding: "utf8",
    });
    const say = ((cleaned.stdout || "") + (cleaned.stderr || "")).trim().replace(/\r?\n/g, " | ");
    console.log("[e2e] 围栏回收（退出码 " + cleaned.status + "）：" + say);
  }
}

main().then((code) => process.exit(code));
