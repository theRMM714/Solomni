#!/usr/bin/env node
/**
 * Solomni 环境层：路径约定（platform/<os>、.tools/）、工具链探测、环境拼装与（征得同意的）项目内安装。
 * 管：工具链在哪、环境怎么拼、缺了怎么补；不管：构建、运行、测试编排（那些在 start.js / run-tests.js）。
 * 为什么单独一层：环境是“换机器 / 换开发环境”时唯一要改的地方；启动器与测试入口共用同一份解析，不再各写一遍。
 * 收敛规则：缓存与产物一律落在项目内 platform/<os>/ 与 .tools/（AGENTS.md）；系统工具链可只读借用，但不写它的 home。
 * CLI：node env.js（看状态）/ node env.js setup [--yes]（只建环境）/ node env.js --print-env（机器可读）。
 */
"use strict";
const { spawnSync, spawn } = require("child_process");
const readline = require("readline");
const path = require("path");
const fs = require("fs");

const ROOT = __dirname;
const IS_WIN = process.platform === "win32";
// 平台目录按 OS 区分（windows / macos / linux）；不再让 macOS 借用 linux。
const OS_KEY = IS_WIN ? "windows" : process.platform === "darwin" ? "macos" : "linux";
const PLATFORM_DIR = path.join(ROOT, "platform", OS_KEY);
const P_RUSTUP = path.join(PLATFORM_DIR, "rustup");
const P_CARGO = path.join(PLATFORM_DIR, "cargo");
const TOOLS = path.join(ROOT, ".tools");
const WINLIBS_BIN = path.join(TOOLS, "mingw64", "bin");
// 第三方再分发的发布源（winlibs zip）；占位仓库，发布后填实。
const REL_BASE = "https://github.com/theRMM714/Solomni/releases/download/dependencies/";
// winlibs.zip 的 SHA256；空 = 首次下载打印哈希供固定。
const WINLIBS_SHA256 = "c1f52294597c0b73786b2a78eb5d176d89226d2f21875eab75e783a8b1cefcc4";

const log = (m) => console.log("[env] " + m);
const die = (m) => { console.error("[env] " + m); process.exit(1); };
// 预检找到的 dlltool 目录；由 ensure() 填，resolve() 拼进 PATH。
let EXTRA_PATH = [];

function envPath(env) {
  // Windows 环境变量名大小写不敏感，但 Node 原样保留：系统里通常是 "Path"。
  // 另写一个 env.PATH 会产生重复键，子进程据此查 as.exe 时 PATH 就坏了。始终顺着已有键读。
  const key = Object.keys(env).find((k) => k.toUpperCase() === "PATH");
  return key ? env[key] : "";
}

function setEnvPath(env, value) {
  const key = Object.keys(env).find((k) => k.toUpperCase() === "PATH") || "PATH";
  env[key] = value;
}

function cargoBin() {
  return path.join(P_CARGO, "bin", IS_WIN ? "cargo.exe" : "cargo");
}

/** 项目内 cargo（唯一默认位置）；不在就返回 null。 */
function findProjectCargo() {
  const p = cargoBin();
  return fs.existsSync(p) ? p : null;
}

let ambientCache = null;
/** 现成环境里的 cargo（项目内缺失时的借用来源；借用只读，缓存与产物仍落项目内）。 */
function findAmbientCargo() {
  if (ambientCache !== null) return ambientCache || null;
  // 直接扫 PATH（不依赖 where/which：受限环境里未必装了它们）。PATH 项可能带外层引号。
  const name = IS_WIN ? "cargo.exe" : "cargo";
  const found = (process.env.PATH || "")
    .split(path.delimiter)
    .map((d) => d.trim().replace(/^"|"$/g, ""))
    .filter(Boolean)
    .map((d) => path.join(d, name))
    .find((p) => { try { return fs.existsSync(p); } catch (e) { return false; } });
  ambientCache = found || "";
  return ambientCache || null;
}

/** 借用系统 cargo 前的可用性探测：跑得起来才算可用（rustup proxy 解析不到 toolchain 会失败）。 */
function probeUsable(cargo) {
  try {
    const r = spawnSync(cargo, ["--version"], { stdio: "ignore", timeout: 15000 });
    return !r.error && r.status === 0;
  } catch (e) {
    return false;
  }
}

function locateDlltool() {
  // 只认项目内安装的完整 binutils（.tools/mingw64/bin）。不扫系统目录、不查 PATH。
  // 返回 { dir, broken }：broken 列只有 dlltool、缺汇编器的半套树。
  if (binutilsReady(WINLIBS_BIN)) return { dir: WINLIBS_BIN, broken: [] };
  if (fs.existsSync(path.join(WINLIBS_BIN, "dlltool.exe"))) return { dir: null, broken: [WINLIBS_BIN] };
  return { dir: null, broken: [] };
}

function binutilsReady(dir) {
  // dlltool 生成导入库时会去拉 GNU 汇编器，所以可用目录需要 dlltool.exe 与 as.exe 同时在场。
  // 静态检查是刻意的：试跑 dlltool 在 Windows 上会把好的 winlibs 误判成坏的。
  if (!dir) return false;
  return fs.existsSync(path.join(dir, "dlltool.exe")) && fs.existsSync(path.join(dir, "as.exe"));
}

function composeEnv(cargo, source) {
  const env = Object.assign({}, process.env);
  // 缓存与安装产物一律落项目内：无论借用系统工具链还是用项目内工具链，CARGO_HOME 都指项目内。
  const cargoHome = process.env.SOLOMNI_CARGO_HOME || P_CARGO;
  env.CARGO_HOME = cargoHome;
  // RUSTUP_HOME 只在用项目内工具链时指项目内；借用系统 rustup proxy 时必须保留系统的（靠它找 toolchain），
  // 且约定只读——门禁不跑会写它的 rustup 变更命令。
  if (source !== "ambient") {
    env.RUSTUP_HOME = process.env.SOLOMNI_RUSTUP_HOME || P_RUSTUP;
  }
  const front = [path.join(cargoHome, "bin"), path.dirname(cargo)]
    .concat(EXTRA_PATH)
    .concat(IS_WIN && fs.existsSync(WINLIBS_BIN) ? [WINLIBS_BIN] : [])
    .filter(Boolean);
  setEnvPath(env, front.concat([envPath(env)].filter(Boolean)).join(path.delimiter));
  return env;
}

/**
 * 把临时目录钉进项目内（缓存/临时不出项目；崩了也留在项目内，可检测可清）。
 * 由入口层在建/跑之前调用，保持 resolve() 无副作用。返回钉住的目录；建不了返回 null（退回系统临时目录）。
 */
function pinTemp(env) {
  const dir = path.join(ROOT, "target", "tmp");
  try {
    fs.mkdirSync(dir, { recursive: true });
  } catch (e) {
    return null;
  }
  env.TMPDIR = dir;
  env.TMP = dir;
  env.TEMP = dir;
  return dir;
}

/**
 * 解析当前环境（不联网、不询问、无副作用）。
 * 覆盖口：SOLOMNI_CARGO / SOLOMNI_CARGO_HOME / SOLOMNI_RUSTUP_HOME（仍不静默用系统安装）。
 * requireProject = true 时只认项目内 / 显式指定，没有就返回 null（交给 ensure 安装）。
 */
function resolve(opts) {
  const requireProject = !!(opts && opts.requireProject);
  let cargo = null;
  let source = "";
  if (process.env.SOLOMNI_CARGO) {
    cargo = process.env.SOLOMNI_CARGO;
    if (!fs.existsSync(cargo)) die("SOLOMNI_CARGO 指向的文件不存在：" + cargo);
    source = "override";
  } else {
    const pc = findProjectCargo();
    if (pc) { cargo = pc; source = "project"; }
  }
  if (!cargo) {
    if (requireProject) return null;
    const ac = findAmbientCargo();
    if (!ac || !probeUsable(ac)) return null;
    cargo = ac;
    source = "ambient";
  }
  return {
    osKey: OS_KEY,
    platformDir: PLATFORM_DIR,
    toolsDir: TOOLS,
    mingwBin: WINLIBS_BIN,
    cargo,
    cargoHome: process.env.SOLOMNI_CARGO_HOME || P_CARGO,
    rustupHome: process.env.SOLOMNI_RUSTUP_HOME || P_RUSTUP,
    source,
    extraPath: EXTRA_PATH.slice(),
    env: composeEnv(cargo, source),
  };
}

/**
 * 确保**项目内**工具链就绪（**开发环境专用**：开发统一用项目内工具链，产物留项目内）。
 * 没有就装进项目；有就直接用。返回 resolve 的结果（source = project）。
 * 与 ensure() 的区别：ensure() 会借用系统工具链（运行环境照旧），这里不借用。
 */
async function ensureProject(opts) {
  const yes = !!(opts && opts.yes);
  if (!findProjectCargo()) {
    await installRust({ yes });
  }
  const r = resolve({ requireProject: true });
  if (!r) die("项目内工具链装好了却仍找不到 cargo（platform/<os>/cargo/bin）");
  return r;
}

/**
 * 保证环境就绪（缺工具链时征求同意后装进项目内），返回 resolve 的结果。
 * 无交互终端且没给 --yes 时不做任何安装，如实报错并给手动步骤（不挂着等输入）。
 */
async function ensure(opts) {
  const yes = !!(opts && opts.yes);
  // 有可用工具链（项目内或系统）就借用；借用只读，缓存与产物仍落项目内。没有或有问题的才装进项目内。
  if (!resolve({})) {
    await installRust({ yes });
  }
  if (IS_WIN) {
    const probe = locateDlltool();
    if (probe.dir) {
      if (!EXTRA_PATH.includes(probe.dir)) EXTRA_PATH.push(probe.dir);
      log("dlltool working: " + probe.dir);
    } else {
      if (probe.broken.length) log("dlltool present but unusable (no assembler): " + probe.broken.join(", "));
      else {
        console.error("[env] dlltool.exe NOT found at any fixed location (rust-lang/rust#140704:");
        console.error("[env] windows-sys raw-dylib needs dlltool; rust-mingw on this toolchain lacks it).");
      }
      const ans = await ask("Install portable MinGW now? winlibs ~200MB into project .tools, no admin, no system changes [y/N] ", { yes });
      if (ans !== null && ans.trim().toLowerCase() === "y") {
        await installWinlibs();
        if (!EXTRA_PATH.includes(WINLIBS_BIN)) EXTRA_PATH.push(WINLIBS_BIN);
      } else {
        console.error("[env] manual route (keeps everything inside the project):");
        console.error("  browser-download a winlibs zip, save it as .tools/winlibs.zip, re-run");
        console.error("  speed-up flags for auto-download: SOLOMNI_GH_MIRROR=<prefix> or HTTPS_PROXY=<url>");
      }
    }
  }
  const r = resolve({});
  if (!r) die("environment not ready: no usable cargo (project or PATH)");
  return r;
}

/**
 * 询问用户；yes = 直接按同意处理，无交互终端 = 返回 null（调用方按“没同意”处置并给手动步骤）。
 */
function ask(question, opts) {
  const yes = !!(opts && opts.yes);
  if (yes) { log("--yes：按同意处理：" + question.trim()); return Promise.resolve("y"); }
  if (!process.stdin.isTTY) return Promise.resolve(null);
  return new Promise((done) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(question, (a) => { rl.close(); done(a); });
  });
}

function runPS(cmd) {
  return spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", cmd], { stdio: "inherit" });
}

function download(url, dest) {
  // curl.exe 随 Windows 自带（10 1803+）；比 Invoke-WebRequest 快且带进度条。
  // -C - 续传已有分片，不从零重来。老系统的兜底走 PowerShell。
  const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
  if (fs.existsSync(curl)) {
    const r = spawnSync(curl, ["-L", "--fail", "--retry", "3", "-C", "-", "--connect-timeout", "30", "-o", dest, url],
      { stdio: "inherit" });
    if (r.status === 0 && fs.existsSync(dest) && fs.statSync(dest).size > 1048576) return true;
    log("curl download failed (exit " + r.status + "); keeping partial file for resume; trying PowerShell fallback...");
  }
  runPS("Invoke-WebRequest -UseBasicParsing '" + url + "' -OutFile '" + dest + "'");
  return fs.existsSync(dest) && fs.statSync(dest).size > 1048576;
}

function mirrorVariants(ghUrl) {
  // 可选镜像：SOLOMNI_GH_MIRROR 用前缀代理形态（如 https://ghfast.top/github.com/owner/repo/...）。
  const m = process.env.SOLOMNI_GH_MIRROR || "";
  if (!m) return [];
  return [m.replace(/\/+$/, "") + "/" + ghUrl.replace("https://github.com/", "")];
}

function sha256(file) {
  const r = spawnSync("certutil", ["-hashfile", file, "SHA256"], { encoding: "utf8" });
  const m = ((r.stdout || "").match(/^[a-f0-9]{64}$/im) || [])[0];
  return (m || "").toLowerCase();
}

function checkWinlibsHash(zip) {
  // 第三方镜像不能悄悄换内容：固定 WINLIBS_SHA256 之后每次下载都校验。
  if (!WINLIBS_SHA256) {
    const h = sha256(zip);
    if (h) {
      log("sha256 " + h);
      log("(pin this in env.js WINLIBS_SHA256 to verify future downloads)");
    }
    return;
  }
  const h = sha256(zip);
  if (h !== WINLIBS_SHA256.toLowerCase()) {
    try { fs.rmSync(zip, { force: true }); } catch (e) { /* remove bad archive */ }
    die("sha256 mismatch (" + (h || "hash unavailable") + " != " + WINLIBS_SHA256 + ") - file deleted. Re-run to download again.");
  }
  log("sha256 verified");
}

function upstreamWinlibsUrl() {
  // 上游资产名带版本，latest/download/winlibs.zip 必然 404；用 API 解析真实文件名。
  const q = [
    "$ErrorActionPreference = 'Stop'",
    "$r = Invoke-RestMethod 'https://api.github.com/repos/brechtsanders/winlibs_mingw/releases/latest'",
    "$a = $r.assets | Where-Object { $_.name -match 'x86_64' -and $_.name -match 'seh' -and $_.name -match '[.]zip$' -and $_.name -notmatch 'llvm' } | Select-Object -First 1",
    "if (-not $a) { $a = $r.assets | Where-Object { $_.name -match 'x86_64' -and $_.name -match '[.]zip$' } | Select-Object -First 1 }",
    "Write-Output $a.browser_download_url"
  ].join("; ");
  const got = spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", q], { encoding: "utf8" });
  const url = (got.stdout || "").trim();
  return /^https:/.test(url) ? url : null;
}

function probeOnce(url) {
  // 一次 2MB 的范围 GET，回 { code, speed }；code 000 = 网络层失败（DNS/重置/超时）。
  return new Promise((done) => {
    const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
    const cmd = IS_WIN ? curl : "curl";
    if (!fs.existsSync(cmd)) { done({ code: "000", speed: 0 }); return; }
    const p = spawn(cmd,
      ["-sL", "-r", "0-2097151", "-o", require("os").devNull, "-w", "%{http_code} %{speed_download}",
       "--connect-timeout", "4", "--max-time", "8", url],
      { encoding: "utf8" });
    let out = "";
    p.stdout.on("data", (d) => { out += d; });
    p.on("error", () => done({ code: "000", speed: 0 }));
    p.on("close", () => {
      const m = out.trim().match(/^(\d{3}) (\d+(?:\.\d+)?)$/);
      if (m) done({ code: m[1], speed: parseFloat(m[2]) || 0 });
      else done({ code: "000", speed: 0 });
    });
  });
}

async function probeSpeed(url) {
  // 试两次：瞬时重置在受限网络上很常见，只探一次会把可达源误杀。
  const a = await probeOnce(url);
  if (/^2/.test(a.code)) return a;
  const b = await probeOnce(url);
  return /^2/.test(b.code) ? b : a;
}

async function pickFastest(urls) {
  // 顺序测速（并行会互相抢带宽、把彼此的数字带偏）。最快者胜，列表顺序只作平手时的次序。
  const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
  if (IS_WIN && !fs.existsSync(curl)) return urls[0];
  log("speed-testing " + urls.length + " source(s), 2MB probe each...");
  const speeds = [];
  const codes = [];
  for (const u of urls) {
    const r = await probeSpeed(u);
    speeds.push(r.speed);
    codes.push(r.code);
    log("  " + Math.round(r.speed / 1024) + " KB/s  [http " + r.code + "]  " + u);
  }
  let best = -1;
  for (let i = 0; i < urls.length; i++) {
    if (/^2/.test(codes[i]) && speeds[i] > 0 && (best < 0 || speeds[i] > speeds[best])) best = i;
  }
  if (best < 0) {
    console.error("[env] no source passed the probe (2xx with data). Codes above mean:");
    console.error("  000 = network blocked/reset (set HTTPS_PROXY, or download the zip manually)");
    console.error("  404 = not on this host (wrong tag/asset name, or mirror does not carry it)");
    console.error("  403 = rate limited (wait a bit and re-run)");
    die("no reachable source. Manual: browser-download a winlibs x86_64 seh zip, save as .tools/winlibs.zip, re-run.");
  }
  log("fastest: " + urls[best]);
  return urls[best];
}

function zipLooksValid(zipPath) {
  // 三态：true = 完整且内含 dlltool；"partial" = 像没下完，留着续传；false = 内容坏，删掉。
  const st = fs.statSync(zipPath);
  if (st.size < 10485760) return "partial";
  const t = spawnSync("tar", ["-tf", zipPath], { encoding: "utf8" });
  if (t.error && t.error.code === "ENOENT") return true; // 没有 tar：只按大小判，不删可能完好的文件
  if (t.status !== 0) return false;
  return /bin[\\/]dlltool\.exe/i.test(t.stdout || "");
}

async function installWinlibs() {
  // 可携 MinGW-w64（binutils 提供 dlltool.exe），只落在项目内 .tools/mingw64。
  // 不要管理员、不改系统 PATH；删掉那个目录即可移除。
  if (binutilsReady(WINLIBS_BIN)) {
    log("portable MinGW already installed and working: " + WINLIBS_BIN);
    return;
  }
  fs.mkdirSync(TOOLS, { recursive: true });
  const zip = path.join(TOOLS, "winlibs.zip");
  if (fs.existsSync(zip)) {
    const v = zipLooksValid(zip);
    if (v === true) {
      log("using existing " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB)");
    } else if (v === "partial") {
      log("found partial " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB) - will resume.");
    } else {
      log("existing " + zip + " is corrupt (wrong content) - deleting and re-downloading.");
      try { fs.rmSync(zip, { force: true }); } catch (e) { /* re-attempt below anyway */ }
    }
  }
  if (!fs.existsSync(zip)) {
    const rel = REL_BASE.replace(/\/+$/, "");
    const ours = rel + "/winlibs.zip";
    const candidates = [ours].concat(mirrorVariants(ours));
    const upstream = await upstreamWinlibsUrl();
    if (upstream) candidates.push(upstream, ...mirrorVariants(upstream));
    const extraMirror = (process.env.SOLOMNI_GH_MIRROR || "").replace(/\/+$/, "");
    if (extraMirror) candidates.push(extraMirror + "/" + (upstream || ours).replace("https://github.com/", ""));
    const seen = [];
    for (const c of candidates) if (seen.indexOf(c) < 0) seen.push(c);
    const url = await pickFastest(seen);
    log("downloading " + url);
    log("(curl with progress; ~200 MB. Too slow? set SOLOMNI_GH_MIRROR=https://ghfast.top");
    log(" or HTTPS_PROXY=http://127.0.0.1:port, or browser-save the zip as .tools/winlibs.zip)");
    const ok = download(url, zip) ? zipLooksValid(zip) : false;
    if (ok === false) {
      log("download still incomplete or corrupt - re-run to resume, or download manually:");
      die("  get a winlibs x86_64 seh zip from https://github.com/brechtsanders/winlibs_mingw/releases");
    }
    checkWinlibsHash(zip);
  }
  log("extracting (takes a minute)...");
  const tar = spawnSync("tar", ["-xf", zip, "-C", TOOLS], { stdio: "ignore" });
  if (tar.status !== 0) runPS("Expand-Archive -Force '" + zip + "' -DestinationPath '" + TOOLS + "'");
  fs.rmSync(zip, { force: true });
  if (!fs.existsSync(path.join(WINLIBS_BIN, "dlltool.exe"))) die("extracted, but dlltool.exe is not in .tools/mingw64/bin - inspect the .tools directory.");
  log("portable MinGW ready: " + WINLIBS_BIN);
}

async function installRust(opts) {
  const yes = !!(opts && opts.yes);
  log("Rust toolchain not found inside the project (system-wide installs are ignored by design).");
  const ans = await ask("Install Rust into the project now via rustup (about 500 MB, once)? [y/N] ", { yes });
  if (ans === null) {
    die("no interactive terminal to ask for consent; nothing installed. Manual: install Rust with RUSTUP_HOME=" +
      P_RUSTUP + " CARGO_HOME=" + P_CARGO + ", or re-run with --yes.");
  }
  if (ans.trim().toLowerCase() !== "y") die("aborted. Re-run and answer y to install Rust inside the project.");
  log("installing Rust (project-local, a few hundred MB, once; nothing written outside the project)...");
  fs.mkdirSync(PLATFORM_DIR, { recursive: true });
  const installEnv = Object.assign({}, process.env, { RUSTUP_HOME: P_RUSTUP, CARGO_HOME: P_CARGO });
  if (IS_WIN) {
    // GNU 工具链自带链接器，不需要 Visual Studio Build Tools；--no-modify-path 不动系统 PATH。
    const initExe = path.join(TOOLS, "rustup-init.exe");
    fs.mkdirSync(TOOLS, { recursive: true });
    if (!download("https://win.rustup.rs/x86_64", initExe)) die("rustup-init download failed. Check network / proxy.");
    const init = spawnSync(initExe,
      ["-y", "--default-toolchain", "stable-x86_64-pc-windows-gnu", "--no-modify-path"],
      { stdio: "inherit", env: installEnv });
    if (init.status !== 0) die("rustup install failed.");
  } else {
    const r = spawnSync("sh",
      ["-c", "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path"],
      { stdio: "inherit", env: installEnv });
    if (r.status !== 0) die("rustup install failed. Check network / curl availability.");
  }
  const cargo = findProjectCargo();
  if (!cargo) die("installed, but cargo still not visible. Reopen the terminal and re-run.");
  log("Rust installed: " + cargo);
  return cargo;
}

function status() {
  const r = resolve({});
  const lines = [
    "os: " + OS_KEY,
    "platform dir: " + PLATFORM_DIR,
    "project cargo: " + (findProjectCargo() || "(none)"),
    "ambient cargo: " + (findAmbientCargo() || "(none)"),
  ];
  if (IS_WIN) lines.push("winlibs: " + (binutilsReady(WINLIBS_BIN) ? WINLIBS_BIN : "(not ready)"));
  lines.push("resolved: " + (r ? r.cargo + " [" + r.source + "]" : "(not found)"));
  console.log(lines.join("\n"));
}

function printEnv() {
  const r = resolve({});
  if (!r) {
    console.error(JSON.stringify({ osKey: OS_KEY, ok: false, platformDir: PLATFORM_DIR }, null, 2));
    process.exitCode = 1;
    return;
  }
  console.log(JSON.stringify({
    osKey: r.osKey, cargo: r.cargo, source: r.source, cargoHome: r.cargoHome, rustupHome: r.rustupHome,
    platformDir: r.platformDir, toolsDir: r.toolsDir, mingwBin: r.mingwBin,
  }, null, 2));
}

async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--print-env")) return printEnv();
  if (argv.includes("setup")) {
    const r = await ensure({ yes: argv.includes("--yes") });
    log("environment ready: " + r.cargo);
    return;
  }
  return status();
}

module.exports = {
  ROOT, IS_WIN, OS_KEY, PLATFORM_DIR, P_RUSTUP, P_CARGO, TOOLS, WINLIBS_BIN,
  resolve, ensure, ensureProject, pinTemp, findProjectCargo, findAmbientCargo, envPath, setEnvPath,
};

if (require.main === module) main();
