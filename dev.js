#!/usr/bin/env node
/**
 * Solomni 开发回路：本机的单层开发命令（编译检查 / 单元测试 / 格式 / 静态检查）。
 * 管：把"这一层怎么跑"映射成固定子命令，并保证用**项目内工具链与缓存**（环境解析交给 env.js，一处口径）。
 * 不管：完整门禁与全量测试（那是 run-tests.js）、构建与启动（那是 start.js）、装环境（那是 setup-dev.js / env.js）。
 * 约束：子命令固定，**不透传任意 cargo 参数**——要跑别的组合一律用 node run-tests.js，别把闸门绕出第二条路。
 * 用法：node dev.js test | fmt | clippy（--help 看全部）；本机开发回路见 docs/testing/execution-ci.md 的「执行入口与报告」。
 */
"use strict";
const { spawnSync } = require("child_process");
const env = require("./env.js");

const ROOT = __dirname;
// 固定子命令 → cargo 参数。--color never：输出可判读、不吃终端能力（门禁与重定向同一份结果）。
const SCENARIOS = {
  test: {
    why: "L1 单元（--bin solomni）",
    args: ["test", "--color", "never", "--bin", "solomni"],
  },
  fmt: {
    why: "T0 格式（cargo fmt --check）",
    // cargo-fmt 只认自己的参数在 `--` 之前；`--color` 是 rustfmt 的，必须放在 `--` 之后。
    args: ["fmt", "--all", "--", "--check", "--color", "never"],
  },
  clippy: {
    why: "T0 静态检查（cargo clippy -D warnings）",
    args: ["clippy", "--all-targets", "--all-features", "--keep-going", "--color", "never", "--", "-D", "warnings"],
  },
};

function usage() {
  return [
    "用法：node dev.js <子命令>",
    "",
    "子命令：",
    ...Object.entries(SCENARIOS).map(([name, s]) => "  " + name.padEnd(7) + " " + s.why),
    "",
    "要跑完整门禁与全量测试：node run-tests.js（迭代时加 --skip-e2e，见 docs/testing/execution-ci.md）。",
  ].join("\n");
}

function main() {
  const argv = process.argv.slice(2);
  if (!argv.length || argv.includes("--help") || argv.includes("-h")) {
    console.log(usage());
    return argv.length ? 0 : 1;
  }
  const name = argv[0];
  const scenario = SCENARIOS[name];
  if (!scenario) {
    console.error("[dev] 没有这个子命令：" + name);
    console.error(usage());
    return 1;
  }
  const resolved = env.resolve({});
  if (!resolved) {
    console.error("[dev] 环境没就绪：跑 node env.js setup，或看 node env.js 的解析结果");
    return 1;
  }
  env.pinTemp(resolved.env);
  console.log("[dev] " + scenario.why + " → " + resolved.cargo + " " + scenario.args.join(" "));
  const r = spawnSync(resolved.cargo, scenario.args, { cwd: ROOT, stdio: "inherit", env: resolved.env });
  if (r.error) {
    console.error("[dev] 起不了 cargo：" + r.error.message);
    return 1;
  }
  return r.status === null ? 1 : r.status;
}

module.exports = { SCENARIOS, usage, main };

if (require.main === module) process.exit(main());
