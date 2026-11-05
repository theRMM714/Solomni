/**
 * 成功 / 失败标记的唯一声明：文本与含义只在这里写一遍。
 * 目的：入口与门禁打印点引用常量；--list --json 的 markers[] 让文档按同一份声明生成"标记 → 含义"，
 *   不再各处手写字符串（散落的字面量改一处漏一处，读报告的人也会把不同标记当成一回事）。
 * 不归本入口打印的标记（e2e 编排、CI 发布）也在这里登记 owner，便于核对谁负责。
 */
"use strict";

const TEXT = {
  REPORT_OK: "TEST-REPORT-OK",
  REPORT_FAIL: "TEST-REPORT-FAIL",
  REPORT_ACCEPTED: "TEST-REPORT-ACCEPTED",
  FRONTEND_SMOKE_OK: "FRONTEND-SMOKE-OK",
  DEV_LOOP_OK: "DEV-LOOP-OK",
  PINS_TOOL_OK: "PINS-TOOL-OK",
  ENV_BOOTSTRAP_OK: "ENV-BOOTSTRAP-OK",
  E2E_OK: "E2E-OK",
  E2E_FAILED: "E2E-FAILED",
  MUTATION_OK: "MUTATION-OK",
  MUTATION_FOUND: "MUTATION-FOUND",
  GATE_OK: "GATE-OK",
};

const MARKERS = [
  { text: TEXT.REPORT_OK, owner: "run-tests.js", meaning: "本次车道的步骤没有失败；--docs / --audit 模式下只代表对应审查通过，不代表代码 / 三平台 / L4 已验收" },
  { text: TEXT.REPORT_FAIL, owner: "run-tests.js", meaning: "本次车道有硬失败步骤或 T0 任一项不过（同时以非零退出码暴露）" },
  { text: TEXT.REPORT_ACCEPTED, owner: "run-tests.js", meaning: "本平台与跨平台缺口账为空；只在整跑模式打印，审查模式不打印" },
  { text: TEXT.FRONTEND_SMOKE_OK, owner: "src/presentation/web/assets/smoke.cjs", meaning: "前端冒烟全部通过（steps.js 据此判通过）" },
  { text: TEXT.DEV_LOOP_OK, owner: "tests/dev-loop.mjs", meaning: "dev.js 的固定子命令表、cargo -- 透传口与拒绝出口自检通过" },
  { text: TEXT.PINS_TOOL_OK, owner: "tests/pins-tool.mjs", meaning: "pins.js 的台账解析 / 来源推导 / 原地重写自检通过（不碰网络）" },
  { text: TEXT.ENV_BOOTSTRAP_OK, owner: "tests/env-bootstrap.mjs", meaning: "env.js 的下载 / 续传 / 结构判定 / sha256 闸自检通过；非 Windows 那部分记 [探针]" },
  { text: TEXT.E2E_OK, owner: "tests/cross-platform/e2e/orchestrator.js", meaning: "L4 端到端编排通过（整跑与 ci-e2e 都按它判）" },
  { text: TEXT.E2E_FAILED, owner: "tests/cross-platform/e2e/orchestrator.js", meaning: "L4 端到端编排失败（失败证据在 e2e 日志）" },
  { text: TEXT.MUTATION_OK, owner: "tests/ci-mutation.mjs", meaning: "突变测试全捕获（手动工作流，不参与验收）" },
  { text: TEXT.MUTATION_FOUND, owner: "tests/ci-mutation.mjs", meaning: "突变测试有漏网（手动工作流，不参与验收）" },
  { text: TEXT.GATE_OK, owner: "tests/ci-gate.mjs", meaning: "CI 验收闸门按变更分类核对五个 lane（docs / structure / workflow / quality / e2e）全部通过" },
];

module.exports = { TEXT, MARKERS };
