/**
 * 命令行解析：开关的**唯一定义处**（组合语义也在这里，别在别处再判一遍 argv）。
 * 目的：入口只做编排；模式优先级是固定梯子，--list 能把它读出来。
 */
"use strict";

const LANES = ["full", "docs", "structure", "selftest"];

/**
 * 解析 argv（不含 node 与脚本名）。
 * 未知参数一律忽略（历史开关与 start.js 的调用方式不变）；已知参数写错值则 invalid。
 */
function parse(argv) {
  const o = {
    release: false,
    serial: false,
    coverage: false,
    docs: false,
    auditScope: null,
    skipAudit: false,
    fenceLive: false,
    skipE2e: false,
    list: false,
    json: false,
    strictEnv: false,
    only: [],
    changed: false,
    run: false,
    help: false,
    invalid: null,
  };
  for (const a of argv) {
    if (a === "--release") o.release = true;
    else if (a === "--serial") o.serial = true;
    else if (a === "--coverage") o.coverage = true;
    else if (a === "--docs") o.docs = true;
    else if (a.startsWith("--audit=")) {
      const s = a.slice("--audit=".length);
      if (s !== "docs" && s !== "structure") { o.invalid = "用法：--audit=docs|structure"; return o; }
      o.auditScope = s;
    } else if (a === "--skip-audit") o.skipAudit = true;
    else if (a === "--fence-live") o.fenceLive = true;
    else if (a === "--skip-e2e") o.skipE2e = true;
    else if (a === "--strict-env") o.strictEnv = true;
    else if (a === "--list") o.list = true;
    else if (a === "--json") o.json = true;
    else if (a.startsWith("--only=")) {
      const v = a.slice("--only=".length).trim();
      if (!v) { o.invalid = "用法：--only=<gate|lane|step>（多个用逗号分隔，可重复）"; return o; }
      for (const x of v.split(",")) if (x.trim()) o.only.push(x.trim());
    }
    else if (a === "--changed") o.changed = true;
    else if (a === "--run") o.run = true;
    else if (a === "--help" || a === "-h") o.help = true;
  }
  o.fenceLive = o.fenceLive || process.env.SOLOMNI_FENCE_LIVE === "1";
  return o;
}

/** 模式梯子：--coverage > --docs > --audit=… > 全量。顺序即优先级，这里和 main 只有一处判定。 */
function mode(opts) {
  if (opts.coverage) return "coverage";
  if (opts.docs) return "docs-all";
  if (opts.auditScope) return "audit-" + opts.auditScope;
  return "full";
}

function isLane(name) {
  return LANES.includes(name);
}

function usage() {
  return [
    "用法：node run-tests.js [开关]",
    "",
    "模式（优先级从高到低）：",
    "  --coverage           覆盖率发现（手动，不写报告）",
    "  --docs               文档审查 + 结构审查（写报告）",
    "  --audit=docs         只跑文档审查（写报告）",
    "  --audit=structure    只跑结构审查（写报告）",
    "  （无模式开关）        整跑：质量门禁 + 单元 + 平台目标 + L4（写报告）",
    "",
    "选择（本地排查用；任意子集不写报告、不打 TEST-REPORT-*）：",
    "  --list [--json]      列出车道 / gate / 步骤（机器可读）",
    "  --only=<id>          跑单个 gate 或步骤（逗号分隔，可重复；车道名=对应车道）",
    "  --changed [--run]    按 git 影响面反查 gate（默认只打印）",
    "",
    "环境：--strict-env       必装工具缺失时记失败而不是 env-skip（CI 用；可选交叉 target 不受影响）",
    "",
    "其它：--skip-e2e / --skip-audit / --serial / --release / --fence-live / --help",
    "判据与报告字段见 TESTING.md 与 docs/testing/execution-ci.md。",
  ].join("\n");
}

module.exports = { LANES, parse, mode, isLane, usage };
