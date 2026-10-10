#!/usr/bin/env node
/**
 * 运行环境安装：让产品能跑起来——项目内 Rust 工具链（系统里有可用 Rust 则只读借用，没有就装进项目内）与平台依赖。
 * 管：运行所需的最小集合；产物只落 platform/<os>/ 与 .tools/。
 * 不管：开发工具（组件 / 交叉 target / 供应链工具）——那是 setup-dev.js。
 * 用法：node setup-runtime.js [--yes]（--yes = 允许装进项目内，不问）
 */
"use strict";
const env = require("./env.js");

(async () => {
  const yes = process.argv.includes("--yes");
  const ready = await env.ensure({ yes });
  console.log("[setup-runtime] 环境就绪：" + ready.cargo + " [" + ready.source + "]");
  console.log("[setup-runtime] 平台目录：" + ready.platformDir);
  if (ready.source !== "project") {
    console.log("[setup-runtime] 说明：当前借用系统工具链（只读）；开发环境请用 node setup-dev.js 装项目内工具链。");
  }
})();
