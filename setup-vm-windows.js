#!/usr/bin/env node
/**
 * 目的：虚拟机档本地环境安装（**Windows**）——把 QEMU（MSYS2 mingw64 包）与 Debian guest 装到项目内 .vm/。
 * 管：QEMU 依赖闭包解析与解包（纯 Rust 解包）、Debian cloud 镜像（qcow2）下载、首启经 cloud-init 装 qemu-guest-agent、guest.yaml。
 * 不管：产品对 VM 档的检测 / 装配（在 src/capabilities/vm/）；Linux / macOS 的环境在 setup-vm-linux.js / setup-vm-mac.js。
 * 用法：node setup-vm-windows.js [--check] [--yes] [--offline]
 * 约束：只写项目内 .vm/；下载前要用户同意（--yes 或交互确认）；不写系统目录、不装系统包、不碰注册表。
 * 下载源：QEMU 走本机 MSYS2 的包库（或 links.yaml 的 msys2.mirror），镜像走 links.yaml 的 guest.debian.qcow2。
 */
"use strict";
const fs = require("fs");
const http = require("http");
const path = require("path");
const { spawn, spawnSync } = require("child_process");
const c = require("./setup-vm-common.js");

const CHECK = process.argv.includes("--check");
const YES = process.argv.includes("--yes");
const OFFLINE = process.argv.includes("--offline");
c.setOffline(OFFLINE);
const { ROOT, VM, QEMU, BASE, CACHE, log, bad, die, ask, exists, lexists, link, capture, curlDownload, unpackArchive } = c;

const QEMU_PKG = "mingw-w64-x86_64-qemu";
const PACMAN_CANDIDATES = ["C:\\msys64\\usr\\bin\\pacman.exe", "C:\\msys64\\usr\\bin\\pacman"];
const PROVISION_TIMEOUT_MS = 40 * 60 * 1000;

const qemuExe = () => path.join(QEMU, "mingw64", "bin", "qemu-system-x86_64.exe");
const qemuReady = () => exists(qemuExe());
const guestImage = () => path.join(BASE, "rootfs.qcow2");
const guestYaml = () => path.join(BASE, "guest.yaml");
const agentMarker = () => path.join(BASE, ".qga-ready");
const agentReady = () => exists(agentMarker());
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

// ---- 首启装 qemu-guest-agent ----
const USER_DATA = (port) =>
  [
    "#cloud-config",
    "package_update: true",
    "packages:",
    "  - qemu-guest-agent",
    "runcmd:",
    // 信号用 python3（cloud-init 自己的运行时，必然在），不赌镜像里有没有 curl / wget。
    "  - >-",
    "    systemctl enable --now qemu-guest-agent >/dev/null 2>&1 &&",
    "    python3 -c \"import urllib.request as u; u.urlopen('http://10.0.2.2:" + port + "/done').read()\" ||",
    "    python3 -c \"import urllib.request as u; u.urlopen('http://10.0.2.2:" + port + "/failed').read()\"",
    "  - touch /etc/cloud/cloud-init.disabled",
    "  - poweroff",
    "",
  ].join("\n");
const META_DATA = "instance-id: solomni-provision\nlocal-hostname: solomni-guest\n";

function startSeedServer() {
  const state = { result: null };
  const server = http.createServer((req, res) => {
    const u = req.url.split("?")[0];
    if (u === "/done") { state.result = "done"; res.writeHead(200); res.end("ok"); return; }
    if (u === "/failed") { state.result = "failed"; res.writeHead(200); res.end("ok"); return; }
    if (u === "/user-data") { res.writeHead(200); res.end(USER_DATA(server.address().port)); return; }
    if (u === "/meta-data") { res.writeHead(200); res.end(META_DATA); return; }
    res.writeHead(404); res.end("");
  });
  return new Promise((resolve) => {
    server.listen(0, "0.0.0.0", () => resolve({ server, state }));
  });
}
function provisionAgent() {
  if (agentReady()) { log("qemu-guest-agent 已在镜像里：" + path.relative(ROOT, agentMarker())); return; }
  if (!qemuReady()) die("先装 QEMU（重跑不加 --check）。");
  if (!exists(guestImage())) die("先下载 guest 镜像（重跑不加 --check）。");
  return startSeedServer().then(
    ({ server, state }) =>
      new Promise((resolve) => {
        const port = server.address().port;
        const accel = process.env.SOLOMNI_QEMU_ACCEL || "whpx:tcg";
        log("首启 guest 装 qemu-guest-agent（cloud-init 数据源 http://10.0.2.2:" + port + "/，加速 " + accel + "）");
        const args = [
          "-m", "2048", "-smp", "2", "-machine", "accel=" + accel,
          "-display", "none", "-no-reboot",
          "-drive", "file=" + guestImage() + ",if=virtio",
          "-netdev", "user,id=net0", "-device", "virtio-net-pci,netdev=net0",
          "-smbios", "type=1,serial=ds=nocloud-net;s=http://10.0.2.2:" + port + "/",
          "-serial", "stdio",
        ];
        const child = spawn(qemuExe(), args, { cwd: ROOT, stdio: "inherit" });
        const timer = setTimeout(() => {
          bad("首启超时（" + Math.round(PROVISION_TIMEOUT_MS / 60000) + " 分钟）：已中止。可重跑；镜像里若已装上 agent 就会跳过。");
          try { child.kill(); } catch (e) {}
        }, PROVISION_TIMEOUT_MS);
        child.on("exit", () => {
          clearTimeout(timer);
          server.close();
          if (state.result === "done") {
            fs.writeFileSync(agentMarker(), "qemu-guest-agent installed by setup-vm-windows.js\n");
            log("guest 里已装好 qemu-guest-agent：" + path.relative(ROOT, agentMarker()));
          } else {
            bad("首启结束但没收到 agent 安装成功的信号（收到：" + state.result + "）。可重跑，或进 guest 手动装 qemu-guest-agent。");
          }
          resolve();
        });
      })
  );
}

(async () => {
  if (CHECK) {
    const qk = qemuReady(), gk = guestReady(), ak = agentReady();
    log("QEMU：" + (qk ? "就绪（" + path.relative(ROOT, qemuExe()) + "）" : "缺"));
    log("guest 镜像：" + (gk ? "就绪（" + path.relative(ROOT, guestImage()) + "）" : "缺"));
    log("镜像里的 qemu-guest-agent：" + (ak ? "就绪" : "缺"));
    if (!qk || !gk || !ak) {
      log("怎么补：跑 node setup-vm-windows.js --yes");
      process.exit(1);
    }
    process.exit(0);
  }
  const need = !qemuReady() || !guestReady() || !agentReady();
  if (need && !YES) {
    if (!process.stdin.isTTY) die("无交互终端：不擅自下载。重跑加 --yes。");
    const a = await ask("要把 QEMU（MSYS2 包闭包，约 200 MB）与 Debian guest 镜像（约 326 MB）装到项目内 .vm/ 吗？[y/N] ");
    if (a.trim().toLowerCase() !== "y") die("已取消。");
  }
  fs.mkdirSync(VM, { recursive: true });
  ensureQemu();
  ensureGuestImage();
  await provisionAgent();
  log("全部就绪。登记方式：");
  log("  · 设置 qemu_path → " + path.relative(ROOT, qemuExe()));
  log("  · 会话 exec.base → " + path.relative(ROOT, BASE));
})();
