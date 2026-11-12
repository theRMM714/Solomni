/**
 * 结构审查·注释契约：判定只有一处（run-hygiene.js 的 scanCommentContract）。
 * 零豁免：扫描出的每一处违规都直接失败，不再有存量快照或销账流程。
 * 门禁自己也要被测：run-hygiene 的两项自测在这里一起跑（注释词法 / 语义卫生）。
 */
"use strict";
const path = require("path");

function contract(root) {
  const problems = [];
  const hygiene = require(path.join(root, "run-hygiene.js"));
  for (const f of hygiene.scanCommentContract().findings) {
    problems.push("注释契约违规：" + f.file + " :: " + f.rule);
  }
  return { problems, hygiene };
}

/** 两项自测：任一条失败都算结构问题（门禁可信度是硬要求）。 */
function hygieneSelfTests(root) {
  const problems = [];
  const hygiene = require(path.join(root, "run-hygiene.js"));
  for (const b of hygiene.commentLexerSelfTest()) problems.push("注释词法自测失败：" + b);
  for (const b of hygiene.semanticSelfTest()) problems.push("语义卫生自测失败：" + b);
  return problems;
}

module.exports = { contract, hygieneSelfTests };
