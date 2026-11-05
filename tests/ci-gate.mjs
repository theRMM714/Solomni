#!/usr/bin/env node
/**
 * 验收闸门判定（CI 的 gate job 唯一逻辑；--self-test 覆盖所有分支）。
 * 管：读 CHANGES / CODE_CHANGED / DOCS_CHANGED / DOCS / STRUCTURE / WORKFLOW / QUALITY / E2E 环境变量，
 *     写 step summary 与 ::error::，给出合并判定；--self-test 用纯输入跑 8 个样例。
 * 不管：跑测试、产报告（run-tests.js / ci-merge.mjs）与规则集本身（仓库外事实，见 docs/testing/execution-ci.md）。
 * 约束：只读环境变量与（可选）GITHUB_STEP_SUMMARY；不联网、不写仓库文件；lane 名必须与 test.yml 的 job 名一一对应。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

// lane 顺序即报告顺序。workflow 恒跑（不按变更类型跳过），所以永不允许 skipped。
const LANES = [
  { key: "docs", name: "文档审查" },
  { key: "structure", name: "结构审查" },
  { key: "workflow", name: "工作流静态检查" },
  { key: "quality", name: "质量与单元" },
  { key: "e2e", name: "端到端" },
];

/**
 * 纯判定：输入是各 job 的 result 与分类结果，输出合并结论。
 * 允许跳过的判据只有一条：本次变更本来就不跑这个 lane（分类字段显式为 "false"）。
 * 分类字段缺失 / 取不到 diff（空串）按"有变更"处理，与 test.yml 的 `!= 'false'` 兜底一致。
 */
export function decide(input) {
  const env = input || {};
  const allowDocs = env.docsChanged === "false";
  const allowCode = env.codeChanged === "false";
  const allow = { docs: allowDocs, structure: false, workflow: false, quality: allowCode, e2e: allowCode };
  const lanes = LANES.map((l) => Object.assign({}, l, { result: env[l.key] || "", allowSkip: allow[l.key] }));
  const failed = lanes.filter((l) => l.result !== "success" && !(l.result === "skipped" && l.allowSkip));
  return { changes: env.changes || "", changesOk: env.changes === "success", lanes, failed, ok: failed.length === 0 };
}

/** 与旧内联脚本同格式的一行 step summary。 */
function summaryLine(l) {
  return "- " + l.name + "：" + (l.result || "（缺失）") + "（允许跳过：" + (l.allowSkip ? "yes" : "no") + "）";
}

function main() {
  const d = decide({
    changes: process.env.CHANGES,
    codeChanged: process.env.CODE_CHANGED,
    docsChanged: process.env.DOCS_CHANGED,
    docs: process.env.DOCS,
    structure: process.env.STRUCTURE,
    workflow: process.env.WORKFLOW,
    quality: process.env.QUALITY,
    e2e: process.env.E2E,
  });
  if (process.env.GITHUB_STEP_SUMMARY) {
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, d.lanes.map(summaryLine).join("\n") + "\n");
  }
  // changes 只是路由：它失败时下游各 job 已按全量兜底（取不到 diff 也是），所以这里只告警不算失败。
  if (!d.changesOk) console.log("::warning title=验收闸门::变更分流未成功，按全量兜底核对");
  for (const l of d.failed) console.log("::error::" + l.name + " 未通过：result=" + (l.result || "（缺失）"));
  if (!d.ok) process.exit(1);
  console.log("GATE-OK");
}

const OK = "success";
const full = {
  changes: OK,
  codeChanged: "true",
  docsChanged: "true",
  docs: OK,
  structure: OK,
  workflow: OK,
  quality: OK,
  e2e: OK,
};
// changes 失败 / 取不到 diff：两个分类字段都是空串，下游两路都跑，跳过一律不合法。
const changesFailed = {
  changes: "failure",
  codeChanged: "",
  docsChanged: "",
  docs: OK,
  structure: OK,
  workflow: OK,
  quality: OK,
  e2e: OK,
};
const CASES = [
  {
    name: "纯文档：docs 跑、平台 job 跳过合法",
    input: { changes: OK, codeChanged: "false", docsChanged: "true", docs: OK, structure: OK, workflow: OK, quality: "skipped", e2e: "skipped" },
    ok: true,
    failed: [],
  },
  {
    name: "纯代码：docs 跳过合法、平台 job 必须跑",
    input: { changes: OK, codeChanged: "true", docsChanged: "false", docs: "skipped", structure: OK, workflow: OK, quality: OK, e2e: OK },
    ok: true,
    failed: [],
  },
  {
    name: "混合：两路都跑",
    input: full,
    ok: true,
    failed: [],
  },
  {
    name: "有代码变更时 quality 必须跑（workflow 恒跑，跳过同样拦）",
    input: Object.assign({}, full, { workflow: "skipped", quality: "skipped" }),
    ok: false,
    failed: ["workflow", "quality"],
  },
  {
    name: "structure 恒跑：跳过即失败",
    input: Object.assign({}, full, { structure: "skipped" }),
    ok: false,
    failed: ["structure"],
  },
  {
    name: "有文档变更时 docs 必须跑：跳过即失败",
    input: { changes: OK, codeChanged: "false", docsChanged: "true", docs: "skipped", structure: OK, workflow: OK, quality: "skipped", e2e: "skipped" },
    ok: false,
    failed: ["docs"],
  },
  {
    name: "changes 失败：按全量兜底，全跑合法；兜底时跳过仍失败",
    input: changesFailed,
    ok: true,
    failed: [],
    // 兜底不是"跳过合法"：取不到分类时 workflow/quality 的 skipped 仍然要拦。
    extra: () => {
      assert.equal(decide(Object.assign({}, changesFailed, { workflow: "skipped" })).ok, false, "兜底时 workflow 跳过必须失败");
      assert.equal(decide(Object.assign({}, changesFailed, { quality: "skipped" })).ok, false, "兜底时 quality 跳过必须失败");
    },
  },
  {
    name: "cancelled：任何 lane 被判 cancelled 都不算通过",
    input: { changes: "cancelled", codeChanged: "", docsChanged: "", docs: "cancelled", structure: "cancelled", workflow: "cancelled", quality: "cancelled", e2e: "cancelled" },
    ok: false,
    failed: ["docs", "structure", "workflow", "quality", "e2e"],
  },
];

function selfTest() {
  const bad = [];
  for (const c of CASES) {
    try {
      const got = decide(c.input);
      assert.equal(got.ok, c.ok, c.name + "：ok 期望 " + c.ok + "，实际 " + got.ok);
      assert.deepEqual(got.failed.map((l) => l.key), c.failed, c.name + "：失败 lane 不符");
      if (c.extra) c.extra(got);
    } catch (e) {
      bad.push(e.message);
    }
  }
  if (bad.length) {
    for (const b of bad) console.error("自检失败：" + b);
    process.exit(1);
  }
  console.log("CI-GATE-SELF-TEST-OK（" + CASES.length + " 例）");
}

const invoked = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  if (process.argv.includes("--self-test")) selfTest();
  else main();
}
