#!/usr/bin/env node
/**
 * 目的：虚拟机档本地环境安装的**共享件**——路径、下载、解包、提问、命令执行与统一提示。
 * 管：项目内 .vm/ 的路径布局；带续传的下载；调产品的 --unpack 解归档；统一的中文提示。
 * 不管：各平台装什么、装到哪（在 setup-vm-<平台>.js）；产品对 VM 档的检测（在 src/capabilities/vm/）。
 * 联动：被 setup-vm-linux.js / setup-vm-windows.js / setup-vm-mac.js 与分发入口 setup-vm.js 使用；下载源走 links.yaml。
 * 用法：被上面几个入口 require，不单独运行。
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
const TOOLS = path.join(VM, "tools");

let offline = false;
function setOffline(v) { offline = !!v; }

const log = (m) => console.log("[setup-vm] " + m);
const bad = (m) => console.error("[setup-vm] " + m);
const die = (m) => { bad(m); process.exit(1); };
const q = (s) => "'" + String(s).replace(/'/g, "'\\''") + "'";
const exists = (p) => { try { return fs.existsSync(p); } catch (e) { return false; } };
// 只看链接本身在不在：guest 里的软链指向 /usr/lib/…，在宿主上跟不过去。
const lexists = (p) => { try { fs.lstatSync(p); return true; } catch (e) { return false; } };
const link = (k) => env.link(k);

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
  if (offline) die("--offline：缺 " + path.relative(ROOT, dest) + "，需要下载 " + url);
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  log("下载 " + url);
  const curl = process.platform === "win32" ? "curl.exe" : "curl";
  const r = spawnSync(curl, ["-L", "--fail", "--retry", "5", "--retry-all-errors", "-C", "-", "-o", dest, url], { cwd: ROOT, stdio: "inherit" });
  return !r.error && r.status === 0 && exists(dest) && fs.statSync(dest).size > 0;
}
function productBinary() {
  return path.join(ROOT, "target", "debug", process.platform === "win32" ? "solomni.exe" : "solomni");
}
function ensureProduct() {
  const exe = productBinary();
  if (exists(exe)) return exe;
  const r = env.resolve({});
  if (!r) die("找不到 Rust 工具链：先跑 node setup-dev.js");
  log("先构建产品（解包要用它）");
  const b = spawnSync(r.cargo, ["build"], { cwd: ROOT, env: r.env, stdio: "inherit" });
  if (b.status !== 0 || !exists(exe)) die("构建失败");
  return exe;
}
function unpackArchive(archive, dest) {
  const exe = ensureProduct();
  fs.mkdirSync(dest, { recursive: true });
  const r = spawnSync(exe, ["--unpack", archive, dest], { cwd: ROOT, stdio: "inherit" });
  return r.status === 0;
}

module.exports = {
  ROOT, VM, QEMU, GUEST, BASE, CACHE, TOOLS,
  log, bad, die, q, exists, lexists, link, run, capture, ask,
  curlDownload, setOffline, productBinary, ensureProduct, unpackArchive,
};
