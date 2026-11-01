#!/usr/bin/env node
/**
 * Solomni 环境层：路径约定（platform/<os>、.tools/）、工具链探测、环境拼装与（征得同意的）项目内安装。
 * 管：工具链在哪、环境怎么拼、缺了怎么补；不管：构建、运行、测试编排（那些在 start.js / run-tests.js）。
 * 为什么单独一层：环境是“换机器 / 换开发环境”时唯一要改的地方；启动器与测试入口共用同一份解析，不再各写一遍。
 * 收敛规则：缓存与产物一律落在项目内 platform/<os>/ 与 .tools/（AGENTS.md）；系统工具链可只读借用，但不写它的 home。
 * 完整性：下载源的校验值与**那条链接成对**放在 links.yaml（`<键>.sha256`），不写死在代码里——
 *   分居两处就会腐烂（曾把 URL 升到新版本、忘了改常量：每次下载都判不符 → 删文件 → 重跑）。
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
// 可携 MinGW 的 binutils（dlltool.exe + as.exe）落在项目内；已有完整 binutils 的机器可以用
// SOLOMNI_MINGW_BIN=<含 dlltool.exe 与 as.exe 的 bin 目录> 指过去，省掉一次 260 MB 下载。
// 校验值不在这里：它与下载源成对放在 links.yaml（winlibs.upstream.asset.sha256）。

const log = (m) => console.log("[env] " + m);
const die = (m) => { console.error("[env] " + m); process.exit(1); };

// 外部连接台账（唯一真相）：项目自用、可能变的外部 URL 都在 links.yaml；下载源的校验值与那条链接
// 成对写在同一份台账里（键 + `.sha256`）。
// 读法只认扁平的"键: 值"（与 tests/gaps.yaml 同一手法）：不引 YAML 依赖；缺键/坏行如实报错。
const LINKS = readLinks(path.join(ROOT, "links.yaml"));

function readLinks(file) {
  if (!fs.existsSync(file)) die("missing links.yaml (external links ledger): " + file);
  const out = {};
  for (const raw of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const m = line.match(/^([A-Za-z0-9_.-]+):\s*(\S+)\s*$/);
    if (!m) die("links.yaml line is not key: value: " + raw);
    if (out[m[1]] !== undefined) die("links.yaml duplicate key: " + m[1]);
    out[m[1]] = m[2];
  }
  return out;
}

function link(key) {
  const v = LINKS[key];
  if (!v) die("links.yaml missing key: " + key);
  return v;
}
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

/** 借用系统 cargo 前的第一道探测：跑得起来才值得往下看（rustup proxy 解析不到 toolchain 会失败）。 */
function probeUsable(cargo) {
  try {
    const r = spawnSync(cargo, ["--version"], { stdio: "ignore", timeout: 15000 });
    return !r.error && r.status === 0;
  } catch (e) {
    return false;
  }
}

/** 目的：cargo -vV 的 host 行——借来的到底是哪种口味（msvc / gnu）；顺带留全文给人看。 */
function cargoVv(r) {
  const v = spawnSync(r.cargo, ["-vV"], { cwd: ROOT, env: r.env, encoding: "utf8", timeout: 20000 });
  const text = ((v.stdout || "") + (v.stderr || "")).trim();
  const host = ((text.match(/^host:\s*(\S+)/m) || [])[1]) || "";
  return { ok: !v.error && v.status === 0 && !!host, host, detail: text };
}

/** 输出尾部：错误原文留最后几行，去掉空行（探测器只负责给证据，不负责翻译）。 */
function tail(text, lines) {
  const all = String(text || "").split(/\r?\n/).map((s) => s.trimEnd()).filter(Boolean);
  return all.slice(-(lines || 6)).join("\n");
}

/** 一句能当结论的错误行：优先第一条 error:，否则第一行（摘要里要塞得下，不能只给 "aborting"）。 */
function firstError(text) {
  const all = String(text || "").split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
  return all.find((s) => /^error(\[|:)/.test(s)) || all[0] || "";
}

let buildProbe = null;
/**
 * 目的：判断一套工具链**能不能构建出二进制**，而不只是能打印版本。
 * 为什么必须真编：Windows 上 `cargo --version` 能跑、host 却是 msvc 且机器没有 link.exe（没装 VS C++）时，
 *   一链接就报 `linker link.exe not found`。只查版本会把这种工具链当"可用"，借来之后在构建阶段才炸，
 *   入口层还会把它归成"是不是另一个窗口在跑"。
 * 约束：只对借用 / 显式指定的工具链探测（项目内那套由本层装出来，坏了由构建输出说明）；
 *   编译一个空程序并链接，用**同一份**拼好的环境（PATH 里已有 mingw bin）；
 *   每进程只探一次；产物只落 `target/tmp/env-probe-<pid>/`，探完即删。
 * 返回：{ ok, host, detail }（detail = 编译器原文尾部）。
 */
function probeBuild(r) {
  if (buildProbe && buildProbe.cargo === r.cargo) return buildProbe.result;
  const result = probeBuildUncached(r);
  buildProbe = { cargo: r.cargo, result };
  return result;
}

function probeBuildUncached(r) {
  const vv = cargoVv(r);
  if (!vv.ok) return { ok: false, host: vv.host, detail: tail(vv.detail) || "读不到 cargo -vV 的 host" };
  // rustc 与 cargo 同在 bin 目录（rustup proxy 也一样）；找不到才退回 PATH 上的。
  const sibling = path.join(path.dirname(r.cargo), IS_WIN ? "rustc.exe" : "rustc");
  const rustc = fs.existsSync(sibling) ? sibling : "rustc";
  // 目录带 pid：门禁与启动器可能同时跑，互不踩同一份探测产物。
  const dir = path.join(ROOT, "target", "tmp", "env-probe-" + process.pid);
  const src = path.join(dir, "probe.rs");
  const out = path.join(dir, IS_WIN ? "probe.exe" : "probe");
  try {
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(src, "fn main() {}\n");
    const p = spawnSync(rustc, [src, "-o", out], { cwd: ROOT, env: r.env, encoding: "utf8", timeout: 120000 });
    if (p.error) return { ok: false, host: vv.host, detail: p.error.message };
    if (p.status !== 0) return { ok: false, host: vv.host, detail: (p.stderr || "") + (p.stdout || "") };
    return { ok: true, host: vv.host, detail: "" };
  } catch (e) {
    return { ok: false, host: vv.host, detail: (e && e.message) || String(e) };
  } finally {
    try { fs.rmSync(dir, { recursive: true, force: true }); } catch (e) { /* 清不掉不影响判定 */ }
  }
}

function locateDlltool() {
  // 返回 { dir, broken }：broken 列"有 dlltool 但缺汇编器"的半套树（试跑 dlltool 会把好的误判成坏的，所以静态查）。
  // ① 显式指定优先：已有完整 binutils 的机器（例如系统 MSYS2）不必为 dlltool 再下 260 MB。
  const override = process.env.SOLOMNI_MINGW_BIN;
  if (override) {
    if (binutilsReady(override)) return { dir: override, broken: [] };
    return { dir: null, broken: [override + "（SOLOMNI_MINGW_BIN）"] };
  }
  // ② 项目内安装的完整 binutils（.tools/mingw64/bin）。不扫系统目录、不查 PATH。
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

/** 目的：拼 PATH 时该放哪个 bin 目录——显式指定（可用）优先，其次项目内那份。 */
function mingwBinOnPath() {
  const o = process.env.SOLOMNI_MINGW_BIN;
  if (o && binutilsReady(o)) return [o];
  return fs.existsSync(WINLIBS_BIN) ? [WINLIBS_BIN] : [];
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
    .concat(IS_WIN ? mingwBinOnPath() : [])
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
 * 保证环境就绪（缺**可用**工具链时征求同意后装进项目内），返回 resolve 的结果。
 * 借用门槛是"能构建"（见 probeBuild）：能打印版本但链接不了的工具链不算可用——这是本机踩过的坑。
 * 无交互终端且没给 --yes 时不做任何安装，如实报错并给手动步骤（不挂着等输入）。
 */
async function ensure(opts) {
  const yes = !!(opts && opts.yes);
  let r = resolve({});
  let host = "";
  if (r && r.source === "ambient") {
    const p = probeBuild(r);
    host = p.host;
    if (p.ok) {
      log("borrowed Rust can build: host " + p.host);
    } else {
      // 借用是只读的；借来的东西构建不出来就等于没有工具链，明说原因再走安装流程。
      console.error("[env] 借用不成立：这套 cargo 能打印版本，却构建不出二进制。");
      console.error("[env]   source " + r.cargo);
      console.error("[env]   host   " + (p.host || "(读不到)"));
      for (const line of [firstError(p.detail)].concat(tail(p.detail, 4).split("\n")).filter(Boolean)) console.error("[env]   " + line);
      console.error("[env] 不借用它：按下面的流程装项目内工具链（Windows 用 GNU，不需要 Visual Studio）。");
      r = null;
    }
  } else if (r && r.source === "override") {
    // 显式指定压倒一切（与 SOLOMNI_MINGW_BIN 同一口径），但把探测结果如实说出来。
    const p = probeBuild(r);
    host = p.host;
    if (!p.ok) log("注意：SOLOMNI_CARGO 指定的工具链构建不出二进制（host " + (p.host || "读不到") + "）：仍按显式指定使用。");
  }
  if (!r) {
    await installRust({ yes });
  }
  if (IS_WIN) {
    // dlltool 只服务 GNU 目标（windows-sys 的 raw-dylib 要它）；借到能链接的 msvc 工具链时不需要它。
    if (!host && r) host = cargoVv(r).host;
    if (!host || /-gnu$/.test(host)) await ensureDlltool(yes);
    else log("dlltool not required: host " + host + " links with its own toolchain");
  }
  const ready = resolve({});
  if (!ready) die("environment not ready: no usable cargo (project or PATH)");
  return ready;
}

/** 目的：Windows GNU 目标的 dlltool（windows-sys 的 raw-dylib 要它；这份 rust-mingw 不带）。 */
async function ensureDlltool(yes) {
  const probe = locateDlltool();
  if (probe.dir) {
    if (!EXTRA_PATH.includes(probe.dir)) EXTRA_PATH.push(probe.dir);
    log("dlltool working: " + probe.dir);
    return;
  }
  if (probe.broken.length) log("dlltool present but unusable (no assembler): " + probe.broken.join(", "));
  else {
    console.error("[env] dlltool.exe NOT found at any fixed location (rust-lang/rust#140704:");
    console.error("[env] windows-sys raw-dylib needs dlltool; rust-mingw on this toolchain lacks it).");
  }
  const ans = await ask("Install portable MinGW now? winlibs ~260MB into project .tools, no admin, no system changes [y/N] ", { yes });
  if (ans !== null && ans.trim().toLowerCase() === "y") {
    await installWinlibs();
    if (!EXTRA_PATH.includes(WINLIBS_BIN)) EXTRA_PATH.push(WINLIBS_BIN);
  } else {
    console.error("[env] manual route (keeps everything inside the project):");
    console.error("  browser-download a winlibs zip, save it as .tools/winlibs.zip, re-run;");
    console.error("  or point at an existing full binutils: SOLOMNI_MINGW_BIN=<dir with dlltool.exe + as.exe>");
    console.error("  speed-up flags for auto-download: SOLOMNI_GH_MIRROR=<prefix> or HTTPS_PROXY=<url>");
  }
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

/**
 * 目的：把 URL 下到目标文件；返回**整份成功**（退出码 0），不再拿"文件比 1 MB 大"当成功——
 *   半截文件被当成成功就会直接进解压（实测报 `找不到中央目录结尾记录`）。
 * 参数：resume = 复用已有分片。**只对同一个 URL 成立**（换源续传等于把两份字节接起来，归档必坏），
 *   所以调用方用分片旁边记下的源来续。
 * 约束：curl 在场就只用 curl（续传失败时保留分片，别用"从头下"把它冲掉）；curl 缺席才走 PowerShell（它从头下）。
 */
function download(url, dest, resume) {
  const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
  if (fs.existsSync(curl)) {
    const args = ["-L", "--fail", "--retry", "3", "--connect-timeout", "30", "-o", dest];
    if (resume) args.push("-C", "-");
    args.push(url);
    const r = spawnSync(curl, args, { stdio: "inherit" });
    if (r.status === 0 && fs.existsSync(dest)) return true;
    // 失败就是失败：分片留着（下次从**同一个**源续），也不再用 PowerShell 从头下把证据冲掉。
    log("curl download failed (exit " + r.status + ") - partial kept for the next run");
    return false;
  }
  // 没有 curl（10 1803 以前的老系统）才走 PowerShell 兜底：它从头下，失败如实失败。
  const ps = runPS("Invoke-WebRequest -UseBasicParsing '" + url + "' -OutFile '" + dest + "'");
  return ps.status === 0 && fs.existsSync(dest);
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

/**
 * 目的：归档完整性闸——校验值来自 links.yaml（`winlibs.upstream.asset.sha256`，与下载源**成对**），
 *   不写死在代码里：分居两处就会腐烂（URL 升了新版本、常量忘了改 ⇒ 每次下载都判 "不符 → 删文件 → 重跑"）。
 * 约束：不符时**不删文件**（删了既丢证据，又会在 pin 过期时变成每次重下 260 MB 的死循环）：
 *   报出台账值 / 实际值 / 来源，并给一个显式接受开关（确实要换资产时用）。
 */
function verifyWinlibsHash(zip) {
  const want = link("winlibs.upstream.asset.sha256").toLowerCase();
  const got = sha256(zip);
  if (!got) die("算不出 " + zip + " 的 sha256（certutil 不可用）：不跳过校验，手动核对后重跑。");
  if (got === want) {
    log("sha256 verified");
    return;
  }
  if (process.env.SOLOMNI_ACCEPT_ASSET_CHANGE === "1") {
    log("sha256 与台账不符，但已按 SOLOMNI_ACCEPT_ASSET_CHANGE=1 接受：实际 " + got);
    return;
  }
  die("sha256 不符：\n  台账 " + want + "\n  实际 " + got + "\n  来源 " + link("winlibs.upstream.asset") +
    "\n  上游换过资产就核对同 release 的 <资产>.sha256（或 GitHub API 的资产 digest 字段），" +
    "\n  再把 links.yaml 的 winlibs.upstream.asset.sha256 改成上面的实际值；" +
    "\n  确认可信、想先跑起来：SOLOMNI_ACCEPT_ASSET_CHANGE=1 重跑一次。");
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
    die("no reachable source. Manual: browser-download a winlibs x86_64 seh zip, save as .tools/winlibs.zip, re-run;" +
    "\n  or point at an existing full binutils with SOLOMNI_MINGW_BIN=<dir with dlltool.exe + as.exe>.");
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

/** 目的：分片来源标记——续传必须打向**产出这份分片**的那个 URL（换源续等于把两份字节接起来，归档必坏）。 */
function winlibsSourceMarker() {
  return path.join(TOOLS, "winlibs.zip.src");
}

/**
 * 目的：可携 MinGW-w64（binutils 提供 dlltool.exe），只落在项目内 .tools/mingw64。
 *   不要管理员、不改系统 PATH；删掉那个目录即可移除。
 * 约束：**任何要解压的归档都先过完整性闸**（sha256 来自 links.yaml，与下载源成对）；
 *   半截归档会真续传，且只打向产出它的那个源；不知道来源的分片宁可从零重下。
 */
async function installWinlibs() {
  if (binutilsReady(WINLIBS_BIN)) {
    log("portable MinGW already installed and working: " + WINLIBS_BIN);
    return;
  }
  fs.mkdirSync(TOOLS, { recursive: true });
  const zip = path.join(TOOLS, "winlibs.zip");
  const marker = winlibsSourceMarker();
  // 三态：true = 完整；"partial" = 像没下完（留着续传）；false = 内容坏（删掉重下）。
  let state = "absent";
  if (fs.existsSync(zip)) {
    state = zipLooksValid(zip);
    if (state === false) {
      log("existing " + zip + " is corrupt (wrong content) - deleting and re-downloading.");
      fs.rmSync(zip, { force: true });
      fs.rmSync(marker, { force: true });
      state = "absent";
    } else if (state === "partial") {
      log("found partial " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB) - will resume.");
    } else {
      log("using existing " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB)");
    }
  }
  // 来源不明的分片不能续（接起来必坏）：宁可从头下。
  if (state === "partial" && !fs.existsSync(marker)) {
    log("partial has no recorded source - restarting from scratch.");
    fs.rmSync(zip, { force: true });
    state = "absent";
  }
  if (state !== true) {
    let url;
    if (state === "partial") {
      url = fs.readFileSync(marker, "utf8").trim();
      log("resuming from the same source: " + url);
    } else {
      // 上游资产名带版本：台账里钉死一个 release 的完整 URL（不查 GitHub API，也就没有限流）。
      const upstream = link("winlibs.upstream.asset");
      const candidates = [upstream].concat(mirrorVariants(upstream));
      const extraMirror = (process.env.SOLOMNI_GH_MIRROR || "").replace(/\/+$/, "");
      if (extraMirror) candidates.push(extraMirror + "/" + upstream.replace("https://github.com/", ""));
      const seen = [];
      for (const c of candidates) if (seen.indexOf(c) < 0) seen.push(c);
      url = await pickFastest(seen);
      log("downloading " + url);
      log("(curl with progress; ~260 MB. Too slow? set SOLOMNI_GH_MIRROR=https://ghfast.top");
      log(" or HTTPS_PROXY=http://127.0.0.1:port, or browser-save the zip as .tools/winlibs.zip)");
      fs.writeFileSync(marker, url);
    }
    if (!download(url, zip, state === "partial")) {
      die("下载没有完成（网络中断？）。分片留在 " + zip + "，重跑会从**同一个**源续传；" +
        "\n  想换源就先删掉它和 " + marker + "；或用浏览器下好整份放到 " + zip);
    }
    fs.rmSync(marker, { force: true });
    if (zipLooksValid(zip) !== true) {
      fs.rmSync(zip, { force: true });
      die("下到的归档结构不对（被截断，或被中间页替换），已删除；重跑，或手动下载：" + link("winlibs.upstream.releases"));
    }
  }
  verifyWinlibsHash(zip);
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
    if (!download(link("rustup.init.windows"), initExe)) die("rustup-init download failed. Check network / proxy.");
    const init = spawnSync(initExe,
      ["-y", "--default-toolchain", "stable-x86_64-pc-windows-gnu", "--no-modify-path"],
      { stdio: "inherit", env: installEnv });
    if (init.status !== 0) die("rustup install failed.");
  } else {
    const r = spawnSync("sh",
      ["-c", "curl --proto '=https' --tlsv1.2 -sSf " + link("rustup.init.unix") + " | sh -s -- -y --no-modify-path"],
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
  if (IS_WIN) {
    const override = process.env.SOLOMNI_MINGW_BIN;
    lines.push("binutils: " + (override && binutilsReady(override) ? override + "（SOLOMNI_MINGW_BIN）" :
      binutilsReady(WINLIBS_BIN) ? WINLIBS_BIN : "(not ready)"));
  }
  if (!r) {
    lines.push("resolved: (not found)");
    lines.push("build: 不可用 —— 项目内与 PATH 上都没有 cargo（补全：node env.js setup）");
    console.log(lines.join("\n"));
    return;
  }
  // 状态是诊断入口：给出 host 与**构建探测**结论，不出现"就绪但构建不了"的假绿。
  const p = probeBuild(r);
  lines.push("resolved: " + r.cargo + " [" + r.source + "]");
  lines.push("host: " + (p.host || "(读不到)"));
  lines.push("build: " + (p.ok ? "可用" : "不可用 —— " + (firstError(p.detail) || "构建探测失败")));
  if (r.source === "ambient") lines.push("borrow: " + (p.ok ? "借用" : "不借用（ensure / setup 会装项目内工具链）"));
  if (IS_WIN) {
    lines.push("dlltool: " + (!p.host || /-gnu$/.test(p.host) ? "需要（GNU 目标）" : "不需要（host 不是 GNU）"));
  }
  console.log(lines.join("\n"));
}

function printEnv() {
  const r = resolve({});
  if (!r) {
    console.error(JSON.stringify({ osKey: OS_KEY, ok: false, platformDir: PLATFORM_DIR }, null, 2));
    process.exitCode = 1;
    return;
  }
  const p = probeBuild(r);
  console.log(JSON.stringify({
    osKey: r.osKey, cargo: r.cargo, source: r.source, cargoHome: r.cargoHome, rustupHome: r.rustupHome,
    platformDir: r.platformDir, toolsDir: r.toolsDir, mingwBin: r.mingwBin,
    host: p.host, canBuild: p.ok,
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
  probeBuild, cargoVv, link,
};

if (require.main === module) main();
