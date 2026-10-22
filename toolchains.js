#!/usr/bin/env node
/**
 * 基础工具链目录（唯一真相）：默认 profile、目标平台与每个工具的固定版本 / 下载键 / 安装布局。
 * 管：给 setup-toolchains.js 提供"装什么、从哪下、解到哪、bin 在哪"；payload 不在这里。
 * 不管：下载 / 校验 / 解包（在 setup-toolchains.js）；启动期 PATH 解析（在 src/entry/toolchains.rs）。
 * 约束：下载 URL 与校验值只在 links.yaml；本文件只写键，不写第二份地址或哈希。
 */
"use strict";
const { link } = require("./env.js");

// 显式 link(...) 调用是 T0 结构审查的消费点，不要改成动态拼键。
const URLS = {
  pythonLinux: link("toolchain.python.3.14.9.linux-x86_64"),
  pythonWindows: link("toolchain.python.3.14.9.windows-x86_64"),
  pythonMacosX64: link("toolchain.python.3.14.9.macos-x86_64"),
  pythonMacosArm64: link("toolchain.python.3.14.9.macos-aarch64"),
  nodeLinux: link("toolchain.node.24.11.0.linux-x86_64"),
  nodeWindows: link("toolchain.node.24.11.0.windows-x86_64"),
  nodeMacosX64: link("toolchain.node.24.11.0.macos-x86_64"),
  nodeMacosArm64: link("toolchain.node.24.11.0.macos-aarch64"),
  zigLinux: link("toolchain.zig.0.15.2.linux-x86_64"),
  zigMacosX64: link("toolchain.zig.0.15.2.macos-x86_64"),
  zigMacosArm64: link("toolchain.zig.0.15.2.macos-aarch64"),
  mingwWindows: link("winlibs.upstream.asset"),
};

const SHAS = {
  pythonLinux: link("toolchain.python.3.14.9.linux-x86_64.sha256"),
  pythonWindows: link("toolchain.python.3.14.9.windows-x86_64.sha256"),
  pythonMacosX64: link("toolchain.python.3.14.9.macos-x86_64.sha256"),
  pythonMacosArm64: link("toolchain.python.3.14.9.macos-aarch64.sha256"),
  nodeLinux: link("toolchain.node.24.11.0.linux-x86_64.sha256"),
  nodeWindows: link("toolchain.node.24.11.0.windows-x86_64.sha256"),
  nodeMacosX64: link("toolchain.node.24.11.0.macos-x86_64.sha256"),
  nodeMacosArm64: link("toolchain.node.24.11.0.macos-aarch64.sha256"),
  zigLinux: link("toolchain.zig.0.15.2.linux-x86_64.sha256"),
  zigMacosX64: link("toolchain.zig.0.15.2.macos-x86_64.sha256"),
  zigMacosArm64: link("toolchain.zig.0.15.2.macos-aarch64.sha256"),
  mingwWindows: link("winlibs.upstream.asset.sha256"),
};

const TARGETS = {
  "linux-x86_64": { os: "linux", arch: "x86_64" },
  "windows-x86_64": { os: "windows", arch: "x86_64" },
  "macos-x86_64": { os: "macos", arch: "x86_64" },
  "macos-aarch64": { os: "macos", arch: "aarch64" },
};

function hostTarget() {
  const os = process.platform === "win32" ? "windows" : process.platform === "darwin" ? "macos" : "linux";
  const arch = process.arch === "x64" ? "x86_64" : process.arch === "arm64" ? "aarch64" : "";
  if (!arch) return null;
  const key = os + "-" + arch;
  return TARGETS[key] ? key : null;
}

const PROFILE = ["python", "node", "cc"];

const TOOLS = {
  python: {
    version: "3.14.9",
    targets: {
      "linux-x86_64": {
        url: URLS.pythonLinux,
        sha256: SHAS.pythonLinux,
        bins: ["python/bin", "python"],
      },
      "windows-x86_64": {
        url: URLS.pythonWindows,
        sha256: SHAS.pythonWindows,
        bins: ["python", "python/Scripts"],
      },
      "macos-x86_64": {
        url: URLS.pythonMacosX64,
        sha256: SHAS.pythonMacosX64,
        bins: ["python/bin", "python"],
      },
      "macos-aarch64": {
        url: URLS.pythonMacosArm64,
        sha256: SHAS.pythonMacosArm64,
        bins: ["python/bin", "python"],
      },
    },
  },
  node: {
    version: "24.11.0",
    targets: {
      "linux-x86_64": {
        url: URLS.nodeLinux,
        sha256: SHAS.nodeLinux,
        bins: ["node-v24.11.0-linux-x64/bin"],
      },
      "windows-x86_64": {
        url: URLS.nodeWindows,
        sha256: SHAS.nodeWindows,
        bins: ["node-v24.11.0-win-x64"],
      },
      "macos-x86_64": {
        url: URLS.nodeMacosX64,
        sha256: SHAS.nodeMacosX64,
        bins: ["node-v24.11.0-darwin-x64/bin"],
      },
      "macos-aarch64": {
        url: URLS.nodeMacosArm64,
        sha256: SHAS.nodeMacosArm64,
        bins: ["node-v24.11.0-darwin-arm64/bin"],
      },
    },
  },
  cc: {
    version: "0.15.2",
    targets: {
      // Windows 复用项目内已有的 winlibs；没有就装到自己的版本目录。
      "windows-x86_64": {
        kind: "mingw",
        version: "16.2.0",
        url: URLS.mingwWindows,
        sha256: SHAS.mingwWindows,
        bins: ["mingw64/bin", "bin"],
      },
      // Linux / macOS：Zig 提供 gcc / g++ / cc / c++ 包装。
      "linux-x86_64": {
        kind: "zig",
        version: "0.15.2",
        url: URLS.zigLinux,
        sha256: SHAS.zigLinux,
        bins: ["bin"],
      },
      "macos-x86_64": {
        kind: "zig",
        version: "0.15.2",
        url: URLS.zigMacosX64,
        sha256: SHAS.zigMacosX64,
        bins: ["bin"],
      },
      "macos-aarch64": {
        kind: "zig",
        version: "0.15.2",
        url: URLS.zigMacosArm64,
        sha256: SHAS.zigMacosArm64,
        bins: ["bin"],
      },
    },
  },
};

/** 目的：工具在某个目标上的固定版本（mingw/zig 的版本可以不同于能力名版本）。 */
function versionOf(id, target) {
  const t = TOOLS[id] && TOOLS[id].targets[target];
  return (t && t.version) || (TOOLS[id] && TOOLS[id].version) || "";
}

/** 目的：安装目录（相对产品根；/ 分隔，跨平台写进 active.json）。 */
function installRel(target, id) {
  return [".tools", "toolchains", target, id, versionOf(id, target)].join("/");
}

module.exports = { PROFILE, TARGETS, TOOLS, hostTarget, versionOf, installRel };
