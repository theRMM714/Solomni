#!/usr/bin/env node
/**
 * Solomni 开发回路：本机的单层开发命令（编译检查 / 单元测试 / 格式 / 静态检查）。
 * 管：把"这一层怎么跑"映射成固定子命令，并保证用**项目内工具链与缓存**（环境解析交给 env.js，一处口径）；
 *     唯一的透传口 `cargo -- <参数>` 也走同一份环境解析，让只读查询 / 单例测试不必另造入口。
 * 不管：完整门禁与全量测试（那是 run-tests.js）、构建与启动（那是 start.js）、装环境（那是 setup-dev.js / env.js）。
 * 约束：固定子命令之外只有 `cargo -- <参数>` 一条透传口；两者都注入 env.js 的环境，**裸 shell 里的 cargo 仍不可用**。
 * 用法：node dev.js test | fmt | clippy（--help 看全部）；本机开发回路见 docs/testing/execution-ci.md 的「执行入口与报告」。
 */
"use strict";
const { spawnSync } = require("child_process");
const env = require("./env.js");

const ROOT = __dirname;
// 透传口：只认 `--` 之后的参数。它不改变环境解析，也不给第二条"绕过 env.js"的路。
const PASSTHROUGH = "cargo";
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
    "透传：",
    "  cargo -- <参数>  任意 cargo 子命令（环境与缓存仍钉在项目内；不要绕过它裸跑 cargo）",
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
  let args;
  let why;
  if (name === PASSTHROUGH) {
    // 只认 `node dev.js cargo -- <参数>`：别名 / 额外前缀一律拒绝，避免长成第二个命令入口。
    const sep = argv.indexOf("--");
    if (sep !== 1 || sep === argv.length - 1) {
      console.error("[dev] 用法：node dev.js cargo -- <cargo 参数>");
      return 1;
    }
    args = argv.slice(sep + 1);
    why = "透传 cargo（环境、缓存与临时目录仍钉在项目内）";
  } else {
    const scenario = SCENARIOS[name];
    if (!scenario) {
      console.error("[dev] 没有这个子命令：" + name);
      console.error(usage());
      return 1;
    }
    args = scenario.args;
    why = scenario.why;
  }
  const resolved = env.resolve({});
  if (!resolved) {
    console.error("[dev] 环境没就绪：跑 node env.js setup，或看 node env.js 的解析结果");
    return 1;
  }
  // 借用的工具链要先真的能构建，否则 cargo 会在链接阶段才报一句看不出原因的错。
  // 显式指定（SOLOMNI_CARGO）不在这里拦：与 env.js 同一口径，只在那边告警。
  if (resolved.source === "ambient") {
    const p = env.probeBuild(resolved);
    if (!p.ok) {
      console.error("[dev] 借用的工具链构建不出二进制（host " + (p.host || "读不到") + "）：");
      for (const line of (p.detail || "").split(/\r?\n/).filter(Boolean).slice(-3)) console.error("[dev]   " + line);
      console.error("[dev] 跑 node env.js setup 装项目内工具链（Windows 用 GNU，不需要 Visual Studio）；node env.js 看探测结论。");
      return 1;
    }
  }
  env.pinTemp(resolved.env);
  console.log("[dev] " + why + " → " + resolved.cargo + " " + args.join(" "));
  const r = spawnSync(resolved.cargo, args, { cwd: ROOT, stdio: "inherit", env: resolved.env });
  if (r.error) {
    console.error("[dev] 起不了 cargo：" + r.error.message);
    return 1;
  }
  return r.status === null ? 1 : r.status;
}

module.exports = { SCENARIOS, PASSTHROUGH, usage, main };

if (require.main === module) process.exit(main());
