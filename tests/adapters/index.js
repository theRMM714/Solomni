//! 目的：三种派发形态的注册表——编排器只认这张表，不认具体形态。
//! 管：给出一组适配器（顺序即执行顺序）与按形态取目标的方法。
//! 不管：选哪些用例（tests/lib/plan.js）；怎么跑（各适配器自己）。
//! 联动：tests/lib/plan.js 的 groups；run-tests.js 的子集执行。

const inproc = require("./inproc.js");
const cli = require("./cli.js");
const http = require("./http.js");

const ALL = [inproc, cli, http];

module.exports = { ALL, inproc, cli, http };
