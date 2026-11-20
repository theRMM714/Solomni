/**
 * 开发回路自检：dev.js 的固定子命令表、通用透传口 `run --`、宿主开关 `--ambient`，以及"环境没就绪就如实拒绝"这条出口。
 * 判据见 docs/testing/execution-ci.md 的「本机开发回路」：子命令固定，额外能力只有 `run --` 一条受控透传；宿主环境必须显式。
 * 跑起来只 require + 若干子进程（--help / 拒绝 / run -- / --ambient）；不通过输出 DEV-LOOP-FAIL。
 */
import assert from "node:assert/strict";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const dev = require(path.join(ROOT, "dev.js"));
const env = require(path.join(ROOT, "env.js"));
const markers = require(path.join(ROOT, "tests", "lib", "markers.js"));

const devPath = path.join(ROOT, "dev.js");
const runDev = (args) => spawnSync(process.execPath, [devPath, ...args], { encoding: "utf8" });

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

// ② 透传口固定为 `run --`，宿主环境固定为 `--ambient`：帮助里要写全，不给第二条绕开 env.js 的路。
assert.equal(dev.RUN, "run", "透传口固定为 run");
assert.equal(dev.AMBIENT, "--ambient", "宿主开关固定为 --ambient");
const usageText = dev.usage();
assert.ok(usageText.includes("run --"), "帮助里要写出透传口 run --");
assert.ok(usageText.includes("--ambient"), "帮助里要写出宿主开关 --ambient");
assert.ok(usageText.includes("只给对照排查用"), "帮助要说明 --ambient 不是验收路径");
for (const name of names) assert.ok(usageText.includes(name), "帮助里要列出 " + name);
assert.ok(usageText.includes("node run-tests.js"), "帮助要把完整入口指出来");

// ③ 真实跑一次 --help：能起、退出码 0、不碰 cargo。
const helpRun = runDev(["--help"]);
assert.equal(helpRun.status, 0, "--help 要正常退出：" + (helpRun.stderr || ""));
assert.ok(helpRun.stdout.includes("test"), "--help 要列出子命令");
assert.ok(helpRun.stdout.includes("run --"), "--help 要写出透传口");

// ④ 透传口的两条边界：不带 -- 要拒绝；带 -- 要真的把参数交给命令（环境已就绪时）。
const noSep = runDev(["run"]);
assert.equal(noSep.status, 1, "run 不带 -- 要以非零退出如实拒绝");
assert.ok(String(noSep.stderr).includes("run --"), "拒绝时要给出正确用法");
const passthrough = runDev(["run", "--", "cargo", "--version"]);
assert.equal(passthrough.status, 0, "run -- cargo --version 要能跑通（环境需就绪）：" + (passthrough.stderr || ""));
assert.ok(/cargo\s+\d/.test(passthrough.stdout), "透传要真的给出 cargo 版本，而不是被 dev.js 吞掉");

// ⑤ 默认注入项目环境：CARGO_HOME 必须是 env.js 解析出的项目内路径。
const resolved = env.resolve({});
assert.ok(resolved, "环境需就绪（跑 node env.js setup）");
const probe = ["run", "--", "node", "-e", "process.stdout.write(process.env.CARGO_HOME || '')"];
const injected = runDev(probe);
assert.equal(injected.status, 0, "默认透传要能起 node：" + (injected.stderr || ""));
// dev.js 会先打一行「[dev] 透传 … → …」，命令自己的输出在最后一行。
const injectedOut = String(injected.stdout).trim().split(/\r?\n/).pop();
assert.equal(injectedOut, resolved.env.CARGO_HOME, "默认透传要注入 env.js 的 CARGO_HOME");

// ⑥ 宿主开关要显式：--ambient 不再注入（CARGO_HOME 保持继承值），并在 stderr 说清这是排查路径。
const ambient = runDev(["--ambient", ...probe]);
assert.equal(ambient.status, 0, "--ambient 要能起 node：" + (ambient.stderr || ""));
assert.equal(ambient.stdout, process.env.CARGO_HOME || "", "--ambient 不该改写 CARGO_HOME");
assert.ok(String(ambient.stderr).includes("宿主"), "--ambient 要在 stderr 说明用了宿主环境");

// ⑦ 出口：旧入口与未知名字要如实拒绝，不给第二条路。
const legacy = runDev(["cargo", "--", "--version"]);
assert.equal(legacy.status, 1, "旧的 cargo -- 入口要如实拒绝");
assert.ok(String(legacy.stderr).includes("没有这个子命令"), "拒绝时要说明原因并给出用法");
const unknown = runDev(["nope"]);
assert.equal(unknown.status, 1, "未知子命令要以非零退出如实拒绝");
assert.ok(String(unknown.stderr).includes("没有这个子命令"), "拒绝时要说明原因并给出用法");

console.log(markers.TEXT.DEV_LOOP_OK);
