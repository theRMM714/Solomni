/**
 * 入口分层表的解析器：把 docs/entry/layers.yaml 的「模块: 层号」行读成 Map。
 * 与门禁分开放的理由同 kernel 机制表：解析器决定红绿，自测要脱离磁盘直接喂样例。
 * 模块名就是 src/entry 下的文件名/目录名（编排根单列成 main），只要小写标识符。
 */
"use strict";
const { parseTierTableText } = require("./mechanisms.js");

const MODULE_RE = /^[a-z_][a-z0-9_]*$/;
const LABEL = "入口分层";
const TABLE = "入口分层表";

/** 解析入口分层表正文。返回 { tiers, problems }。 */
function parseEntryTiersText(text) {
  return parseTierTableText(text, { label: LABEL, table: TABLE, noun: "模块", nameRe: MODULE_RE });
}

module.exports = { parseEntryTiersText, MODULE_RE, LABEL, TABLE };
