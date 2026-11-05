/**
 * 结构审查·注释契约：判定与棘轮比对只有一处（run-hygiene.js 的 scanCommentContract / compareContract）。
 * 棘轮只减不增：新文件零容忍（快照里没有 = 从 0 起算），老文件同类变多也是新增；变少 = 应销账，同样硬失败。
 * 门禁自己也要被测：run-hygiene 的三项自测在这里一起跑（棘轮 / 注释词法 / 语义卫生）。
 */
"use strict";
const fs = require("fs");
const path = require("path");

function contract(root) {
  const problems = [];
  const hygiene = require(path.join(root, "run-hygiene.js"));
  const rel = (f) => path.relative(root, f).replace(/\\/g, "/");
  const COMMENT_BASELINE = path.join(root, "tests", "comment-baseline.json");
  const contractNow = hygiene.scanCommentContract().byFileRule;
  if (!fs.existsSync(COMMENT_BASELINE)) {
    problems.push("缺注释契约快照：tests/comment-baseline.json（建它：node run-hygiene.js --tighten）");
  } else {
    const baselineRaw = JSON.parse(fs.readFileSync(COMMENT_BASELINE, "utf8"));
    delete baselineRaw._comment;
    const { added, stale } = hygiene.compareContract(contractNow, baselineRaw);
    for (const a of added) problems.push("注释契约新增违规（棘轮只减不增）：" + a);
    for (const s of stale) problems.push("注释契约快照应销账（现在变少了）：" + s + "（收紧：node run-hygiene.js --tighten）");
  }
  return { problems, baseline: rel(COMMENT_BASELINE), hygiene };
}

/** 三项自测：任一条失败都算结构问题（门禁可信度是硬要求）。 */
function hygieneSelfTests(root) {
  const problems = [];
  const hygiene = require(path.join(root, "run-hygiene.js"));
  for (const b of hygiene.ratchetSelfTest()) problems.push("棘轮自测失败：" + b);
  for (const b of hygiene.commentLexerSelfTest()) problems.push("注释词法自测失败：" + b);
  for (const b of hygiene.semanticSelfTest()) problems.push("语义卫生自测失败：" + b);
  return problems;
}

module.exports = { contract, hygieneSelfTests };
