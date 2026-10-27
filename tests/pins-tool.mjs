/**
 * pins.js 自检：只测解析与 URL/校验源推导，不碰网络、不写文件。
 * 判据见 docs/testing/execution-ci.md 的「依赖哈希工具自检」。
 */
import assert from "node:assert/strict";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const pins = require(path.join(ROOT, "pins.js"));

const H = "0123456789abcdef".repeat(4);

// ① 扁平台账：只认 键: 值，注释与空行跳过，重复键报错。
const flat = pins.parseFlatText("a.b: https://x/y\n# 注释\n\nc.sha256: " + H + "\n");
assert.deepEqual([...flat.entries.keys()], ["a.b", "c.sha256"]);
assert.equal(flat.entries.get("a.b").value, "https://x/y");
assert.throws(() => pins.parseFlatText("a: 1\na: 2\n"), /重复键/);
assert.throws(() => pins.parseFlatText("不是键值\n"), /不是扁平/);

// ② dependencies.yaml：条目按 - id 切块，只取 source / sha256。
const deps = pins.parseDepsText(
  "deps:\n  - id: vm.demo\n    source: release:deps-qemu-1.0/a.bin\n    sha256: " + H + "\n" +
    "  - id: vm.latest\n    source: guest.debian.image\n",
);
assert.equal(deps.blocks.length, 2);
assert.equal(deps.blocks[0].id, "vm.demo");
assert.equal(deps.blocks[0].source.value, "release:deps-qemu-1.0/a.bin");
assert.equal(deps.blocks[0].sha256.value, H);
assert.equal(deps.blocks[1].sha256, undefined);

// ③ 校验值解析：大小写归一、SHASUMS 按文件名取、Zig 按 tarball 取。
assert.equal(pins.firstSha("ABC " + H.toUpperCase() + "  file"), H);
assert.equal(pins.firstSha("没有哈希"), null);
assert.equal(pins.shaFromShasums(H + "  node.tar.xz\n", "node.tar.xz"), H);
assert.equal(pins.shaFromShasums(H + "  *node.tar.xz\n", "node.tar.xz"), H);
assert.equal(pins.shaFromShasums(H + "  other.tar.xz\n", "node.tar.xz"), null);
assert.equal(pins.findZigShasum({ "0.15.2": { "x86_64-linux": { tarball: "u", shasum: H.toUpperCase() } } }, "u"), H);
assert.equal(pins.findZigShasum({ "0.15.2": { "x86_64-linux": { tarball: "u" } } }, "u"), null);

// ④ GitHub release 坐标与自托管 source：旧 base=.../dependencies/ 与新 base=.../download/ 都要认。
const gh = pins.githubRelease("https://github.com/o/r/releases/download/t/a.bin");
assert.deepEqual(gh, { owner: "o", repo: "r", tag: "t", asset: "a.bin" });
assert.equal(pins.githubRelease("https://example.com/a.bin"), null);
let rt = pins.releaseTarget("https://github.com/o/r/releases/download/dependencies/", "release:a.bin");
assert.equal(rt.tag, "dependencies");
assert.equal(rt.asset, "a.bin");
assert.equal(rt.url, "https://github.com/o/r/releases/download/dependencies/a.bin");
rt = pins.releaseTarget("https://github.com/o/r/releases/download/", "release:deps-qemu-11.1.1-4/a.bin");
assert.equal(rt.tag, "deps-qemu-11.1.1-4");
assert.equal(rt.asset, "a.bin");

// ⑤ 链接键 → 校验源：Node 用 SHASUMS256.txt，Zig 用 index.json，Python 用 release digest。
let t = pins.linkTarget("toolchain.node.24.11.0.linux-x86_64", "https://nodejs.org/dist/v24.11.0/node-v24.11.0-linux-x64.tar.xz");
assert.equal(t.kind, "shasums");
assert.equal(t.url, "https://nodejs.org/dist/v24.11.0/SHASUMS256.txt");
assert.equal(t.name, "node-v24.11.0-linux-x64.tar.xz");
t = pins.linkTarget("toolchain.zig.0.15.2.linux-x86_64", "https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz");
assert.equal(t.kind, "zig");
assert.equal(t.url, "https://ziglang.org/download/index.json");
t = pins.linkTarget(
  "toolchain.python.3.14.9.linux-x86_64",
  "https://github.com/astral-sh/python-build-standalone/releases/download/20261021/cpython-3.14.9+20261021-x86_64-unknown-linux-gnu-install_only.tar.gz",
);
assert.equal(t.kind, "github");
assert.equal(t.owner, "astral-sh");
assert.equal(t.tag, "20261021");
assert.equal(t.asset, "cpython-3.14.9+20261021-x86_64-unknown-linux-gnu-install_only.tar.gz");
t = pins.linkTarget("smbd.windows", "https://github.com/o/r/releases/download/v1/smbd.exe");
assert.equal(t.kind, "sidecar");
assert.equal(t.url, "https://github.com/o/r/releases/download/v1/smbd.exe.sha256");
assert.equal(t.github.tag, "v1");

// ⑥ 原地重写：保留缩进与行尾空白，只换值。
assert.equal(pins.replaceTrailingValue("  sha256: old  ", H), "  sha256: " + H + "  ");
assert.equal(pins.replaceTrailingValue("key: https://x", "https://y"), "key: https://y");
assert.throws(() => pins.replaceTrailingValue("没有值", H), /没有可替换/);

// ⑦ 入口：--help 可跑且列出三个子命令。
const help = spawnSync(process.execPath, [path.join(ROOT, "pins.js"), "--help"], { encoding: "utf8" });
assert.equal(help.status, 0, help.stderr);
for (const name of ["update", "check", "bump"]) assert.ok(help.stdout.includes(name), "帮助要列出 " + name);

console.log("PINS-TOOL-OK");
