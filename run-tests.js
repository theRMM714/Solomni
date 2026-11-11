#!/usr/bin/env node
/**
 * 测试总入口（见 TESTING.md）：**薄编排**——参数解析、车道 / 步骤选择、依赖调度、步骤渲染、报告写一次。
 * 质量门禁的判定在 tests/gates（registry 是唯一清单，纯文本、只读）；子进程 / 环境 / 步骤框架 / 报告在 tests/lib。
 * 选择性执行：--list 查清单、--only=<gate|lane|step> 只跑需要的那几项、--changed 按 git 影响面反查。
 * 任意子集**不写 target/test-report.json、不打 TEST-REPORT-***：报告只由车道级运行产出，验收只认车道。
 * 直接跑，或经 node start.js -test（后者会先把项目内工具链环境备好，再调本脚本）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const ROOT = __dirname;
const cli = require("./tests/lib/cli.js");
const opts = cli.parse(process.argv.slice(2));
if (opts.invalid) {
  console.error(opts.invalid);
  process.exit(2);
}
if (opts.help) {
  console.log(cli.usage());
  process.exit(0);
}

const proc = require("./tests/lib/proc.js").create({ root: ROOT, fenceLive: opts.fenceLive });
const step = require("./tests/lib/step.js").create();
const reportLib = require("./tests/lib/report.js");
const markers = require("./tests/lib/markers.js");
const outside = require("./tests/lib/outside.js");
const gates = require("./tests/gates/registry.js");
const stepsLib = require("./tests/gates/steps.js");
const { matchAny } = require("./tests/lib/fsutil.js");

const IS_WIN = process.platform === "win32";
const OS_KEY = IS_WIN ? "windows" : process.platform === "darwin" ? "macos" : "linux";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const PROFILE = opts.release ? "release" : "debug";
const BIN = path.join(ROOT, "target", PROFILE, EXE);
const PLATFORM_TARGETS = ["cross-platform", "windows", "linux", "macos"];
// 交叉类型检查的 target 与开发环境清单**同源**（dev-tools.js，唯一真相）：setup-dev.js 按它装，这里按它查。
// **只查不装**：装 target 是开发环境准备（node setup-dev.js），门禁不写系统工具链。
const CROSS_TARGETS = require("./dev-tools.js").targets.map((t) => t.triple);
// 只报不拦的步骤时间预算（秒）：超了在报告里标 [slow]，给找慢步骤留基线（对齐 run-hygiene 的"只报不拦"）。
const STEP_BUDGET_S = {
  "cargo build": 300,
  "T0 编译（--all-targets）": 300,
  "T0 交叉类型检查（x86_64-pc-windows-msvc）": 600,
  "T0 交叉类型检查（aarch64-apple-darwin）": 600,
  "T0 交叉类型检查（x86_64-apple-darwin）": 600,
  "T0 静态检查（clippy）": 300,
  "L1 单元（--bin solomni）": 300,
  "前端类型检查（tsc）": 120,
  "环境引导自检": 120,
  "目标 cross-platform": 300,
  "L4 端到端": 600,
};

/** 步骤对象共享的运行上下文：谁跑谁拿这一份，不依赖模块级可变全局。 */
function makeCtx() {
  return {
    root: ROOT,
    opts,
    proc,
    step,
    bin: BIN,
    exe: EXE,
    osKey: OS_KEY,
    platformTargets: PLATFORM_TARGETS,
    crossTargets: CROSS_TARGETS,
    state: { doctor: null, checkOut: null, build: null, outsideBefore: null, tempSwept: [] },
  };
}

// ---------- 清单（--list） ----------

/** 车道 / gate / 步骤的机器可读清单：--only 与 --changed 的取值都从这里读。 */
function listModel() {
  return {
    modes: [
      { id: "full", write: true, note: "整跑：质量门禁 + 单元 + 平台目标 + L4（写报告、打 TEST-REPORT-*）" },
      { id: "docs", write: true, note: "--docs：文档审查 + 结构审查（写报告）" },
      { id: "structure", write: true, note: "--audit=structure：只跑结构审查（写报告）" },
      { id: "selftest", write: false, note: "--only=selftest：只跑门禁自测（不写报告）" },
    ],
    lanes: [
      { id: "full", alias: "--only=full", write: true },
      { id: "docs", alias: "--only=docs（= --audit=docs）", write: true },
      { id: "structure", alias: "--only=structure（= --audit=structure）", write: true },
      { id: "selftest", alias: "--only=selftest", write: false },
    ],
    markers: markers.MARKERS,
    gates: gates.listModel(),
    steps: stepsLib.listModel(),
  };
}

function printList() {
  const model = listModel();
  if (opts.json) {
    console.log(JSON.stringify(model, null, 2));
    return;
  }
  console.log("模式（优先级：--coverage > --docs > --audit=… > 整跑）：");
  for (const m of model.modes) console.log("  " + m.id.padEnd(10) + (m.write ? "写报告  " : "不写报告") + "  " + m.note);
  // 列宽按内容算：id 长度不一，固定宽度会让最长的 id 顶掉后面的列。
  const gateIdW = Math.max(...model.gates.map((g) => g.id.length)) + 2;
  const stepIdW = Math.max(...model.steps.map((s) => s.id.length)) + 2;
  const stepNameW = Math.max(...model.steps.map((s) => s.name.length)) + 2;
  console.log("");
  console.log("gate（--only=<id>；纯文本 / 只读；不写报告）：");
  for (const g of model.gates) console.log("  " + g.id.padEnd(gateIdW) + g.lane.padEnd(11) + g.inputs.join(", "));
  console.log("");
  console.log("步骤（--only=<id>；不写报告；依赖自动补齐）：");
  for (const s of model.steps) console.log("  " + s.id.padEnd(stepIdW) + s.name.padEnd(stepNameW) + (s.deps.length ? "deps=" + s.deps.join(",") : "") + (s.requiresFull ? "（只能整跑）" : ""));
  console.log("");
  console.log("标记（打印点引用 tests/lib/markers.js，含义同源）：");
  for (const m of model.markers) console.log("  " + m.text.padEnd(22) + m.owner);
  console.log("");
  console.log("车道别名：--only=full / docs / structure / selftest（车道级写报告，selftest 不写）");
  console.log("影响面：--changed [--run]（依据工作树 vs HEAD，取不到差异按全量）");
}

// ---------- 审查车道（--docs / --audit=docs|structure / --only=<车道>） ----------

/**
 * 审查车道：只跑纯文本审查，不跑 cargo、单元、平台目标、前端冒烟与 L4；全程只读并写本次报告。
 * - all：文档审查 + 结构审查（本地 --docs，也是全量的 T0 结构审查步骤）；
 * - docs：只查文档自身（CI 的 docs job）；
 * - structure：只查代码↔文档结构（CI 的 structure job）。
 */
function runAudit(scope) {
  const label = scope === "docs" ? "T0 文档审查" : scope === "structure" ? "T0 结构审查" : "T0 文档门禁";
  step.announce(label);
  const audit = gates.runLane(ROOT, scope === "docs" ? "docs" : scope === "structure" ? "structure" : "all");
  const ms = step.elapsed();
  const detail = audit.problems.length
    ? audit.problems.join("；")
    : scope === "docs"
      ? "文档检查通过"
      : audit.targets.length + " 个目标 / " + audit.testFiles + " 个测试文件";
  step.announceDone(audit.problems.length ? "失败" : "完成", audit.problems.length ? audit.problems.length + " 条问题" : detail);
  step.push({ step: label, status: audit.problems.length ? "quality-fail" : "pass", detail, ms, slow: false });

  const qualityFailed = step.steps.filter((s) => s.status === "quality-fail");
  reportLib.write(
    ROOT,
    {
      mode: scope === "structure" ? "structure" : "docs",
      osKey: OS_KEY,
      profile: PROFILE,
      fenceLive: false,
      doctor: null,
      envSkips: [],
      quality: { failed: qualityFailed.length, steps: qualityFailed.map((s) => ({ step: s.step, detail: s.detail })) },
      globalGaps: [],
      gaps: [],
      timeouts: proc.timeouts,
      failed: 0,
      toolchain: reportLib.detectToolchain(proc),
    },
    step.steps,
  );

  const what = scope === "docs" ? "文档审查" : scope === "structure" ? "结构审查" : "文档审查 + 结构审查";
  console.log("");
  console.log("=== 测试汇总（" + what + "，报告见 target/test-report.json）===");
  for (const s of step.steps) {
    console.log("  " + s.status.padEnd(13) + " " + s.step.padEnd(24) + " " + (s.detail || "") + (s.ms ? "  [" + (s.ms / 1000).toFixed(1) + "s]" : ""));
  }
  if (qualityFailed.length) {
    for (const s of qualityFailed) {
      console.log("=== 质量详情：" + s.step + " ===");
      console.log(s.detail || "");
    }
    console.log(markers.TEXT.REPORT_FAIL);
    process.exit(1);
  }
  console.log(markers.TEXT.REPORT_OK);
  console.log("[audit] 本次只跑" + what + "：代码门禁、三平台与 L4 未运行；CI 报告统一写 ci-report。");
}

// ---------- 覆盖率门禁（手动） ----------

/**
 * 覆盖率门禁（node run-tests.js --coverage）：委托 tests/coverage.mjs --collect。
 * 四步：测试进程内 profile → 插桩构建产品 → L4 子进程 profile → 合并报告，按风险区判 80/60/100。
 * 只在 Linux 上采集：Windows 的 child.kill('SIGINT') 是 TerminateProcess，产品进程来不及在 atexit
 * 写出 profile，子进程覆盖率会凭空缺失；三平台的安全行为矩阵与平台探针仍由 quality job 每 PR 跑。
 * 缺工具直接失败（不再 env-skip），否则门禁会被"没装工具"冒充通过。
 */
function runCoverage() {
  if (process.platform !== "linux") {
    console.log("[覆盖率] 只在 Linux 上采集（子进程 profile 依赖 POSIX 信号正常退出）。");
    console.log("  Windows / macOS 用 CI 的 coverage-report 产物，或把报告路径交给 node tests/coverage.mjs --report <txt> --check。");
    process.exit(1);
  }
  if (!proc.toolAvailable("llvm-cov")) {
    console.log("[覆盖率] 缺少 cargo-llvm-cov（装：node setup-dev.js，或 cargo install --locked --root .tools cargo-llvm-cov）。");
    process.exit(1);
  }
  const r = proc.sh(process.execPath, [path.join(ROOT, "tests", "coverage.mjs"), "--collect"], 60 * 60 * 1000);
  console.log(r.out.trim().split(/\r?\n/).slice(-60).join("\n"));
  process.exit(r.code === 0 ? 0 : 1);
}

// ---------- 全量（整跑） ----------

function runFull() {
  // 项目外缓存快照与上次崩溃残留清理（清零前先记，跑完再对）。
  const outsideBefore = outside.snapshot(ROOT);
  const tempSwept = outside.sweepStaleTemp();
  const ctx = makeCtx();
  ctx.state.outsideBefore = outsideBefore;
  ctx.state.tempSwept = tempSwept;

  // CI 前置步骤（门禁之外执行）的墙钟由 workflow 经环境变量带进来，进报告，免得只能靠感觉。
  const preinstallMs = Number(process.env.SOLOMNI_CI_PREINSTALL_MS || 0);
  if (preinstallMs > 0) {
    step.push({ step: "CI 前置：供应链工具安装", status: "pass", ms: preinstallMs, detail: "在 run-tests 之前完成（CI 墙钟）", raw: null });
  }

  for (const s of stepsLib.STEPS) {
    if (s.skip && s.skip(opts)) {
      if (s.skipNote) console.log(s.skipNote);
      continue;
    }
    s.run(ctx);
  }

  // 只报不拦的时间预算：超预算标 [slow]（找慢步骤的基线，不改变成败）。
  for (const s of step.steps) {
    const budget = STEP_BUDGET_S[s.step];
    if (budget && s.ms > budget * 1000) s.slow = true;
  }
  const gaps = reportLib.readPlatformGaps(ROOT, OS_KEY);
  const globalGapsList = reportLib.readGlobalGaps(ROOT);
  const failed = step.steps.filter((s) => s.status === "fail");
  const qualityFailed = step.steps.filter((s) => s.status === "quality-fail");
  const envSkips = step.steps.filter((s) => s.status === "env-skip");
  const skips = step.steps.flatMap((s) => s.skips || []);
  const doctor = ctx.state.doctor;
  reportLib.write(
    ROOT,
    {
      mode: "full",
      osKey: OS_KEY,
      profile: PROFILE,
      fenceLive: opts.fenceLive,
      doctor: doctor,
      envSkips: skips,
      quality: { failed: qualityFailed.length, steps: qualityFailed.map((s) => ({ step: s.step, detail: s.detail })) },
      globalGaps: globalGapsList,
      gaps: gaps,
      timeouts: proc.timeouts,
      failed: failed.length,
      toolchain: reportLib.detectToolchain(proc),
    },
    step.steps,
  );

  console.log("");
  console.log("=== 测试汇总（" + process.platform + " " + process.arch + "，报告见 target/test-report.json）===");
  for (const s of step.steps) {
    console.log("  " + s.status.padEnd(13) + " " + s.step.padEnd(24) + " " + (s.detail || "") + (s.ms ? "  [" + (s.ms / 1000).toFixed(1) + "s]" : "") + (s.slow ? " [slow]" : ""));
  }
  if (opts.skipE2e) console.log("  [stage] 已跳过 L4 端到端：由独立 e2e job 承担，两份报告在发布时合并（见 docs/testing/execution-ci.md）");
  if (doctor && doctor.fence) console.log("  [doctor] 围栏 fs=" + doctor.fence.fs + " net=" + doctor.fence.net + " tree=" + doctor.fence.tree + "（" + doctor.fence.note + "）");
  for (const s of envSkips) console.log("  [env-skip] " + s.step + "：" + s.detail);
  for (const s of skips) console.log("  [env-skip] " + s);
  for (const t of proc.timeouts) console.log("  [超时] " + t);
  for (const s of qualityFailed) console.log("  [quality-fail] " + s.step + "：" + s.detail);
  // 文件粒度回执：超 800 行的实现文件逐个列出，只报不拦——行数是气味，依赖才是规则。
  const bigNotes = step.steps.flatMap((s) => s.notes || []);
  if (bigNotes.length) {
    console.log("  [big] 行数不是失败条件：新增独立职责优先拆子模块或抽端口，仅为数字拆文件会更乱。");
    for (const n of bigNotes) console.log("  " + n);
  }
  for (const g of gaps) console.log("  [gap] " + g);
  for (const g of globalGapsList) console.log("  [global-gap] " + g);
  if (failed.length || qualityFailed.length) {
    for (const s of failed) {
      console.log("=== 失败详情：" + s.step + " ===");
      if (s.detail) console.log(s.detail);
      console.log(s.raw || "");
    }
    for (const s of qualityFailed) {
      console.log("=== 质量详情：" + s.step + " ===");
      if (s.detail) console.log(s.detail);
      console.log(s.raw || "");
    }
    console.log(markers.TEXT.REPORT_FAIL);
    process.exit(1);
  }
  console.log(markers.TEXT.REPORT_OK);
  if (gaps.length === 0) console.log(markers.TEXT.REPORT_ACCEPTED + "（本平台缺口账为空）");
  else console.log("[验收] 本平台仍有 " + gaps.length + " 条缺口未关账（tests/" + OS_KEY + "/gaps.yaml 与 tests/cross-platform/gaps.yaml）");
  if (globalGapsList.length) {
    console.log("[验收] 仍有 " + globalGapsList.length + " 条全局长期目标（tests/gaps.yaml）：" + globalGapsList.join("、"));
  }
}

// ---------- 门禁自测（--only=selftest） ----------

function runSelftestLane() {
  console.log("[selftest] 门禁自测：解析器 / 项目外写 / 注释契约三项 / 门禁清单 / 步骤表");
  const groups = [
    ["结构审查内自测（解析器 + 项目外写 + 注释契约三项）", () => gates.runLane(ROOT, "selftest").problems],
    ["门禁清单", () => gates.selfTest(ROOT)],
    ["步骤表", () => stepsLib.selfTest()],
  ];
  let bad = 0;
  for (const [name, fn] of groups) {
    const problems = fn();
    if (problems.length) {
      bad += problems.length;
      console.log("[selftest] " + name + " FAIL（" + problems.length + " 条）");
      for (const p of problems) console.log("  " + p);
    } else {
      console.log("[selftest] " + name + " ok");
    }
  }
  process.exitCode = bad ? 1 : 0;
}

// ---------- 选择性子集（--only=<gate|step>） ----------

/** 跑任意子集：**不写报告、不打 TEST-REPORT-***（报告只由车道级运行产出）。 */
function runSelection(names) {
  if (names.length === 1 && cli.isLane(names[0])) {
    const lane = names[0];
    if (lane === "full") return runFull();
    if (lane === "docs" || lane === "structure") return runAudit(lane);
    return runSelftestLane();
  }
  const gateIds = [];
  const stepIds = [];
  for (const n of names) {
    if (gates.find(n)) gateIds.push(n);
    else if (stepsLib.STEPS.some((s) => s.id === n)) stepIds.push(n);
    else {
      console.error("未知 gate 或步骤：" + n + "（用 --list 看清单，--only=车道名 跑整条车道）");
      process.exit(2);
    }
  }
  console.log("[only] 子集运行：不写 target/test-report.json、不打印 TEST-REPORT-*；验收请跑车道（--docs / --audit=… / 整跑）");
  let bad = 0;
  // gate 按清单顺序跑（同一车道内顺序固定，输出可比）。
  const ordered = gates.GATES.filter((g) => gateIds.includes(g.id)).map((g) => g.id);
  for (const id of ordered) {
    const res = gates.runGates(ROOT, [id]);
    if (res.problems.length) {
      bad += res.problems.length;
      console.log("[gate] " + id + " FAIL（" + res.problems.length + " 条）");
      for (const p of res.problems) console.log("  " + p);
    } else {
      console.log("[gate] " + id + " ok");
    }
  }
  // 步骤按表顺序跑，依赖先补齐（比如 warnings 需要 check 的输出）。
  const wanted = new Set(stepIds);
  const orderedSteps = [];
  const addWithDeps = (id) => {
    const s = stepsLib.STEPS.find((x) => x.id === id);
    for (const d of s.deps) addWithDeps(d);
    if (!orderedSteps.includes(id)) orderedSteps.push(id);
  };
  for (const s of stepsLib.STEPS) if (wanted.has(s.id)) addWithDeps(s.id);
  const ctx = makeCtx();
  for (const id of orderedSteps) {
    const s = stepsLib.STEPS.find((x) => x.id === id);
    if (s.requiresFull) {
      console.log("[step] " + id + " 只能整跑（需要整跑的项目外写快照窗口）：node run-tests.js");
      bad++;
      continue;
    }
    const before = step.steps.length;
    s.run(ctx);
    for (const rec of step.steps.slice(before)) {
      if (rec.status === "fail" || rec.status === "quality-fail") {
        bad++;
        console.log("[step] " + rec.step + " " + rec.status + (rec.detail ? "：" + rec.detail : ""));
      } else {
        console.log("[step] " + rec.step + " " + rec.status);
      }
    }
  }
  process.exitCode = bad ? 1 : 0;
}

// ---------- 变更影响面（--changed） ----------

function isDocPath(file) {
  const p = String(file).replace(/\\/g, "/");
  return p.startsWith("docs/") || p.endsWith(".md") || p === "LICENSE";
}

/** 取本次差异：优先工作树 vs HEAD；没有未提交差异时退到 HEAD~1..HEAD；取不到就是 null（按全量）。 */
function gitChanged() {
  const run = (args) => {
    try {
      return execFileSync("git", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }).split(/\r?\n/).filter(Boolean);
    } catch (e) {
      return null;
    }
  };
  const wt = run(["diff", "--name-only", "HEAD", "--", "."]);
  if (wt && wt.length) return { files: wt, source: "工作树 vs HEAD" };
  const last = run(["diff", "--name-only", "HEAD~1", "HEAD"]);
  if (last && last.length) return { files: last, source: "HEAD~1..HEAD" };
  return { files: null, source: "取不到差异" };
}

// 门禁引擎自身的路径：改动它们 = 全部文本 gate 与整条车道都受影响（不猜具体哪一项）。
const ENGINE_INPUTS = ["run-tests.js", "tests/lib/**/*.js", "tests/gates/**/*.js"];

function runChanged() {
  const { files, source } = gitChanged();
  if (!files) {
    console.log("[changed] " + source + "：按全量处理（宁可全量，不可漏测）");
    if (opts.run) runSelection(["docs", "structure"]);
    return;
  }
  const matchedGates = gates.GATES.filter((g) => files.some((f) => matchAny(g.inputs, f))).map((g) => g.id);
  const matchedSteps = stepsLib.STEPS.filter((s) => files.some((f) => matchAny(s.inputs, f))).map((s) => s.id);
  const unknown = files.filter((f) => !gates.GATES.some((g) => matchAny(g.inputs, f)) && !stepsLib.STEPS.some((s) => matchAny(s.inputs, f)) && !isDocPath(f));
  const engineChanged = files.some((f) => matchAny(ENGINE_INPUTS, f));
  const fallback = unknown.length > 0 || engineChanged;
  const ids = fallback ? gates.GATES.map((g) => g.id) : matchedGates;
  const lanes = new Set();
  if (files.some(isDocPath)) lanes.add("docs");
  lanes.add("structure");
  if (files.some((f) => !isDocPath(f)) || fallback) lanes.add("full");
  console.log("[changed] 依据：" + source + "（" + files.length + " 个文件）");
  if (engineChanged) console.log("[changed] 门禁引擎自身有改动（run-tests.js / tests/lib / tests/gates）：全部文本 gate 与车道都受影响");
  if (unknown.length) console.log("[changed] 有未归类路径（" + unknown.length + " 个），按全量处理：" + unknown.slice(0, 5).join("、") + (unknown.length > 5 ? " …" : ""));
  console.log("[changed] 受影响 gate（" + ids.length + "）：");
  for (const id of ids) console.log("  " + id);
  if (matchedSteps.length) console.log("[changed] 受影响步骤（" + matchedSteps.length + "）：" + matchedSteps.join("、"));
  console.log("[changed] 建议车道：" + [...lanes].join(" / "));
  console.log("[changed] 子集运行不写报告、不能当验收；full 车道请整跑：node run-tests.js");
  if (!opts.run) return;
  console.log("");
  console.log("[changed] --run：只跑受影响 gate（不写报告）");
  runSelection(ids);
}

// ---------- main ----------

function main() {
  if (opts.list) {
    printList();
    return;
  }
  if (opts.only.length) {
    runSelection(opts.only);
    return;
  }
  if (opts.changed) {
    runChanged();
    return;
  }
  const mode = cli.mode(opts);
  if (mode === "coverage") return runCoverage();
  if (mode === "docs-all") return runAudit("all");
  if (mode === "audit-docs") return runAudit("docs");
  if (mode === "audit-structure") return runAudit("structure");
  return runFull();
}

main();
