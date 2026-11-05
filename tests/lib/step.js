/**
 * 步骤框架：编号、用时、状态三件事只在这里算。
 * 目的：main（全量）与 runAudit（审查）用同一套步骤记录，报告字段不再两处各写一遍。
 * 注意：这是**可注入的状态对象**，不是模块级全局——谁新建谁持有，跨文件不共享。
 */
"use strict";

function create() {
  let stepNo = 0;
  let stepStart = Date.now();
  const steps = [];

  /** 开一步：打印序号与标题，并把起跑时刻钉在现在。 */
  function announce(label) {
    stepNo++;
    stepStart = Date.now();
    process.stdout.write("[" + stepNo + "] " + label + " ... ");
  }

  /** 收一步：打印状态、说明与用时。 */
  function announceDone(status, detail) {
    console.log(status + (detail ? "（" + detail + "）" : "") + "  [" + ((Date.now() - stepStart) / 1000).toFixed(1) + "s]");
  }

  /** 记一步：用时自动带上（步骤对象不关心时间时也不用写两遍）。 */
  function push(obj) {
    steps.push(Object.assign({ ms: Date.now() - stepStart }, obj));
  }

  function elapsed() {
    return Date.now() - stepStart;
  }

  return { announce, announceDone, push, elapsed, steps };
}

module.exports = { create };
