/**
 * 结构审查·门禁自测：决定红绿的解析器与快照比较必须能自证（会静默全绿的门禁比缺工具更危险）。
 * 解析器自测跟着函数走（tests/lib/parsers.js）；项目外写自测含"监视根派生"一条。
 */
"use strict";
const { gateParserSelfTest } = require("../../lib/parsers.js");
const { outsideSelfTest, watchRootsSelfTest } = require("../../lib/outside.js");

function parsers(root) {
  return gateParserSelfTest(root);
}

function outside(root) {
  return outsideSelfTest().concat(watchRootsSelfTest());
}

module.exports = { parsers, outside };
