/**
 * 分层表的解析器（受限子集：「名字: 层号」，井号之后是注释，名字不合规当场报）。
 * 解析器与门禁分开放的理由：解析器决定红绿，自测要能脱离磁盘直接喂样例（tests/lib/parsers.js 的 gateParserSelfTest）。
 * 两张表共用这一份：kernel 机制表（docs/kernel/mechanisms.yaml，名字写成分区/名字）与
 * 入口分层表（docs/entry/layers.yaml，名字就是模块名）——只有名字规则与报错用词不同。
 */
"use strict";

/**
 * 解析一张分层表的正文。
 * 参数：`text` 是文件内容；`opts.label` 是报错前缀（如「机制分层」）、`opts.table` 是表的叫法（「机制表」）、
 *       `opts.noun` 是条目的叫法（「机制」）、`opts.nameRe` 是名字必须匹配的正则。
 * 返回：`{ tiers: Map<名字, 层号>, problems: string[] }`。
 */
function parseTierTableText(text, opts) {
  const { label, table, noun, nameRe } = opts;
  const tiers = new Map();
  const problems = [];
  let lineNo = 0;
  for (const raw of String(text).split(/\r?\n/)) {
    lineNo++;
    const line = raw.replace(/#.*$/, "").trim();
    if (!line) continue;
    const m = line.match(/^([^:\s]+):\s*(\d+)$/);
    if (!m || !nameRe.test(m[1])) {
      problems.push(label + "：" + table + "第 " + lineNo + " 行不是「" + noun + ": 层号」或" + noun + "名不合规：" + raw.trim());
      continue;
    }
    if (tiers.has(m[1])) {
      problems.push(label + "：" + table + "里 " + m[1] + " 归了两次层");
      continue;
    }
    tiers.set(m[1], Number(m[2]));
  }
  return { tiers, problems };
}

const NAME_RE = /^(ports|(contracts|domain|detail)\/[a-z_][a-z0-9_]*)$/;

/** 解析 kernel 机制表正文（名字只认「分区/名字」，ports 单列）。返回 { tiers, problems }。 */
function parseMechanismsText(text) {
  return parseTierTableText(text, { label: "机制分层", table: "机制表", noun: "机制", nameRe: NAME_RE });
}

module.exports = { parseTierTableText, parseMechanismsText, NAME_RE };
