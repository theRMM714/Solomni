#!/usr/bin/env node
/**
 * 目的：虚拟机档本地环境安装（**macOS**）——骨架：如实报当前缺什么，不假装能装。
 * 管：探测本机已有的 QEMU / guest 基础根，把"还没做"与"可以怎么做"如实说出来。
 * 不管：产品对 VM 档的检测 / 装配（在 src/capabilities/vm/）；Linux / Windows 的环境在 setup-vm-linux.js / setup-vm-windows.js。
 * 用法：node setup-vm-mac.js [--check]
 * 约束：不写系统目录、不装系统包；本平台的一键取包尚未实现（见 tests/gaps.yaml）。
 */
"use strict";
const path = require("path");
const c = require("./setup-vm-common.js");
const { ROOT, log } = c;

log("macOS 的一键取包尚未实现（见 tests/gaps.yaml）。");
log("现在可以做的：");
log("  · 自行安装 QEMU（如 brew install qemu），让它出现在 PATH，或在设置里登记 qemu_path；");
log("  · guest 用磁盘镜像（宿主没有 9p 时 root 必须是磁盘）：把镜像放进 " + path.relative(ROOT, c.BASE) + "，并放一份 guest.yaml；");
log("  · 镜像源见 links.yaml 的 guest.debian.qcow2。");
process.exit(1);
