/**
 * kernel 机制表的解析器：把 docs/kernel/mechanisms.yaml 的「机制: 层号」行读成 Map。
 * 与门禁分开放的理由：解析器决定红绿，自测要能脱离磁盘直接喂样例（tests/lib/parsers.js 的 gateParserSelfTest）。
 * 机制名只认「分区/名字」（ports 单列）：名字不合规当场报，别让错名被当成新机制静默登记。
 */
"use strict";

const NAME_RE = /^(ports|(contracts|domain|detail)\/[a-z_][a-z0-9_]*)$/;

/** 解析机制表正文：一行一条「机制: 层号」，井号之后是注释。返回 { tiers, problems }。 */
function parseMechanismsText(text) {
  const tiers = new Map();
  const problems = [];
  let lineNo = 0;
  for (const raw of String(text).split(/\r?\n/)) {
    lineNo++;
    const line = raw.replace(/#.*$/, "").trim();
    if (!line) continue;
    const m = line.match(/^([^:\s]+):\s*(\d+)$/);
    if (!m || !NAME_RE.test(m[1])) {
      problems.push("机制分层：机制表第 " + lineNo + " 行不是「机制: 层号」或机制名不合规：" + raw.trim());
      continue;
    }
    if (tiers.has(m[1])) {
      problems.push("机制分层：机制表里 " + m[1] + " 归了两次层");
      continue;
    }
    tiers.set(m[1], Number(m[2]));
  }
  return { tiers, problems };
}

module.exports = { parseMechanismsText, NAME_RE };
