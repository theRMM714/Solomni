// CI 的 e2e job 入口（契约见 docs/testing/execution-ci.md）：每个平台恰好两个 job，本脚本承担
// L4 端到端那一半。它构建产品、跑编排器，写一份 e2e-report.json 片段；发布 job 用
// tests/ci-merge.mjs 把它并进该平台唯一的 test-report.json。判据与 run-tests.js 的 L4 步骤一致：
// 退出码 0 且输出含 E2E-OK 才算通过。构建失败、报告缺失都算失败，不静默跳过。
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const IS_WIN = process.platform === "win32";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const BIN = path.join(ROOT, "target", "debug", EXE);
const LOG_DIR = path.join(ROOT, "target", "test-logs");
const REPORT = path.join(ROOT, "target", "e2e-report.json");
fs.mkdirSync(LOG_DIR, { recursive: true });

// 输出走文件而不是管道：受限环境里"用管道抓子进程输出"会 EPERM（与 run-tests.js 的 sh 同一条约束）。
// 环境必须带上真机围栏开关：编排器与产品都从这两个变量判断能不能真写 ACL / 建容器 profile，
// 不带就等于把真机 e2e 悄悄降成安全模式（那正是 CI 要给出的结论）。本地请用 node run-tests.js --fence-live。
const ENV = Object.assign({}, process.env, { SOLOMNI_FENCE_LIVE: "1", SOLOMNI_FENCE_WRITE: "1" });

function run(cmd, args, logName) {
  const logFile = path.join(LOG_DIR, logName);
  const fd = fs.openSync(logFile, "w");
  process.stdout.write("    实时日志：" + path.relative(ROOT, logFile) + "\n");
  const r = spawnSync(cmd, args, { cwd: ROOT, env: ENV, stdio: ["ignore", fd, fd] });
  fs.closeSync(fd);
  return { code: r.error ? -1 : r.status, out: fs.readFileSync(logFile, "utf8"), error: r.error ? String(r.error.message) : null };
}

/** 探针级跳过（[探针] 前缀），与 run-tests.js 的 skipsIn 同一口径。 */
const probes = (out) => out.split(/\r?\n/).filter((l) => l.includes("[探针]")).map((l) => l.trim());

function main() {
  const start = Date.now();
  let status = "pass";
  let detail = "";
  let raw = null;
  const envSkips = [];

  const build = run("cargo", ["build", "--color", "never"], "e2e-cargo-build.log");
  envSkips.push(...probes(build.out));
  if (build.code !== 0) {
    status = "fail";
    detail = "cargo build 失败：" + (build.error || "") + "（日志 target/test-logs/e2e-cargo-build.log）";
    raw = build.out.slice(-800);
  } else if (!fs.existsSync(BIN)) {
    status = "fail";
    detail = "cargo build 成功但找不到二进制 " + path.relative(ROOT, BIN);
  } else {
    const orchestrator = path.join(ROOT, "tests", "cross-platform", "e2e", "orchestrator.js");
    if (!fs.existsSync(orchestrator)) {
      status = "gap";
      detail = "cross-platform.e2e.not-in-runner（编排尚未迁入）";
    } else {
      const r = run(process.execPath, [orchestrator], "e2e-orchestrator.log");
      envSkips.push(...probes(r.out));
      status = r.code === 0 && r.out.includes("E2E-OK") ? "pass" : "fail";
      detail = r.out.trim().split(/\r?\n/).slice(-2).join(" / ");
      if (status === "fail") raw = r.out.slice(-800);
    }
  }

  const report = {
    platform: process.platform,
    arch: process.arch,
    osKey: IS_WIN ? "windows" : process.platform === "darwin" ? "macos" : "linux",
    steps: [{ step: "L4 端到端", status, detail, ms: Date.now() - start }],
    envSkips,
    failed: status === "fail" ? 1 : 0,
    quality: { failed: 0, steps: [] },
  };
  fs.mkdirSync(path.dirname(REPORT), { recursive: true });
  fs.writeFileSync(REPORT, JSON.stringify(report, null, 2));

  console.log("");
  console.log("=== L4 端到端（" + process.platform + " " + process.arch + "）===");
  console.log("  " + status.padEnd(13) + " L4 端到端  " + detail);
  if (raw) console.log(raw);
  if (status === "fail") {
    console.log("TEST-REPORT-FAIL");
    process.exit(1);
  }
  // gap = 编排还没迁入：如实记缺口，不冒用"端到端完成"的标记（与 run-tests.js 的 gap 语义一致）。
  if (status === "pass") console.log("E2E-OK");
  else console.log("[gap] L4 端到端未跑（编排未迁入）");
}

main();
