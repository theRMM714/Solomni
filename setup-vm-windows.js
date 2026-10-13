#!/usr/bin/env node
/**
 * 目的：虚拟机档本地环境安装（**Windows**）——把 QEMU（MSYS2 mingw64 包）与 Debian guest 装到项目内 .vm/。
 * 管：QEMU 依赖闭包解析与解包（纯 Rust 解包）、Debian cloud 镜像（qcow2）下载、首启经 cloud-init 装 qemu-guest-agent、guest.yaml。
 * 不管：产品对 VM 档的检测 / 装配（在 src/capabilities/vm/）；Linux / macOS 的环境在 setup-vm-linux.js / setup-vm-mac.js。
 * 用法：node setup-vm-windows.js [--check] [--yes] [--offline]
 * 约束：只写项目内 .vm/；下载前要用户同意（--yes 或交互确认）；不写系统目录、不装系统包、不碰注册表。
 * 下载源：QEMU 走本机 MSYS2 的包库，镜像走 links.yaml 的 guest.debian.qcow2。
 */
"use strict";
const fs = require("fs");
const http = require("http");
const net = require("net");
const path = require("path");
const { spawn } = require("child_process");
const c = require("./setup-vm-common.js");

const CHECK = process.argv.includes("--check");
const YES = process.argv.includes("--yes");
const OFFLINE = process.argv.includes("--offline");
c.setOffline(OFFLINE);
const { ROOT, VM, QEMU, BASE, CACHE, log, bad, die, ask, exists, link, capture, curlDownload, unpackArchive } = c;

const QEMU_PKG = "mingw-w64-x86_64-qemu";
const PACMAN_CANDIDATES = ["C:\\msys64\\usr\\bin\\pacman.exe", "C:\\msys64\\usr\\bin\\pacman"];
const PROVISION_TIMEOUT_MS = 40 * 60 * 1000;

const qemuExe = () => path.join(QEMU, "mingw64", "bin", "qemu-system-x86_64.exe");
const qemuReady = () => exists(qemuExe());
const guestImage = () => path.join(BASE, "rootfs.qcow2");
const guestYaml = () => path.join(BASE, "guest.yaml");
const agentMarker = () => path.join(BASE, ".qga-ready");
const agentReady = () => exists(agentMarker());
const runtimesMarker = () => path.join(BASE, ".runtimes-ready");
const runtimesReady = () => exists(runtimesMarker());
const guestReady = () => exists(guestImage()) && exists(guestYaml());

function writeGuestYaml() {
  fs.mkdirSync(BASE, { recursive: true });
  fs.writeFileSync(
    guestYaml(),
    [
      "# 由 setup-vm-windows.js 生成：这份镜像的命令通道是 QGA，共享客户端按打包情况声明。",
      "id: debian-13-genericcloud-amd64",
      "arch: amd64",
      "agent: qga",
      "clients: [9p, virtiofs]",
      "",
    ].join("\n")
  );
}

// ---- QEMU（MSYS2 闭包）----
function findPacman() {
  for (const p of PACMAN_CANDIDATES) if (exists(p)) return p;
  const w = capture("where", ["pacman"]);
  if (w.ok) {
    const first = w.text.split(/\r?\n/)[0].trim();
    if (first) return first;
  }
  return null;
}
function resolveClosure(pacman) {
  const r = capture(pacman, ["-Sp", "--print-format", "%n %l", QEMU_PKG]);
  if (!r.ok) return null;
  const rows = r.text
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter(Boolean)
    .map((line) => {
      const i = line.lastIndexOf(" ");
      return { name: line.slice(0, i).trim(), url: line.slice(i + 1).trim() };
    })
    .filter((x) => x.url);
  return rows.length ? rows : null;
}
function msys2PathFromFileUrl(url) {
  const rel = url.replace(/^file:\/\/\//, "").replace(/\//g, "\\");
  return path.join("C:\\msys64", rel);
}
function ensureQemu() {
  if (qemuReady()) {
    log("QEMU 已在项目内：" + path.relative(ROOT, qemuExe()));
    return;
  }
  const pacman = findPacman();
  if (!pacman) {
    die(
      "没找到 MSYS2 的 pacman（C:\\msys64\\usr\\bin\\pacman.exe）。两条路：装 MSYS2 后重跑；" +
        "或把 QEMU 放进 " + path.relative(ROOT, QEMU) + "\\mingw64\\bin\\ 后重跑 --check。"
    );
  }
  const rows = resolveClosure(pacman);
  if (!rows) die("解析 QEMU 依赖闭包失败（pacman -Sp 没给出包）。");
  log("QEMU 依赖闭包：" + rows.length + " 个包（来自本机 MSYS2 包库）");
  const cache = path.join(CACHE, "msys2");
  fs.mkdirSync(cache, { recursive: true });
  let done = 0;
  for (const row of rows) {
    if (row.url.startsWith("file://")) {
      const src = msys2PathFromFileUrl(row.url);
      const dst = path.join(cache, path.basename(src));
      if (!exists(dst)) {
        try { fs.copyFileSync(src, dst); } catch (e) { die("复制本机缓存包失败：" + src); }
      }
    } else {
      const dst = path.join(cache, path.basename(new URL(row.url).pathname));
      if (!curlDownload(row.url, dst)) die("下载失败：" + row.url);
    }
    done += 1;
    if (done % 20 === 0) log("已备好 " + done + " / " + rows.length + " 个包");
  }
  fs.mkdirSync(QEMU, { recursive: true });
  for (const f of fs.readdirSync(cache)) {
    if (!f.endsWith(".pkg.tar.zst")) continue;
    if (!unpackArchive(path.join(cache, f), QEMU)) die("解包失败：" + f);
  }
  if (!qemuReady()) die("包解完了但没找到 " + qemuExe());
  const v = capture(qemuExe(), ["-version"]);
  if (!v.ok) die("QEMU 装好了但起不来：" + qemuExe() + "（" + v.text.split(/\r?\n/)[0] + "）");
  log("QEMU 就绪：" + path.relative(ROOT, qemuExe()));
}

// ---- guest 镜像（Debian cloud qcow2）----
function ensureGuestImage() {
  fs.mkdirSync(BASE, { recursive: true });
  if (!exists(guestImage())) {
    if (!curlDownload(link("guest.debian.qcow2"), guestImage())) die("Debian 镜像下载失败");
  } else {
    log("guest 镜像已在项目内：" + path.relative(ROOT, guestImage()));
  }
  if (!exists(guestYaml())) writeGuestYaml();
}

// ---- cloud-init 数据源（只发 user-data / meta-data）----
// 首启要做两件事：装上 qemu-guest-agent，并让它随设备出现而启动（Debian 的 unit 绑定 virtio-serial 端口）。
const USER_DATA = () =>
  [
    "#cloud-config",
    "runcmd:",
    "  - >-",
    "    dpkg -s qemu-guest-agent >/dev/null 2>&1 || (apt-get update -o Acquire::Languages=none &&",
    "    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends qemu-guest-agent);",
    "    systemctl enable qemu-guest-agent >/dev/null 2>&1; systemctl start qemu-guest-agent >/dev/null 2>&1; true",
    "",
  ].join("\n");
const META_DATA = "instance-id: solomni-provision\nlocal-hostname: solomni-guest\n";

function startSeedServer() {
  const server = http.createServer((req, res) => {
    const u = req.url.split("?")[0];
    if (u === "/user-data") { res.writeHead(200); res.end(USER_DATA()); return; }
    if (u === "/meta-data") { res.writeHead(200); res.end(META_DATA); return; }
    res.writeHead(404); res.end("");
  });
  return new Promise((resolve) => server.listen(0, "0.0.0.0", () => resolve({ server })));
}

// ---- QGA 最小客户端：判据是"guest 里的 agent 真能握手"，不是 cloud-init 的回执 ----
function qgaCall(port, payload, timeoutMs) {
  return new Promise((resolve, reject) => {
    const s = net.connect({ host: "127.0.0.1", port });
    let buf = "";
    const finish = (err, val) => { try { s.destroy(); } catch (e) {} if (err) reject(err); else resolve(val); };
    s.setTimeout(timeoutMs || 5000, () => finish(new Error("QGA 超时")));
    s.on("error", (e) => finish(e));
    s.on("connect", () => s.write(JSON.stringify(payload) + "\n"));
    s.on("data", (d) => {
      buf += d.toString("utf8");
      let i;
      while ((i = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 1);
        if (!line.trim()) continue;
        let msg;
        try { msg = JSON.parse(line); } catch (e) { continue; }
        if (msg.error) return finish(new Error(msg.error.desc || "QGA 报错"));
        return finish(null, msg.return);
      }
    });
  });
}
async function qgaSync(port) {
  const id = Math.floor(Math.random() * 1000000000) + 1;
  const back = await qgaCall(port, { execute: "guest-sync", arguments: { id } }, 5000);
  if (Number(back) !== id) throw new Error("QGA 同步号对不上：" + back);
}
function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.on("error", reject);
    s.listen(0, "127.0.0.1", () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });
}

// ---- 一次"维护启动"：起 guest、等 QGA 握手、把活交给 work(port)，然后关机 ----
async function withGuest(label, needSeed, work) {
  if (!qemuReady()) die("先装 QEMU（重跑不加 --check）。");
  if (!exists(guestImage())) die("先下载 guest 镜像（重跑不加 --check）。");
  const { server } = await startSeedServer();
  const port = await freePort();
  const accel = process.env.SOLOMNI_QEMU_ACCEL || "whpx:tcg";
  log(label + "（QGA 端口 " + port + "，加速 " + accel + "）");
  const args = [
    "-m", "2048", "-smp", "2", "-machine", "accel=" + accel,
    "-display", "none", "-no-reboot",
    "-drive", "file=" + guestImage() + ",if=virtio",
    "-netdev", "user,id=net0", "-device", "virtio-net-pci,netdev=net0",
  ];
  if (needSeed) {
    args.push("-smbios", "type=1,serial=ds=nocloud-net;s=http://10.0.2.2:" + server.address().port + "/");
  }
  args.push(
    "-chardev", "socket,id=qga0,host=127.0.0.1,port=" + port + ",server=on,wait=off",
    "-device", "virtio-serial-pci",
    "-device", "virtserialport,chardev=qga0,name=org.qemu.guest_agent.0",
    "-serial", "stdio"
  );
  const child = spawn(qemuExe(), args, { cwd: ROOT, stdio: "inherit" });
  let exited = false;
  child.on("exit", () => { exited = true; });
  const deadline = Date.now() + PROVISION_TIMEOUT_MS;
  let up = false;
  while (!exited && Date.now() < deadline) {
    try { await qgaSync(port); up = true; break; } catch (e) { /* 还没起来 */ }
    await new Promise((r) => setTimeout(r, 5000));
  }
  let ok = false;
  if (up) {
    log("QGA 握手成功：qemu-guest-agent 在 guest 里活着");
    ok = await work(port);
  } else {
    bad("没等到 guest 里的 qemu-guest-agent（超时或 QEMU 提前退出）——可重跑；宿主 WHPX 不可用时会明显变慢。");
  }
  try { await qgaCall(port, { execute: "guest-shutdown", arguments: { mode: "powerdown" } }, 15000); } catch (e) {}
  for (let i = 0; i < 90 && !exited; i++) await new Promise((r) => setTimeout(r, 1000));
  if (!exited) { try { child.kill(); } catch (e) {} }
  server.close();
  return ok;
}

// ---- 在 guest 里跑一条命令并等它结束（QGA guest-exec + guest-exec-status）----
async function qgaExec(port, argv, budgetMs) {
  const started = await qgaCall(port, { execute: "guest-exec", arguments: { path: argv[0], arg: argv.slice(1), "capture-output": true } }, 15000);
  const pid = started && started.pid;
  if (!pid) return false;
  const deadline = Date.now() + (budgetMs || 20 * 60 * 1000);
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 5000));
    const st = await qgaCall(port, { execute: "guest-exec-status", arguments: { pid } }, 15000);
    if (st && st.exited) {
      if (Number(st.exitcode) !== 0) {
        const out = Buffer.from(String(st["out-data"] || ""), "base64").toString("utf8");
        bad("guest 里的命令非零退出（" + st.exitcode + "）：" + out.slice(-400));
      }
      return Number(st.exitcode) === 0;
    }
  }
  return false;
}

async function provisionAgent() {
  if (agentReady()) { log("qemu-guest-agent 已在镜像里：" + path.relative(ROOT, agentMarker())); return; }
  const ok = await withGuest("首启 guest：cloud-init 装 qemu-guest-agent，装好后用 QGA 握手确认", true, async (port) => {
    // agent 活着之后顺手把 cloud-init 关掉（以后启动更快）；补装运行包走下面的维护启动。
    try { await qgaExec(port, ["/bin/sh", "-c", "touch /etc/cloud/cloud-init.disabled"], 30000); } catch (e) {}
    return true;
  });
  if (ok) {
    fs.writeFileSync(agentMarker(), "QGA guest-sync 通过（setup-vm-windows.js）\n");
    log("guest 里已确认 qemu-guest-agent：" + path.relative(ROOT, agentMarker()));
  }
}

// ---- 镜像预装运行包（1+2 的 1）：模块工具在 guest 里要用的 python / node ----
async function ensureGuestRuntimes() {
  if (runtimesReady()) { log("guest 运行包已在镜像里：" + path.relative(ROOT, runtimesMarker())); return; }
  if (!agentReady()) die("先确认 guest 里的 qemu-guest-agent（重跑不加 --check）。");
  const ok = await withGuest("维护启动：经 QGA 在 guest 里装运行包（python3 / python / nodejs）", false, async (port) => {
    // 镜像里已有首启留下的 apt 索引：先不 update 直接装；装不上再 update 重试（update 在 TCG 下很慢）。
    const cmd =
      "DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends python3 python-is-python3 nodejs || " +
      "(apt-get update -o Acquire::Languages=none && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends python3 python-is-python3 nodejs)";
    return await qgaExec(port, ["/bin/sh", "-c", cmd]);
  });
  if (ok) {
    fs.writeFileSync(runtimesMarker(), "python3 / python / nodejs（setup-vm-windows.js）\n");
    log("guest 运行包已装好：" + path.relative(ROOT, runtimesMarker()));
  } else {
    bad("guest 运行包没装成——可重跑（维护启动会重试）。");
  }
}

(async () => {
  if (CHECK) {
    const qk = qemuReady(), gk = guestReady(), ak = agentReady(), rk = runtimesReady();
    log("QEMU：" + (qk ? "就绪（" + path.relative(ROOT, qemuExe()) + "）" : "缺"));
    log("guest 镜像：" + (gk ? "就绪（" + path.relative(ROOT, guestImage()) + "）" : "缺"));
    log("镜像里的 qemu-guest-agent：" + (ak ? "已确认（QGA 握手）" : "缺"));
    log("镜像里的运行包（python3 / python / nodejs）：" + (rk ? "已装" : "缺"));
    if (!qk || !gk || !ak || !rk) {
      log("怎么补：跑 node setup-vm-windows.js --yes");
      process.exit(1);
    }
    process.exit(0);
  }
  if (!qemuReady() || !guestReady() || !agentReady() || !runtimesReady()) {
    if (!YES) {
      if (!process.stdin.isTTY) die("无交互终端：不擅自下载。重跑加 --yes。");
      const a = await ask("要把 QEMU（MSYS2 包闭包，约 200 MB）与 Debian guest 镜像（约 326 MB）装到项目内 .vm/ 吗？[y/N] ");
      if (a.trim().toLowerCase() !== "y") die("已取消。");
    }
  }
  fs.mkdirSync(VM, { recursive: true });
  ensureQemu();
  ensureGuestImage();
  await provisionAgent();
  await ensureGuestRuntimes();
  log("全部就绪。登记方式：");
  log("  · 设置 qemu_path → " + path.relative(ROOT, qemuExe()));
  log("  · 会话 exec.base → " + path.relative(ROOT, BASE));
})();
