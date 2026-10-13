#!/usr/bin/env node
/**
 * 虚拟机档本地环境安装（**Linux**）：把 QEMU 与 guest 基础根装到项目内 .vm/（不改系统、不装系统包）。
 * 管：QEMU（Arch 走 pacman -Sp 取依赖闭包）、Debian 13 基础根（cloud 镜像 → rootfs 目录 + 通用内核 + 9p initrd + qemu-guest-agent）、QEMU 包装脚本。
 * 不管：产品对 VM 档的检测 / 装配（在 src/capabilities/vm/）；Windows / macOS 的环境在 setup-vm-windows.js / setup-vm-mac.js。
 * 用法：node setup-vm-linux.js [--check] [--yes] [--offline]
 * 约束：只写项目内 .vm/；下载前要用户同意（--yes 或交互确认）；不碰系统包管理器、不写系统目录。
 * 下载源全部走 links.yaml（guest.*）：换源只改台账，不改本文件。
 */
"use strict";
const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const readline = require("readline");
const env = require("./env.js");

const ROOT = __dirname;
const VM = path.join(ROOT, ".vm");
const QEMU = path.join(VM, "qemu");
const GUEST = path.join(VM, "guest");
const BASE = path.join(GUEST, "base");
const CACHE = path.join(VM, "cache");
const PKGS = path.join(CACHE, "pkgs");

const CHECK = process.argv.includes("--check");
const YES = process.argv.includes("--yes");
const OFFLINE = process.argv.includes("--offline");
const IS_WIN = process.platform === "win32";

// Linux guest 统一 Debian stable（trixie 13）。cloud 镜像给 rootfs；通用内核给 9p；QGA 走 Debian 仓库。
const QEMU_PKGS = ["qemu-system-x86", "qemu-common", "qemu-system-x86-firmware", "qemu-img"];
const GUEST_KERNEL_ABI = "6.12.111+deb13-amd64";
const IMAGE_TAR = () => path.join(CACHE, path.basename(new URL(env.link("guest.debian.image")).pathname));
const DISK_RAW = () => path.join(CACHE, "disk.raw");
const ROOTFS_IMG = () => path.join(CACHE, "rootfs.ext4");

const log = (m) => console.log("[setup-vm] " + m);
const bad = (m) => console.error("[setup-vm] " + m);
const die = (m) => { bad(m); process.exit(1); };
const q = (s) => "'" + String(s).replace(/'/g, "'\\''") + "'";
const exists = (p) => { try { return fs.existsSync(p); } catch (e) { return false; } };
// 只看链接本身在不在：guest 里的单元软链指向 /usr/lib/…，在宿主上当然不存在，不能用 existsSync 跟过去判。
const lexists = (p) => { try { fs.lstatSync(p); return true; } catch (e) { return false; } };

function run(cmd, args, opts) {
  const r = spawnSync(cmd, args, Object.assign({ cwd: ROOT, stdio: "inherit" }, opts || {}));
  if (r.error) die(cmd + " 无法执行：" + r.error.message);
  return r.status === 0;
}
function capture(cmd, args) {
  const r = spawnSync(cmd, args, { cwd: ROOT, encoding: "utf8" });
  return { ok: !r.error && r.status === 0, text: ((r.stdout || "") + (r.stderr || "")).trim() };
}
function ask(question) {
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(question, (a) => { rl.close(); resolve(a); });
  });
}
function curlDownload(url, dest) {
  if (exists(dest) && fs.statSync(dest).size > 0) { log("已有缓存：" + path.relative(ROOT, dest)); return true; }
  if (OFFLINE) die("--offline：缺 " + path.relative(ROOT, dest) + "，需要下载 " + url);
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  log("下载 " + url);
  const curl = IS_WIN ? "curl.exe" : "curl";
  const r = spawnSync(curl, ["-L", "--fail", "--retry", "5", "--retry-all-errors", "-C", "-", "-o", dest, url], { cwd: ROOT, stdio: "inherit" });
  return !r.error && r.status === 0 && exists(dest) && fs.statSync(dest).size > 0;
}
function listUrls(url) {
  const r = capture("curl", ["-sSL", "--max-time", "90", url]);
  if (!r.ok) return [];
  return r.text.split(/\r?\n/);
}
function pickNewest(lines, re) {
  const hits = [];
  for (const l of lines) for (const m of l.matchAll(re)) hits.push(m[0]);
  const uniq = [...new Set(hits)];
  uniq.sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
  return uniq[uniq.length - 1];
}
function extractDataTar(deb, destDir) {
  // .deb 是 ar 归档：先取出成员，再把 data.tar.* 解进目标（不跑 maintainer 脚本）。
  const tmp = path.join(CACHE, "deb-x");
  fs.rmSync(tmp, { recursive: true, force: true });
  fs.mkdirSync(tmp, { recursive: true });
  const bsd = capture("bsdtar", ["--version"]).ok;
  const ok = bsd ? run("bsdtar", ["-xf", deb, "-C", tmp]) : run("ar", ["x", deb], { cwd: tmp });
  if (!ok) return false;
  const data = fs.readdirSync(tmp).find((f) => f.startsWith("data.tar"));
  if (!data) return false;
  return run("tar", ["-xf", path.join(tmp, data), "-C", destDir]);
}

// ---- QEMU ----
function qemuWrapperPath() { return path.join(QEMU, "bin", IS_WIN ? "qemu-system-x86_64.exe" : "qemu-system-x86_64"); }
function qemuReady() { return lexists(qemuWrapperPath()); }
function wrapperScript() {
  return "#!/bin/sh\n" +
    "# 项目内 QEMU 包装：把解包出来的库与固件目录喂给真正的二进制（不改系统、不进 PATH 之外）。由 setup-vm.js 生成。\n" +
    "d=$(CDPATH= cd -- \"$(dirname -- \"$0\")/..\" && pwd)\n" +
    "LD_LIBRARY_PATH=\"$d/usr/lib:$LD_LIBRARY_PATH\" exec \"$d/usr/bin/qemu-system-x86_64\" -L \"$d/usr/share/qemu\" \"$@\"\n";
}
function ensureQemu() {
  if (qemuReady()) { log("QEMU 已在项目内：" + path.relative(ROOT, qemuWrapperPath())); return; }
  const urls = capture("pacman", ["-Sp", ...QEMU_PKGS]);
  if (!urls.ok || !urls.text.includes("https://")) {
    die("本发行版暂不支持自动装 QEMU（未找到可用的 pacman 包源）。请把 QEMU 装到项目内并把可执行文件路径填进设置 qemu_path。");
  }
  const list = urls.text.split(/\r?\n/).map((s) => s.trim()).filter((s) => s.startsWith("https://"));
  fs.mkdirSync(PKGS, { recursive: true });
  for (const u of list) {
    const dest = path.join(PKGS, path.basename(new URL(u).pathname));
    if (!curlDownload(u, dest)) die("下载失败：" + u);
  }
  fs.mkdirSync(QEMU, { recursive: true });
  for (const f of fs.readdirSync(PKGS)) {
    if (!f.endsWith(".pkg.tar.zst")) continue;
    if (!run("sh", ["-c", "zstd -dc " + q(path.join(PKGS, f)) + " | tar -xf - -C " + q(QEMU) + " --no-same-owner"])) {
      die("解包失败：" + f);
    }
  }
  fs.mkdirSync(path.join(QEMU, "bin"), { recursive: true });
  fs.writeFileSync(qemuWrapperPath(), wrapperScript(), { mode: 0o755 });
  fs.chmodSync(qemuWrapperPath(), 0o755);
  if (!capture(qemuWrapperPath(), ["-version"]).ok) die("QEMU 装好了但起不来：" + qemuWrapperPath());
  log("QEMU 就绪：" + path.relative(ROOT, qemuWrapperPath()));
}

// ---- guest 基础根 ----
function fstabSanitized() {
  const f = path.join(BASE, "etc", "fstab");
  return exists(f) && !fs.readFileSync(f, "utf8").includes("PARTUUID=");
}
function baseReady() {
  return lexists(path.join(BASE, "vmlinuz"))
    && lexists(path.join(BASE, "initrd.img"))
    && exists(path.join(BASE, "usr", "sbin", "qemu-ga"))
    && lexists(path.join(BASE, "etc", "systemd", "system", "multi-user.target.wants", "qemu-guest-agent.service"))
    && fstabSanitized();
}
function ensureImage() {
  const tar = IMAGE_TAR();
  if (!exists(tar)) { if (!curlDownload(env.link("guest.debian.image"), tar)) die("镜像下载失败"); }
  if (!exists(DISK_RAW())) {
    log("解开镜像 tar（disk.raw）");
    if (!run("tar", ["-xf", tar, "-C", CACHE])) die("解镜像 tar 失败");
  }
  if (!exists(ROOTFS_IMG())) {
    // 从 GPT 里找 Linux root 分区（type GUID 4F68BCE3-…），取出成独立 ext4 镜像。
    const j = capture("sfdisk", ["-J", DISK_RAW()]);
    if (!j.ok) die("读不了分区表（需要 util-linux 的 sfdisk）");
    let table; try { table = JSON.parse(j.text); } catch (e) { die("sfdisk 输出不是 JSON"); }
    const parts = table.partitiontable.partitions || [];
    const root = parts.find((p) => p.type === "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709") || parts[0];
    log("取出 root 分区（start=" + root.start + " size=" + root.size + "）");
    if (!run("dd", ["if=" + DISK_RAW(), "of=" + ROOTFS_IMG(), "bs=512", "skip=" + root.start, "count=" + root.size, "status=none"])) {
      die("dd 取分区失败");
    }
  }
}
function ensureRootfs() {
  if (exists(path.join(BASE, "etc", "os-release"))) return;
  log("把 rootfs 解到 " + path.relative(ROOT, BASE) + "（debugfs rdump）");
  fs.mkdirSync(BASE, { recursive: true });
  if (!run("debugfs", ["-R", "rdump / " + BASE, ROOTFS_IMG()])) die("debugfs 解 rootfs 失败");
}
function ensureKernel() {
  if (exists(path.join(BASE, "boot", "config-" + GUEST_KERNEL_ABI))) return;
  const lines = listUrls(env.link("guest.debian.linux"));
  const name = pickNewest(lines, /linux-image-[0-9][^"]*-amd64-unsigned_[^"]*_amd64\.deb/g);
  if (!name) die("在 kernel 源里没找到 amd64 内核包");
  const deb = path.join(CACHE, name);
  if (!curlDownload(env.link("guest.debian.linux") + name, deb)) die("内核下载失败");
  log("装入通用内核（带 9p）：" + name);
  if (!extractDataTar(deb, BASE)) die("内核解包失败");
}
function ensureQga() {
  if (!exists(path.join(BASE, "usr", "sbin", "qemu-ga"))) installQgaBinary();
  const wants = path.join(BASE, "etc", "systemd", "system", "multi-user.target.wants");
  fs.mkdirSync(wants, { recursive: true });
  const unit = path.join(wants, "qemu-guest-agent.service");
  if (!lexists(unit)) fs.symlinkSync("/usr/lib/systemd/system/qemu-guest-agent.service", unit);
}
function installQgaBinary() {
  const lines = listUrls(env.link("guest.debian.pool"));
  let name = pickNewest(lines, /qemu-guest-agent_[^"]*\+deb13[^"]*_amd64\.deb/g);
  if (!name) name = pickNewest(lines, /qemu-guest-agent_[^"]*_amd64\.deb/g);
  if (!name) die("在 qemu 源里没找到 guest agent 包");
  const deb = path.join(CACHE, name);
  if (!curlDownload(env.link("guest.debian.pool") + name, deb)) die("qemu-guest-agent 下载失败");
  log("装入 qemu-guest-agent：" + name);
  if (!extractDataTar(deb, BASE)) die("qemu-guest-agent 解包失败");
}
function sanitizeBase() {
  // 9p root 没有 PARTUUID 对应的块设备：留着 cloud 镜像的 fstab 会让 local-fs.target 掉进 emergency。
  fs.writeFileSync(path.join(BASE, "etc", "fstab"), "# guest root 由内核命令行给出（9p），这里不写 fstab 条目。\n");
  const sys = path.join(BASE, "etc", "systemd", "system");
  fs.mkdirSync(sys, { recursive: true });
  const mask = path.join(sys, "systemd-remount-fs.service");
  if (!exists(mask)) fs.symlinkSync("/dev/null", mask);
}
function ensureInitrd() {
  const conf = path.join(BASE, "etc", "initramfs-tools", "modules");
  fs.mkdirSync(path.dirname(conf), { recursive: true });
  const need = ["9p", "9pnet", "9pnet_virtio", "virtio", "virtio_pci", "virtio_ring"];
  const cur = exists(conf) ? fs.readFileSync(conf, "utf8") : "";
  const lines = cur.split(/\r?\n/).filter(Boolean);
  let changed = false;
  for (const m of need) if (!lines.includes(m)) { lines.push(m); changed = true; }
  if (changed) fs.writeFileSync(conf, lines.join("\n") + "\n");
  const scriptPath = path.join(BASE, "tmp", "solomni-initrd.sh");
  fs.mkdirSync(path.dirname(scriptPath), { recursive: true });
  fs.writeFileSync(scriptPath, "#!/bin/bash\nset -e\ndepmod " + GUEST_KERNEL_ABI + "\nmkinitramfs -o /boot/initrd.img-" + GUEST_KERNEL_ABI + " " + GUEST_KERNEL_ABI + "\n");
  log("在 chroot（用户命名空间，无需 root）里生成带 9p 的 initrd");
  const inner = "mount --bind /proc " + q(path.join(BASE, "proc")) + " 2>/dev/null || true; chroot " + q(BASE) + " /bin/bash /tmp/solomni-initrd.sh";
  if (!run("unshare", ["-r", "-m", "sh", "-c", inner])) die("生成 initrd 失败（需要内核允许非特权用户命名空间）");
  fs.rmSync(scriptPath, { force: true });
  if (!exists(path.join(BASE, "boot", "initrd.img-" + GUEST_KERNEL_ABI))) die("initrd 没生成出来");
}
function linkKernel() {
  const v = path.join(BASE, "vmlinuz");
  if (!lexists(v)) fs.symlinkSync("boot/vmlinuz-" + GUEST_KERNEL_ABI, v);
  const i = path.join(BASE, "initrd.img");
  if (!lexists(i)) fs.symlinkSync("boot/initrd.img-" + GUEST_KERNEL_ABI, i);
}
function ensurePythonAlias() {
  // Debian 只提供 python3；模块工具命令常写 `python`，给 guest 补上这个别名（等价 python-is-python3）。
  const py3 = path.join(BASE, "usr", "bin", "python3");
  const py = path.join(BASE, "usr", "bin", "python");
  if (exists(py3) && !lexists(py)) fs.symlinkSync("python3", py);
}
function ensureBase() {
  if (!baseReady()) {
    ensureImage();
    ensureRootfs();
    ensureKernel();
    ensureQga();
    sanitizeBase();
    ensureInitrd();
    linkKernel();
  }
  ensurePythonAlias();
  if (!baseReady()) die("基础根还没就绪，请检查 .vm/guest/base");
  log("guest 基础根就绪：" + path.relative(ROOT, BASE));
}

(async () => {
  if (CHECK) {
    const qk = qemuReady(), bk = baseReady();
    log("QEMU：" + (qk ? "就绪（" + path.relative(ROOT, qemuWrapperPath()) + "）" : "缺（跑 node setup-vm-linux.js 装到项目内）"));
    log("guest 基础根：" + (bk ? "就绪（" + path.relative(ROOT, BASE) + "）" : "缺（跑 node setup-vm-linux.js 装到项目内）"));
    if (!qk || !bk) process.exit(1);
    return;
  }
  const needDownload = !qemuReady() || !baseReady();
  if (needDownload && !YES) {
    if (!process.stdin.isTTY) die("无交互终端：不擅自下载。重跑加 --yes，或先手动准备。");
    const a = await ask("要把 QEMU 与 Debian guest 基础根下载到项目内 .vm/（约 400 MB，只写项目内）吗？[y/N] ");
    if (a.trim().toLowerCase() !== "y") die("已取消。");
  }
  fs.mkdirSync(VM, { recursive: true });
  ensureQemu();
  ensureBase();
  log("全部就绪。登记方式：");
  log("  · 设置 qemu_path → " + path.relative(ROOT, qemuWrapperPath()));
  log("  · 会话 exec.base → " + path.relative(ROOT, BASE));

})();
