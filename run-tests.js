#!/usr/bin/env node
/**
 * 测试总入口（见 TESTING.md）：先问本机事实（solomni --doctor），再逐目标点名跑，最后汇总四态。
 * 直接跑，或经 node start.js -test（后者会先把项目内工具链环境备好，再调本脚本）。
 */
"use strict";
const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const ROOT = __dirname;
const IS_WIN = process.platform === "win32";
const OS_KEY = IS_WIN ? "windows" : process.platform === "darwin" ? "macos" : "linux";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const PROFILE = process.argv.includes("--release") ? "release" : "debug";
const BIN = path.join(ROOT, "target", PROFILE, EXE);
const PLATFORM_TARGETS = ["cross-platform", "windows", "linux", "macos"];
// 真机围栏测试（会改本机状态：建 AppContainer profile、写目录 ACL）默认不跑，必须显式开启。
const FENCE_LIVE = process.argv.includes("--fence-live") || process.env.SOLOMNI_FENCE_LIVE === "1";
const REPORT = path.join(ROOT, "target", "test-report.json");

function buildEnv() {
  // start.js -test 会传好现成的环境；直接跑时尽力指向项目内工具链（不碰系统安装）。
  const e = Object.assign({}, process.env);
  // 把开关传给测试与产品：默认"不写本机状态"，只有 --fence-live 才允许。
  e.SOLOMNI_FENCE_LIVE = FENCE_LIVE ? "1" : "0";
  e.SOLOMNI_FENCE_WRITE = FENCE_LIVE ? "1" : "0";
  const osDir = path.join(ROOT, "platform", IS_WIN ? "windows" : "linux");
  // 项目内工具链存在就用它（本地收敛原则）；不存在（例如 CI runner）就用环境里现成的。
  const localCargo = path.join(osDir, "cargo");
  const localRustup = path.join(osDir, "rustup");
  if (!e.CARGO_HOME && fs.existsSync(localCargo)) e.CARGO_HOME = localCargo;
  if (!e.RUSTUP_HOME && fs.existsSync(localRustup)) e.RUSTUP_HOME = localRustup;
  const mingw = path.join(ROOT, ".tools", "mingw64", "bin");
  const cargoBin = path.join(osDir, "cargo", "bin");
  const KEY = Object.keys(e).find((k) => k.toUpperCase() === "PATH") || "PATH";
  const front = [cargoBin, IS_WIN ? mingw : null].filter((p) => p && fs.existsSync(p));
  if (front.length) e[KEY] = front.join(path.delimiter) + path.delimiter + (e[KEY] || "");
  return e;
}

let stepNo = 0;
function announce(label) {
  stepNo++;
  process.stdout.write("[" + stepNo + "] " + label + " ... ");
}
function announceDone(status, detail) {
  console.log(status + (detail ? "（" + detail + "）" : ""));
}

function sh(cmd, args) {
  // 输出走文件而不是管道：受限环境里"用管道抓子进程输出"会 EPERM；落成日志还顺带留了档案。
  const logDir = path.join(ROOT, "target", "test-logs");
  fs.mkdirSync(logDir, { recursive: true });
  const slug = (s) => String(s).replace(/[^\w.-]+/g, "_");
  const logFile = path.join(logDir, slug(path.basename(cmd)) + "-" + args.map(slug).join("_") + ".log");
  const fd = fs.openSync(logFile, "w");
  const r = spawnSync(cmd, args, { cwd: ROOT, env: buildEnv(), stdio: ["ignore", fd, fd] });
  fs.closeSync(fd);
  const out = fs.readFileSync(logFile, "utf8");
  return { code: r.error ? -1 : r.status, out: out, error: r.error ? String(r.error.message) : null, log: path.relative(ROOT, logFile) };
}

/** 汇总 cargo 的 test result 行：一次运行可能有多条（多目标/多测试）。 */
function cargoCounts(out) {
  let passed = 0, failed = 0, resultLines = 0;
  for (const line of out.split(/\r?\n/)) {
    const m = line.match(/^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed/);
    if (m) { resultLines++; passed += Number(m[2]); failed += Number(m[3]); }
  }
  return { passed, failed, resultLines };
}

function skipsIn(out) {
  return out.split(/\r?\n/).filter((l) => l.includes("[探针]")).map((l) => l.trim());
}

/** 读该平台（含跨平台层）的缺口账：条目存在 = 这条测试还没有。 */
function gapLedgers() {
  const files = [path.join(ROOT, "tests", "cross-platform", "gaps.yaml"), path.join(ROOT, "tests", OS_KEY, "gaps.yaml")];
  const ids = [];
  for (const f of files) {
    if (!fs.existsSync(f)) continue;
    for (const line of fs.readFileSync(f, "utf8").split(/\r?\n/)) {
      const m = line.match(/^\s*-\s*id:\s*(\S+)/);
      if (m) ids.push(m[1]);
    }
  }
  return ids;
}

function main() {
  const steps = [];
  let doctor = null;
  announce("cargo build");
  console.log(FENCE_LIVE
    ? "[安全性] 已开启真机围栏测试：会在本机写权限（本工作区及其每一层祖先目录 + 命令用到的解释器安装目录）并创建容器 profile；一次性环境（CI / VM）里才建议开"
    : "[安全性] 安全模式：会改本机状态的测试已跳过（Windows 容器探针、端到端里的真实围栏写入）；要真跑加 --fence-live");
  const build = sh("cargo", ["build", "--color", "never"].concat(PROFILE === "release" ? ["--release"] : []));
  announceDone(build.code === 0 ? "完成" : "失败", build.code === 0 ? "" : build.log);
  steps.push({
    step: "cargo build",
    status: build.code === 0 ? "pass" : "fail",
    detail: build.code === 0 ? "" : (build.error || "") + " 日志：" + build.log,
    raw: build.code === 0 ? null : build.out.slice(-800),
  });
  if (build.code === 0 && fs.existsSync(BIN)) {
    const d = sh(BIN, ["--doctor"]);
    try { doctor = JSON.parse(d.out.trim()); } catch { doctor = { error: d.out.slice(0, 400) }; }
  }

  // L1：crate 内联单元测试
  announce("L1 单元（--bin solomni）");
  const unit = sh("cargo", ["test", "--color", "never", "--bin", "solomni", "--", "--test-threads=1", "--nocapture"]);
  const uc = cargoCounts(unit.out);
  announceDone(unit.code === 0 ? "完成" : "失败", uc.passed + " passed / " + uc.failed + " failed");
  steps.push({
    step: "L1 单元（--bin solomni）",
    status: unit.code === 0 ? "pass" : "fail",
    detail: uc.passed + " passed / " + uc.failed + " failed",
    skips: skipsIn(unit.out),
    raw: unit.code === 0 ? null : unit.out.slice(-800),
  });

  // L2/L3：四个按平台分的测试目标逐一点名（缺目标即失败：新增测试文件必须挂到目标上）
  for (const t of PLATFORM_TARGETS) {
    announce("目标 " + t);
    const r = sh("cargo", ["test", "--color", "never", "--test", t, "--", "--test-threads=1", "--nocapture"]);
    const c = cargoCounts(r.out);
    const skips = skipsIn(r.out);
    const isOtherPlatform = t !== "cross-platform" && t !== OS_KEY;
    let status;
    if (r.code !== 0) status = "fail";
    else if (c.resultLines === 0) status = "fail";
    else if (c.passed === 0 && isOtherPlatform) status = "skip-platform";
    else status = "pass";
    announceDone(status === "fail" ? "失败" : status === "skip-platform" ? "本平台不适用" : "完成", c.passed + " passed / " + c.failed + " failed");
    steps.push({
      step: "目标 " + t,
      status: status,
      detail:
        c.passed + " passed / " + c.failed + " failed" +
        (status === "skip-platform" ? "（本平台不适用）" : "") +
        (skips.length ? "；env-skip " + skips.length + " 条" : ""),
      skips: skips,
      raw: status === "fail" ? r.out.slice(-800) : null,
    });
  }

  // 前端冒烟（自动发现同目录 *.smoke.cjs）
  announce("前端冒烟");
  const fe = sh(process.execPath, [path.join("src", "presentation", "web", "smoke.cjs")]);
  announceDone(fe.code === 0 ? "完成" : "失败", "");
  steps.push({
    step: "前端冒烟",
    status: fe.code === 0 && fe.out.includes("FRONTEND-SMOKE-OK") ? "pass" : "fail",
    detail: fe.out.trim().split(/\r?\n/).slice(-2).join(" / "),
    raw: fe.code === 0 ? null : fe.out.slice(-600),
  });

  // L4：端到端（有编排才跑；没有就是一条缺口，不装作跑过）
  const e2e = path.join(ROOT, "tests", "cross-platform", "e2e", "orchestrator.js");
  if (fs.existsSync(e2e)) {
    announce("L4 端到端");
    const r = sh(process.execPath, [e2e]);
    announceDone(r.code === 0 ? "完成" : "失败", "");
    steps.push({
      step: "L4 端到端",
      status: r.code === 0 && r.out.includes("E2E-OK") ? "pass" : "fail",
      detail: r.out.trim().split(/\r?\n/).slice(-2).join(" / "),
      raw: r.code === 0 ? null : r.out.slice(-800),
    });
  } else {
    steps.push({ step: "L4 端到端", status: "gap", detail: "cross-platform.e2e.not-in-runner（编排尚未迁入）" });
  }

  const gaps = gapLedgers();
  const failed = steps.filter((s) => s.status === "fail");
  const skips = steps.flatMap((s) => s.skips || []);
  const report = {
    platform: process.platform,
    arch: process.arch,
    osKey: OS_KEY,
    profile: PROFILE,
    fenceLive: FENCE_LIVE,
    doctor: doctor,
    steps: steps.map((s) => ({ step: s.step, status: s.status, detail: s.detail })),
    envSkips: skips,
    gaps: gaps,
    failed: failed.length,
  };
  fs.mkdirSync(path.dirname(REPORT), { recursive: true });
  fs.writeFileSync(REPORT, JSON.stringify(report, null, 2));

  console.log("");
  console.log("=== 测试汇总（" + process.platform + " " + process.arch + "，报告见 target/test-report.json）===");
  for (const s of steps) console.log("  " + s.status.padEnd(13) + " " + s.step.padEnd(22) + " " + (s.detail || ""));
  if (doctor && doctor.fence) console.log("  [doctor] 围栏 fs=" + doctor.fence.fs + " net=" + doctor.fence.net + " tree=" + doctor.fence.tree + "（" + doctor.fence.note + "）");
  for (const s of skips) console.log("  [env-skip] " + s);
  for (const g of gaps) console.log("  [gap] " + g);
  if (failed.length) {
    for (const s of failed) {
      console.log("=== 失败详情：" + s.step + " ===");
      if (s.detail) console.log(s.detail);
      console.log(s.raw || "");
    }
    console.log("TEST-REPORT-FAIL");
    process.exit(1);
  }
  console.log("TEST-REPORT-OK");
  if (gaps.length === 0) console.log("TEST-REPORT-ACCEPTED（本平台缺口账为空）");
  else console.log("[验收] 本平台仍有 " + gaps.length + " 条缺口未关账（tests/" + OS_KEY + "/gaps.yaml 与 tests/cross-platform/gaps.yaml）");
}

main();
