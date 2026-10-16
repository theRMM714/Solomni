/**
 * 开发回路自检：dev.js 的固定子命令表与它的"环境没就绪就如实拒绝"这条出口。
 * 判据见 docs/testing/execution-ci.md 的「本机开发回路」：子命令固定、不透传任意 cargo 参数。
 * 跑起来不建任何东西（只读 require + 一次 --help 子进程）；不通过输出 DEV-LOOP-FAIL。
 */
import assert from "node:assert/strict";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const dev = require(path.join(ROOT, "dev.js"));

// ① 子命令表：固定、唯一、每个都能说出它是哪一层（列子命令就是在列"闸门里的哪一步"）。
const names = Object.keys(dev.SCENARIOS);
assert.deepEqual(names, ["test", "fmt", "clippy"], "子命令表固定为 test / fmt / clippy");
for (const name of names) {
  const s = dev.SCENARIOS[name];
  assert.equal(typeof name, "string");
  assert.ok(s.why, name + " 要说清它对应哪一层");
  assert.ok(Array.isArray(s.args) && s.args.length >= 2, name + " 要有具体 cargo 参数");
  assert.equal(s.args[0], name, name + " 的 cargo 子命令要与子命令同名");
  assert.ok(!s.args.includes(";") && !s.args.includes("|"), name + " 的参数里不该有 shell 拼接");
}

// ② 帮助：列全子命令，说不接受第二个参数（不透传任意 cargo 参数）。
const help = dev.usage();
for (const name of names) assert.ok(help.includes(name), "帮助里要列出 " + name);
assert.ok(help.includes("node run-tests.js"), "帮助要把完整入口指出来");

// ③ 真实跑一次 --help：能起、退出码 0、不碰 cargo。
const helpRun = spawnSync(process.execPath, [path.join(ROOT, "dev.js"), "--help"], { encoding: "utf8" });
assert.equal(helpRun.status, 0, "--help 要正常退出：" + (helpRun.stderr || ""));
assert.ok(helpRun.stdout.includes("test"), "--help 要列出子命令");

// ④ 出口：环境没就绪要如实拒绝（不猜、不回落系统 cargo）。
const savedPath = process.env.PATH;
process.env.PATH = "";
const savedCargo = process.env.SOLOMNI_CARGO;
delete process.env.SOLOMNI_CARGO;
let code;
try {
  code = dev.main(["test"]);
} finally {
  process.env.PATH = savedPath;
  if (savedCargo !== undefined) process.env.SOLOMNI_CARGO = savedCargo;
}
assert.equal(code, 1, "环境没就绪时要以非零退出如实拒绝");

console.log("DEV-LOOP-OK");
