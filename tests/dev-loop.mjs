/**
 * 开发回路自检：dev.js 的固定子命令表、唯一透传口 cargo --，以及"环境没就绪就如实拒绝"这条出口。
 * 判据见 docs/testing/execution-ci.md 的「本机开发回路」：子命令固定，额外能力只有 cargo -- 一条受控透传。
 * 跑起来只 require + 两次 --help 子进程 + 一次 cargo --version 子进程；不通过输出 DEV-LOOP-FAIL。
 */
import assert from "node:assert/strict";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const dev = require(path.join(ROOT, "dev.js"));
const markers = require(path.join(ROOT, "tests", "lib", "markers.js"));

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

// ② 透传口固定为 cargo，且只认 -- 之后的参数：不另造入口，也不绕过 env.js 的环境。
assert.equal(dev.PASSTHROUGH, "cargo", "透传口固定为 cargo");
const usageText = dev.usage();
assert.ok(usageText.includes("cargo --"), "帮助里要写出透传口 cargo --");
for (const name of names) assert.ok(usageText.includes(name), "帮助里要列出 " + name);
assert.ok(usageText.includes("node run-tests.js"), "帮助要把完整入口指出来");

// ③ 真实跑一次 --help：能起、退出码 0、不碰 cargo。
const helpRun = spawnSync(process.execPath, [path.join(ROOT, "dev.js"), "--help"], { encoding: "utf8" });
assert.equal(helpRun.status, 0, "--help 要正常退出：" + (helpRun.stderr || ""));
assert.ok(helpRun.stdout.includes("test"), "--help 要列出子命令");
assert.ok(helpRun.stdout.includes("cargo --"), "--help 要写出透传口");

// ④ 透传口的两条边界：不带 -- 要拒绝；带 -- 要真的把参数交给 cargo（环境已就绪时）。
const noSep = spawnSync(process.execPath, [path.join(ROOT, "dev.js"), "cargo"], { encoding: "utf8" });
assert.equal(noSep.status, 1, "cargo 不带 -- 要以非零退出如实拒绝");
assert.ok(String(noSep.stderr).includes("cargo --"), "拒绝时要给出正确用法");
const passthrough = spawnSync(process.execPath, [path.join(ROOT, "dev.js"), "cargo", "--", "--version"], { encoding: "utf8" });
assert.equal(passthrough.status, 0, "cargo -- --version 要能跑通（环境需就绪）：" + (passthrough.stderr || ""));
assert.ok(/cargo\s+\d/.test(passthrough.stdout), "透传要真的给出 cargo 版本，而不是被 dev.js 吞掉");

// ⑤ 出口：固定子命令之外的名字要如实拒绝，不给第二条路。
const unknown = spawnSync(process.execPath, [path.join(ROOT, "dev.js"), "nope"], { encoding: "utf8" });
assert.equal(unknown.status, 1, "未知子命令要以非零退出如实拒绝");
assert.ok(String(unknown.stderr).includes("没有这个子命令"), "拒绝时要说明原因并给出用法");

console.log(markers.TEXT.DEV_LOOP_OK);
