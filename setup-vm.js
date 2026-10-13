#!/usr/bin/env node
/**
 * 目的：虚拟机档本地环境安装的**分发入口**——按平台交给对应的 setup-vm-<平台>.js。
 * 管：认平台、转发参数（--check / --yes / --offline）、把要跑的入口如实告诉用户。
 * 不管：具体装什么（在各平台脚本里）；产品对 VM 档的检测（在 src/capabilities/vm/）。
 * 联动：setup-vm-common.js（共享件）、setup-vm-linux.js / setup-vm-windows.js / setup-vm-mac.js。
 * 用法：node setup-vm.js [--check] [--yes] [--offline]（等价于跑本平台那一个）
 */
"use strict";
const { spawnSync } = require("child_process");
const path = require("path");

const P = process.platform === "win32" ? "windows" : process.platform === "darwin" ? "mac" : "linux";
const entry = path.join(__dirname, "setup-vm-" + P + ".js");
console.log("[setup-vm] 平台 " + process.platform + " → " + path.basename(entry));
const r = spawnSync(process.execPath, [entry].concat(process.argv.slice(2)), { cwd: __dirname, stdio: "inherit" });
process.exit(r.status === null ? 1 : r.status);
