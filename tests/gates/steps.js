/**
 * 全量入口的步骤表：**顺序、依赖、跳过条件、步骤名**都只在这里定义。
 * 目的：main 只做调度与汇总；步骤名是报告与文档的锚点，改动必须同步 docs/testing/execution-ci.md。
 * 子进程一律走 ctx.proc（环境、日志、超时一处口径）；步骤记录一律走 ctx.step（报告字段一处口径）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const parsers = require("../lib/parsers.js");
const gates = require("./registry.js");
const markers = require("../lib/markers.js");

const E2E_TIMEOUT_MS = 20 * 60 * 1000;
const ENV_BOOTSTRAP_TIMEOUT_MS = 5 * 60 * 1000;

/** 已安装的交叉 target（rustup 不在或查不动 = null；门禁只查不装）。 */
function installedTargets(ctx) {
  const r = ctx.proc.sh("rustup", ["target", "list", "--installed"]);
  if (r.code !== 0) return null;
  return r.out.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
}

/** 交叉检查失败是不是"本机没有该平台的 C 工具链 / SDK"这类环境结论（只对 macOS 判定，别处一律当失败）。 */
function crossEnvLimited(target, out) {
  if (!target.includes("apple-darwin")) return false;
  return /unrecognized command-line option '-arch'/.test(out) || /-mmacosx-version-min/.test(out) || /error occurred in cc-rs/.test(out);
}

/** 依赖重复治理记录：tests/dependency-policy.json；读不动就是治理链断了，门禁要红而不是静默放行。 */
function duplicatePolicy(root) {
  const file = path.join(root, "tests", "dependency-policy.json");
  let raw;
  try {
    raw = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (e) {
    return { error: "读不到依赖重复治理记录 tests/dependency-policy.json：" + e.message, allow: [] };
  }
  return { error: null, allow: Array.isArray(raw.allow) ? raw.allow : [] };
}

/** 供应链门禁：已知 CVE（cargo audit）与许可证/禁用/来源策略（cargo deny）；工具缺失 = env-skip。 */
function supplyChainCheck(ctx) {
  const wanted = [
    { tool: "audit", args: ["audit", "--color", "never"], why: "已知 CVE" },
    { tool: "deny", args: ["deny", "--color", "never", "check"], why: "许可证 / 禁用 / 来源" },
  ];
  const has = wanted.filter((w) => ctx.proc.toolAvailable(w.tool));
  if (!has.length) {
    return { status: "env-skip", detail: "cargo-audit / cargo-deny 未安装（装：node setup-dev.js，落在项目内 .tools/）", raw: null };
  }
  const missing = wanted.filter((w) => !has.includes(w)).map((w) => w.tool);
  const ran = [];
  const failed = [];
  const skipped = [];
  for (const w of has) {
    const r = ctx.proc.sh("cargo", w.args);
    ran.push(w.tool);
    if (r.code === 0) continue;
    // 取不到 advisory 数据 / 连不上网 = 环境不允许（env-skip）；有确凿结论（CVE、许可证不合规）= 硬失败。
    const findings = /(\d+ vulnerabilit|vulnerabilit(y|ies) found|advisories FAILED|licenses FAILED|bans FAILED|sources FAILED|error\[)/i;
    const dataIssue = /(could not|couldn't|failed to) (fetch|connect|download)|connection (refused|reset)|dns|network|advisory.db|proxy|timed? ?out/i;
    if (dataIssue.test(r.out) && !findings.test(r.out)) skipped.push(w.tool + "（取不到数据）");
    else failed.push(w.tool + "：" + ((r.error || "") + "（日志 " + r.log + "）"));
  }
  if (failed.length) return { status: "fail", detail: failed.join("；"), raw: null };
  const parts = [];
  if (ran.length) parts.push("通过：" + ran.join("、"));
  if (missing.length) parts.push("未安装（env-skip）：" + missing.join("、"));
  if (skipped.length) parts.push("取不到数据（env-skip）：" + skipped.join("、"));
  return { status: parts.some((p) => p.startsWith("通过")) ? "pass" : "env-skip", detail: parts.join("；"), raw: null };
}

/** 记一步的通用收尾：状态、说明、原始输出。 */
function finish(ctx, obj) {
  ctx.step.push(obj);
}

/** --strict-env：必装工具缺失时把 env-skip 升为失败；可选项（交叉 target）不经过这里。 */
function envSkipResult(ctx, detail, failStatus) {
  return ctx.opts.strictEnv
    ? { status: failStatus || "quality-fail", detail: "strict-env：必装工具缺失（CI 必须装齐）→ " + detail }
    : { status: "env-skip", detail: detail };
}

/** 安全探针的环境跳过：围栏 / ACL / 容器 / 权限 / 平台强制机制。CI 下这些不许当成通过。 */
function securitySkipsIn(lines) {
  return (lines || []).filter((l) => /围栏|ACL|容器|权限|seccomp|Landlock|seatbelt/i.test(l));
}

const STEPS = [
  {
    id: "build",
    name: "cargo build",
    deps: [],
    inputs: ["Cargo.toml", "Cargo.lock", "src/**/*.rs", "src/**/*.toml"],
    run(ctx) {
      ctx.step.announce("cargo build");
      console.log(ctx.opts.fenceLive
        ? "[安全性] 已开启真机围栏测试：会在本机写权限（本工作区及其每一层祖先目录 + 命令用到的解释器安装目录）并创建容器 profile；一次性环境（CI / VM）里才建议开"
        : "[安全性] 安全模式：会改本机状态的测试已跳过（Windows 容器探针、端到端里的真实围栏写入）；要真跑加 --fence-live");
      const build = ctx.proc.sh("cargo", ["build", "--color", "never"].concat(ctx.opts.release ? ["--release"] : []));
      ctx.step.announceDone(build.code === 0 ? "完成" : "失败", build.code === 0 ? "" : build.log);
      finish(ctx, {
        step: "cargo build",
        status: build.code === 0 ? "pass" : "fail",
        detail: build.code === 0 ? "" : (build.error || "") + " 日志：" + build.log,
        raw: build.code === 0 ? null : build.out.slice(-800),
      });
      ctx.state.build = build;
      if (build.code === 0 && fs.existsSync(ctx.bin)) {
        const d = ctx.proc.sh(ctx.bin, ["--doctor"]);
        try { ctx.state.doctor = JSON.parse(d.out.trim()); } catch { ctx.state.doctor = { error: d.out.slice(0, 400) }; }
      }
    },
  },
  {
    id: "check",
    name: "T0 编译（--all-targets）",
    deps: [],
    inputs: ["src/**/*.rs", "tests/**/*.rs", "Cargo.toml", "Cargo.lock"],
    run(ctx) {
      ctx.step.announce("T0 编译（--all-targets）");
      const check = ctx.proc.sh("cargo", ["check", "--all-targets", "--color", "never"]);
      ctx.step.announceDone(check.code === 0 ? "完成" : "失败", check.code === 0 ? "" : check.log);
      finish(ctx, {
        step: "T0 编译（--all-targets）",
        status: check.code === 0 ? "pass" : "fail",
        detail: check.code === 0 ? "" : (check.error || "") + " 日志：" + check.log,
        raw: check.code === 0 ? null : check.out.slice(-800),
      });
      ctx.state.checkOut = check.out;
    },
  },
  {
    id: "audit",
    name: "T0 结构审查",
    deps: [],
    inputs: [".github/**", "docs/**/*.md", "src/**/*.rs", "tests/**/*.rs", "*.md", "*.yaml", "run-tests.js", "tests/lib/**/*.js", "tests/gates/**/*.js"],
    skip: (opts) => opts.skipAudit,
    skipNote: "[T0] 已跳过结构审查（--skip-audit）：由 CI 的 structure job 承担",
    run(ctx) {
      ctx.step.announce("T0 结构审查");
      const audit = gates.runLane(ctx.root, "all");
      ctx.step.announceDone(
        audit.problems.length ? "失败" : "完成",
        audit.problems.length
          ? audit.problems.length + " 条问题"
          : audit.targets.length + " 个目标 / " + audit.testFiles + " 个测试文件" + (audit.notes.length ? " / " + audit.notes.length + " 个文件超行数软线" : ""),
      );
      finish(ctx, {
        step: "T0 结构审查",
        status: audit.problems.length ? "quality-fail" : "pass",
        detail: audit.problems.join("；"),
        notes: audit.notes,
        raw: null,
      });
    },
  },
  {
    id: "fmt",
    name: "T0 格式（fmt --check）",
    deps: [],
    inputs: ["src/**/*.rs", "tests/**/*.rs", "src/**/*.toml"],
    run(ctx) {
      ctx.step.announce("T0 格式（fmt --check）");
      if (!ctx.proc.toolAvailable("fmt")) {
        const s = envSkipResult(ctx, "cargo-fmt 未安装：node setup-dev.js", "quality-fail");
        ctx.step.announceDone(s.status === "env-skip" ? "env-skip" : "失败", s.detail);
        finish(ctx, { step: "T0 格式（fmt --check）", status: s.status, detail: s.detail });
        return;
      }
      const r = ctx.proc.sh("cargo", ["fmt", "--all", "--", "--check", "--color", "never"]);
      const files = [...parsers.fmtDeviations(r.out, ctx.root)];
      const ok = r.code === 0 && !files.length;
      ctx.step.announceDone(ok ? "完成" : "失败", ok ? "" : files.length + " 个文件有格式偏差");
      finish(ctx, {
        step: "T0 格式（fmt --check）",
        status: ok ? "pass" : "quality-fail",
        detail: ok ? "" : "有格式偏差（跑 cargo fmt --all 后再提交）：" + files.join("、"),
        raw: ok ? null : r.out.slice(-1200),
      });
    },
  },
  {
    id: "clippy",
    name: "T0 静态检查（clippy）",
    deps: [],
    inputs: ["src/**/*.rs", "tests/**/*.rs", "Cargo.toml"],
    run(ctx) {
      ctx.step.announce("T0 静态检查（clippy）");
      if (!ctx.proc.toolAvailable("clippy")) {
        const s = envSkipResult(ctx, "cargo-clippy 未安装：node setup-dev.js", "quality-fail");
        ctx.step.announceDone(s.status === "env-skip" ? "env-skip" : "失败", s.detail);
        finish(ctx, { step: "T0 静态检查（clippy）", status: s.status, detail: s.detail });
        return;
      }
      // --keep-going：-D warnings 会让首个失败的单元中断调度；加上它所有目标单元都编译完，计数才可复现。
      const r = ctx.proc.sh("cargo", ["clippy", "--all-targets", "--all-features", "--keep-going", "--color", "never", "--", "-D", "warnings"]);
      const got = parsers.clippyLints(r.out);
      const total = Object.values(got).reduce((a, b) => a + b, 0);
      const ok = r.code === 0 && total === 0;
      ctx.step.announceDone(ok ? "完成" : "失败", ok ? "零告警" : total + " 处（" + Object.keys(got).join("、") + "）");
      finish(ctx, {
        step: "T0 静态检查（clippy）",
        status: ok ? "pass" : "quality-fail",
        detail: ok ? "" : "clippy 有告警：" + Object.entries(got).map(([k, v]) => k + "=" + v).join("、") + "（设计取舍项要带理由 allow，见 docs/testing/quality-isolation.md）",
        raw: ok ? null : r.out.slice(-1200),
      });
    },
  },
  {
    id: "warnings",
    name: "T0 编译告警",
    deps: ["check"],
    inputs: ["src/**/*.rs", "tests/**/*.rs"],
    run(ctx) {
      ctx.step.announce("T0 编译告警");
      const got = parsers.checkWarnings(ctx.state.checkOut || "");
      const ok = got === 0;
      ctx.step.announceDone(ok ? "完成" : "失败", ok ? "零告警" : got + " 条");
      finish(ctx, {
        step: "T0 编译告警",
        status: ok ? "pass" : "quality-fail",
        detail: ok ? "" : "rustc 告警 " + got + " 条（cargo check 的原文见日志）",
        raw: ok ? null : String(ctx.state.checkOut || "").slice(-1200),
      });
    },
  },
  {
    id: "cross-check",
    name: "T0 交叉类型检查",
    deps: [],
    inputs: ["src/**/*.rs", "tests/**/*.rs", "Cargo.toml", "dev-tools.js"],
    run(ctx) {
      // 开发机专属：CI 由三平台矩阵各自原生编译 --all-targets 覆盖，这里不跑。
      if (process.env.CI) {
        ctx.step.announce("T0 交叉类型检查");
        ctx.step.announceDone("skip-platform", "CI 各平台原生编译已覆盖");
        finish(ctx, {
          step: "T0 交叉类型检查",
          status: "skip-platform",
          detail: "CI：三平台矩阵各自原生 cargo check --all-targets 已覆盖本平台；交叉检查是开发机补齐它平台的手段，这里不跑",
        });
        return;
      }
      const installed = installedTargets(ctx);
      for (const target of ctx.crossTargets) {
        const label = "T0 交叉类型检查（" + target + "）";
        ctx.step.announce(label);
        if (!installed || !installed.includes(target)) {
          ctx.step.announceDone("env-skip", "未安装该 target");
          finish(ctx, { step: label, status: "env-skip", detail: "未安装 " + target + "：rustup target add " + target });
          continue;
        }
        // --all-targets：平台专属的测试代码本机也不编译，必须一起查（CI 就是在 Windows 上这样红的）。
        const r = ctx.proc.sh("cargo", ["check", "--target", target, "--all-targets", "--color", "never"]);
        if (r.code === 0) {
          ctx.step.announceDone("完成", "");
          finish(ctx, { step: label, status: "pass", detail: "" });
        } else if (crossEnvLimited(target, r.out)) {
          ctx.step.announceDone("env-skip", "本机没有该平台的 C 工具链 / SDK");
          finish(ctx, { step: label, status: "env-skip", detail: "本机无法交叉编译 " + target + "（缺 Apple SDK / 交叉 C 工具链）：真实检查归该平台的真机 CI" });
        } else {
          ctx.step.announceDone("失败", "编译失败");
          finish(ctx, { step: label, status: "fail", detail: "交叉类型检查失败（平台专属代码本机不编译）：" + (r.error || "") + " 日志：" + r.log, raw: r.out.slice(-1200) });
        }
      }
    },
  },
  {
    id: "duplicates",
    name: "T0 依赖重复（cargo tree）",
    deps: [],
    inputs: ["Cargo.toml", "Cargo.lock", "tests/dependency-policy.json"],
    run(ctx) {
      ctx.step.announce("T0 依赖重复（cargo tree）");
      if (!ctx.proc.toolAvailable("tree")) {
        const s = envSkipResult(ctx, "cargo tree 不可用", "quality-fail");
        ctx.step.announceDone(s.status === "env-skip" ? "env-skip" : "失败", s.detail);
        finish(ctx, { step: "T0 依赖重复（cargo tree）", status: s.status, detail: s.detail });
        return;
      }
      const r = ctx.proc.sh("cargo", ["tree", "--duplicates", "--color", "never"]);
      const policy = duplicatePolicy(ctx.root);
      const cls = parsers.explainedDuplicates(parsers.duplicateCrates(r.out), policy.allow);
      const ok = r.code === 0 && !policy.error && !cls.unexplained.length;
      let detail = "";
      if (policy.error) detail = policy.error;
      else if (!ok) detail = "重复依赖未解释（写进 tests/dependency-policy.json 的 allow 才能放行）：" + cls.unexplained.join("、");
      else if (cls.hit.length) detail = "允许的重复（理由见 tests/dependency-policy.json）：" + cls.hit.join("、");
      if (ok && cls.stale.length) detail += (detail ? "；" : "") + "本平台未出现（跨平台条目）：" + cls.stale.join("、");
      ctx.step.announceDone(ok ? "完成" : "失败", ok ? (cls.hit.length ? "允许 " + cls.hit.length + " 个重复" : "无重复") : policy.error ? "治理记录读不动" : cls.unexplained.length + " 个重复未解释");
      finish(ctx, {
        step: "T0 依赖重复（cargo tree）",
        status: ok ? "pass" : "quality-fail",
        detail,
        raw: ok ? null : r.out.slice(-1200),
      });
    },
  },
  {
    id: "supply",
    name: "T0 供应链（audit/deny）",
    deps: [],
    inputs: ["Cargo.lock", "deny.toml"],
    run(ctx) {
      ctx.step.announce("T0 供应链（audit/deny）");
      let supply = supplyChainCheck(ctx);
      if (ctx.opts.strictEnv && supply.status === "env-skip") {
        supply = { status: "fail", detail: "strict-env：供应链工具缺失或取不到 advisory 数据 → " + supply.detail, raw: supply.raw };
      }
      ctx.step.announceDone(supply.status === "pass" ? "完成" : supply.status === "env-skip" ? "env-skip" : "失败", supply.detail);
      finish(ctx, { step: "T0 供应链（audit/deny）", status: supply.status, detail: supply.detail, raw: supply.raw });
    },
  },
  {
    id: "unit",
    name: "L1 单元（--bin solomni）",
    deps: [],
    inputs: ["src/**/*.rs", "tests/**/*.rs", "Cargo.toml", "Cargo.lock"],
    run(ctx) {
      ctx.step.announce("L1 单元（--bin solomni）");
      const unitArgs = ["test", "--color", "never", "--bin", "solomni", "--", "--nocapture"].concat(ctx.opts.serial ? ["--test-threads=1"] : []);
      const unit = ctx.proc.sh("cargo", unitArgs);
      const uc = parsers.cargoCounts(unit.out);
      const skips = parsers.skipsIn(unit.out);
      const securitySkips = ctx.opts.fenceLive ? securitySkipsIn(skips) : [];
      let status = "pass";
      if (unit.code !== 0 || uc.ignored > 0) status = "fail";
      else if (securitySkips.length) status = "quality-fail";
      else if (ctx.opts.strictEnv && !ctx.opts.fenceLive) status = "quality-fail";
      const unitDetail =
        uc.passed + " passed / " + uc.failed + " failed" +
        (uc.ignored ? " / " + uc.ignored + " ignored（#[ignore] 被禁止）" : "") +
        (securitySkips.length ? " / 安全探针环境跳过 " + securitySkips.length + " 条（CI 不容忍）" : "") +
        (ctx.opts.strictEnv && !ctx.opts.fenceLive ? " / strict-env 必须同时 --fence-live" : "") +
        (ctx.opts.serial ? "；--serial" : "");
      ctx.step.announceDone(status === "pass" ? "完成" : "失败", unitDetail);
      finish(ctx, { step: "L1 单元（--bin solomni）", status, detail: unitDetail, skips, raw: status === "pass" ? null : unit.out.slice(-1200) });
    },
  },
  ...["cross-platform", "windows", "linux", "macos"].map((t) => ({
    id: "target/" + t,
    name: "目标 " + t,
    deps: [],
    inputs: ["tests/" + t + "/**/*.rs", "tests/helpers/**/*.rs", "Cargo.toml", "src/**/*.rs"],
    run(ctx) {
      ctx.step.announce("目标 " + t);
      const tArgs = ["test", "--color", "never", "--test", t, "--", "--nocapture"].concat(ctx.opts.serial ? ["--test-threads=1"] : []);
      const r = ctx.proc.sh("cargo", tArgs);
      const c = parsers.cargoCounts(r.out);
      const skips = parsers.skipsIn(r.out);
      const securitySkips = ctx.opts.fenceLive ? securitySkipsIn(skips) : [];
      const isOtherPlatform = t !== "cross-platform" && t !== ctx.osKey;
      let status;
      if (r.code !== 0) status = "fail";
      else if (c.ignored > 0) status = "fail";
      else if (c.resultLines === 0) status = "fail";
      else if (c.passed === 0 && isOtherPlatform) status = "skip-platform";
      else if (securitySkips.length) status = "quality-fail";
      else status = "pass";
      const counts = c.passed + " passed / " + c.failed + " failed" + (c.ignored ? " / " + c.ignored + " ignored（#[ignore] 被禁止）" : "");
      const detail = counts + (status === "skip-platform" ? "（本平台不适用）" : "") +
        (securitySkips.length ? "；安全探针环境跳过 " + securitySkips.length + " 条（CI 不容忍）" : "") +
        (skips.length ? "；env-skip " + skips.length + " 条" : "");
      ctx.step.announceDone(status === "fail" || status === "quality-fail" ? "失败" : status === "skip-platform" ? "本平台不适用" : "完成", detail);
      finish(ctx, {
        step: "目标 " + t,
        status,
        detail,
        skips,
        raw: status === "fail" || status === "quality-fail" ? r.out.slice(-1200) : null,
      });
    },
  })),
  {
    id: "frontend-smoke",
    name: "前端冒烟",
    deps: [],
    inputs: ["src/presentation/web/assets/**"],
    run(ctx) {
      ctx.step.announce("前端冒烟");
      const fe = ctx.proc.sh(process.execPath, [path.join("src", "presentation", "web", "assets", "smoke.cjs")]);
      ctx.step.announceDone(fe.code === 0 ? "完成" : "失败", "");
      finish(ctx, {
        step: "前端冒烟",
        status: fe.code === 0 && fe.out.includes(markers.TEXT.FRONTEND_SMOKE_OK) ? "pass" : "fail",
        detail: fe.out.trim().split(/\r?\n/).slice(-2).join(" / "),
        raw: fe.code === 0 ? null : fe.out.slice(-600),
      });
    },
  },
  {
    id: "frontend-typecheck",
    name: "前端类型检查（tsc）",
    deps: [],
    inputs: ["tsconfig.json", "package.json", "package-lock.json", "src/presentation/web/assets/**/*.js"],
    run(ctx) {
      ctx.step.announce("前端类型检查（tsc）");
      const tsc = path.join(ctx.root, "node_modules", "typescript", "bin", "tsc");
      if (!fs.existsSync(tsc)) {
        const s = envSkipResult(ctx, "node_modules 未安装：npm ci --ignore-scripts", "quality-fail");
        ctx.step.announceDone(s.status === "env-skip" ? "env-skip" : "失败", s.detail);
        finish(ctx, { step: "前端类型检查（tsc）", status: s.status, detail: s.detail });
        return;
      }
      const r = ctx.proc.sh(process.execPath, [tsc, "-p", "tsconfig.json"]);
      const ok = r.code === 0;
      ctx.step.announceDone(ok ? "完成" : "失败", ok ? "零错误" : "存在类型错误");
      finish(ctx, {
        step: "前端类型检查（tsc）",
        status: ok ? "pass" : "quality-fail",
        detail: ok ? "" : r.out.trim().split(/\r?\n/).slice(0, 20).join(" / "),
        raw: ok ? null : r.out.slice(-2000),
      });
    },
  },
  {
    id: "selfcheck/dev-loop",
    name: "开发回路自检",
    deps: [],
    inputs: ["dev.js", "tests/dev-loop.mjs"],
    run(ctx) {
      ctx.step.announce("开发回路自检");
      const dl = ctx.proc.sh(process.execPath, [path.join("tests", "dev-loop.mjs")]);
      ctx.step.announceDone(dl.code === 0 ? "完成" : "失败", "");
      finish(ctx, {
        step: "开发回路自检",
        status: dl.code === 0 && dl.out.includes(markers.TEXT.DEV_LOOP_OK) ? "pass" : "fail",
        detail: dl.out.trim().split(/\r?\n/).slice(-1).join(""),
        raw: dl.code === 0 ? null : dl.out.slice(-600),
      });
    },
  },
  {
    id: "selfcheck/pins-tool",
    name: "依赖哈希工具自检",
    deps: [],
    inputs: ["pins.js", "tests/pins-tool.mjs", "dependencies.yaml"],
    run(ctx) {
      ctx.step.announce("依赖哈希工具自检");
      const pt = ctx.proc.sh(process.execPath, [path.join("tests", "pins-tool.mjs")]);
      ctx.step.announceDone(pt.code === 0 ? "完成" : "失败", "");
      finish(ctx, {
        step: "依赖哈希工具自检",
        status: pt.code === 0 && pt.out.includes(markers.TEXT.PINS_TOOL_OK) ? "pass" : "fail",
        detail: pt.out.trim().split(/\r?\n/).slice(-1).join(""),
        raw: pt.code === 0 ? null : pt.out.slice(-600),
      });
    },
  },
  {
    id: "selfcheck/ci-changes",
    name: "变更分流自检",
    deps: [],
    inputs: ["tests/ci-changes.mjs"],
    run(ctx) {
      ctx.step.announce("变更分流自检");
      const cc = ctx.proc.sh(process.execPath, [path.join("tests", "ci-changes.mjs"), "--self-test"]);
      ctx.step.announceDone(cc.code === 0 ? "完成" : "失败", "");
      finish(ctx, {
        step: "变更分流自检",
        status: cc.code === 0 ? "pass" : "fail",
        detail: cc.out.trim().split(/\r?\n/).slice(-1).join(""),
        raw: cc.code === 0 ? null : cc.out.slice(-600),
      });
    },
  },
  {
    id: "selfcheck/env-bootstrap",
    name: "环境引导自检",
    deps: [],
    inputs: ["env.js", "tests/env-bootstrap.mjs"],
    run(ctx) {
      ctx.step.announce("环境引导自检");
      const eb = ctx.proc.sh(process.execPath, [path.join("tests", "env-bootstrap.mjs")], ENV_BOOTSTRAP_TIMEOUT_MS);
      const ebSkips = parsers.skipsIn(eb.out);
      ctx.step.announceDone(eb.code !== 0 ? "失败" : ebSkips.length ? "env-skip" : "完成", "");
      finish(ctx, {
        step: "环境引导自检",
        // 非 Windows 上那条路（dlltool）不存在：自检自己打 [探针] 并照常结束——那是 env-skip，不是通过。
        status: eb.code !== 0 ? "fail" : ebSkips.length ? "env-skip" : "pass",
        detail: ebSkips.length ? ebSkips.join(" / ") : eb.out.trim().split(/\r?\n/).slice(-1).join(""),
        skips: ebSkips,
        raw: eb.code === 0 ? null : eb.out.slice(-1200),
      });
    },
  },
  {
    id: "e2e",
    name: "L4 端到端",
    deps: [],
    inputs: ["tests/cross-platform/e2e/**", "src/**/*.rs", "Cargo.toml"],
    skip: (opts) => opts.skipE2e,
    run(ctx) {
      // 有编排才跑；没有就是一条缺口，不装作跑过。
      const e2e = path.join(ctx.root, "tests", "cross-platform", "e2e", "orchestrator.js");
      if (fs.existsSync(e2e)) {
        ctx.step.announce("L4 端到端");
        const r = ctx.proc.sh(process.execPath, [e2e], E2E_TIMEOUT_MS);
        ctx.step.announceDone(r.code === 0 ? "完成" : "失败", "");
        finish(ctx, {
          step: "L4 端到端",
          status: r.code === 0 && r.out.includes(markers.TEXT.E2E_OK) ? "pass" : "fail",
          detail: r.out.trim().split(/\r?\n/).slice(-2).join(" / "),
          raw: r.code === 0 ? null : r.out.slice(-800),
        });
      } else {
        finish(ctx, { step: "L4 端到端", status: "gap", detail: "cross-platform.e2e.not-in-runner（编排尚未迁入）" });
      }
    },
  },
  {
    id: "outside-writes",
    name: "T0 项目外写",
    deps: [],
    requiresFull: true,
    inputs: ["src/**/*.rs", "tests/**/*.rs", "env.js"],
    run(ctx) {
      // 快照在整跑开始时取（run-tests 调度层做）；只跑子集时这个窗口不成立，所以此步只能整跑。
      ctx.step.announce("T0 项目外写");
      const outsideChanged = require("../lib/outside.js").diff(ctx.state.outsideBefore || {}, require("../lib/outside.js").snapshot(ctx.root));
      const tempSwept = ctx.state.tempSwept || [];
      ctx.step.announceDone(
        outsideChanged.length ? "失败" : "完成",
        outsideChanged.length ? outsideChanged.length + " 处项目外写入" : (tempSwept.length ? "清理了上次残留 " + tempSwept.length + " 项" : ""),
      );
      finish(ctx, {
        step: "T0 项目外写",
        status: outsideChanged.length ? "fail" : "pass",
        detail: outsideChanged.length
          ? "检测到项目外缓存/工具链目录被写：" + outsideChanged.join("、") + "（借用系统工具链也不许写它的 home；确认不是其它进程同写后重跑）"
          : (tempSwept.length ? "系统临时目录清掉上次崩溃残留：" + tempSwept.join("、") : ""),
        raw: null,
      });
    },
  },
];

/** 步骤可产生的状态声明（不是运行结果）：文档据此生成"这一步会不会 env-skip / 哪一步是零容忍"。 */
const OUTCOMES = {
  build: { status: ["pass", "fail"], envSkip: null },
  check: { status: ["pass", "fail"], envSkip: null },
  audit: { status: ["pass", "quality-fail"], envSkip: null },
  fmt: { status: ["pass", "quality-fail", "env-skip"], envSkip: "cargo-fmt 未安装（--strict-env 下算失败）" },
  clippy: { status: ["pass", "quality-fail", "env-skip"], envSkip: "cargo-clippy 未安装（--strict-env 下算失败）" },
  warnings: { status: ["pass", "quality-fail"], envSkip: null },
  "cross-check": { status: ["pass", "fail", "env-skip", "skip-platform"], envSkip: "可选：该 target 未安装，或本机缺该平台的 C 工具链 / SDK；CI 原生矩阵才是权威，--strict-env 不强制" },
  duplicates: { status: ["pass", "quality-fail", "env-skip"], envSkip: "cargo-tree 不可用（--strict-env 下算失败）" },
  supply: { status: ["pass", "fail", "env-skip"], envSkip: "cargo-audit / cargo-deny 未安装或取不到 advisory 数据（--strict-env 下算失败）" },
  unit: { status: ["pass", "fail", "quality-fail"], envSkip: "安全探针在 --fence-live 下跳过；或 strict-env 没带 --fence-live" },
  "target/cross-platform": { status: ["pass", "fail", "quality-fail"], envSkip: "安全探针在 --fence-live 下跳过" },
  "target/windows": { status: ["pass", "fail", "quality-fail", "skip-platform"], envSkip: "非本平台且整目标为空记 skip-platform；安全探针跳过记 quality-fail" },
  "target/linux": { status: ["pass", "fail", "quality-fail", "skip-platform"], envSkip: "非本平台且整目标为空记 skip-platform；安全探针跳过记 quality-fail" },
  "target/macos": { status: ["pass", "fail", "quality-fail", "skip-platform"], envSkip: "非本平台且整目标为空记 skip-platform；安全探针跳过记 quality-fail" },
  "frontend-smoke": { status: ["pass", "fail"], envSkip: null },
  "frontend-typecheck": { status: ["pass", "quality-fail", "env-skip"], envSkip: "node_modules 未安装（--strict-env 下算失败）" },
  "selfcheck/dev-loop": { status: ["pass", "fail"], envSkip: null },
  "selfcheck/pins-tool": { status: ["pass", "fail"], envSkip: null },
  "selfcheck/ci-changes": { status: ["pass", "fail"], envSkip: null },
  "selfcheck/env-bootstrap": { status: ["pass", "fail", "env-skip"], envSkip: "非 Windows 平台那条路径不存在（自检打 [探针]）" },
  e2e: { status: ["pass", "fail", "gap"], envSkip: null },
  "outside-writes": { status: ["pass", "fail"], envSkip: null },
};

const ALLOWED_STATUS = ["pass", "fail", "quality-fail", "env-skip", "skip-platform", "gap"];

const BY_ID = new Map(STEPS.map((s) => [s.id, s]));

/** 步骤自测：id 唯一、名字不空、依赖存在且无环（步骤表是调度与 --only 的唯一依据）。 */
function selfTest() {
  const problems = [];
  const bad = (m) => problems.push("步骤表自检失败：" + m);
  const seen = new Set();
  for (const s of STEPS) {
    if (!s.id) bad("有步骤缺 id");
    if (seen.has(s.id)) bad("步骤 id 重复：" + s.id);
    seen.add(s.id);
    if (!s.name) bad(s.id + " 缺步骤名（报告与文档的锚点）");
    if (typeof s.run !== "function") bad(s.id + " 缺 run");
    if (!Array.isArray(s.inputs) || !s.inputs.length) bad(s.id + " 缺 inputs（--changed 依赖它反查）");
    if (!Array.isArray(s.deps)) bad(s.id + " 缺 deps");
  }
  for (const s of STEPS) {
    for (const d of s.deps) if (!seen.has(d)) bad(s.id + " 依赖了不存在的步骤：" + d);
  }
  // 状态声明：每步都要有，取值必须是既有四态之一，且必须能 pass（否则这一项永远红）。
  for (const s of STEPS) {
    const o = OUTCOMES[s.id];
    if (!o) { bad(s.id + " 缺状态声明（OUTCOMES）"); continue; }
    if (!Array.isArray(o.status) || !o.status.length) bad(s.id + " 的状态声明为空");
    for (const st of o.status) if (!ALLOWED_STATUS.includes(st)) bad(s.id + " 声明了未知状态：" + st);
    if (!o.status.includes("pass")) bad(s.id + " 的状态声明里没有 pass");
  }
  // 无环：DFS 三色。
  const color = new Map();
  const visit = (id, stack) => {
    if (color.get(id) === 1) { bad("步骤依赖成环：" + stack.concat(id).join(" -> ")); return; }
    if (color.get(id) === 2) return;
    color.set(id, 1);
    for (const d of BY_ID.get(id).deps) visit(d, stack.concat(id));
    color.set(id, 2);
  };
  for (const s of STEPS) visit(s.id, []);
  return problems;
}

/** 只读视图（--list --json / --changed 用）。 */
function listModel() {
  return STEPS.map((s) => ({
    id: s.id,
    name: s.name,
    deps: s.deps.slice(),
    inputs: s.inputs.slice(),
    requiresFull: !!s.requiresFull,
    status: (OUTCOMES[s.id] || { status: [] }).status.slice(),
    envSkip: (OUTCOMES[s.id] || { envSkip: null }).envSkip,
  }));
}

module.exports = { STEPS, selfTest, listModel, E2E_TIMEOUT_MS, ENV_BOOTSTRAP_TIMEOUT_MS };
