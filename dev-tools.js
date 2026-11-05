/**
 * 开发环境清单：**唯一真相**——setup-dev.js 按它装，run-tests.js 按它查 / 跳过。
 * 管：开发与门禁需要的 Rust 组件、交叉 target、项目内工具（crates）与只探测的运行时。
 * 不管：运行环境（那是产品 provision 与 env.js 的事）；安装方式（在 setup-dev.js 一处）。
 * 约束：条目只写「名字 + 为什么 + 是否必装」。required=false 的缺项不阻断 `--check`（`--strict` 才要求全齐）；
 *      必装 = T0 判据本身（缺了这一步没有结论），可选项 = 提前发现 / 手动模式，CI 另有覆盖。
 */
"use strict";

module.exports = {
  // rustup component add：必装项缺了门禁只能靠 CI；可选项目只服务手动覆盖率。
  components: [
    { name: "rustfmt", why: "T0 格式（cargo fmt --check）", required: true },
    { name: "clippy", why: "T0 静态检查（cargo clippy -D warnings）", required: true },
    // add 的名字是 llvm-tools-preview，装完后 component list 里登记为 llvm-tools（listed）。
    { name: "llvm-tools-preview", listed: "llvm-tools", why: "覆盖率发现模式（node run-tests.js --coverage）", required: false },
  ],

  // rustup target add + cargo check --target：平台专属 #[cfg] 本机不编译，靠它提前发现。
  // 可选：权威验收是 CI 三平台原生矩阵；macOS 两项在缺 Apple SDK / 交叉 C 工具链的机器上仍会 env-skip。
  targets: [
    { triple: "x86_64-pc-windows-msvc", why: "Windows 专属 #[cfg] 的类型检查", required: false },
    { triple: "aarch64-apple-darwin", why: "macOS 专属 #[cfg] 的类型检查", required: false },
    { triple: "x86_64-apple-darwin", why: "macOS 专属 #[cfg] 的类型检查", required: false },
  ],

  // cargo install --locked --root .tools：落 .tools/bin，门禁从 PATH 找。
  // 供应链是 T0 安全项，必装；覆盖率工具只服务手动模式。
  crates: [
    { name: "cargo-audit", why: "T0 供应链（cargo audit）", required: true },
    { name: "cargo-deny", why: "T0 供应链（cargo deny check）", required: true },
    { name: "cargo-llvm-cov", why: "覆盖率发现模式（--coverage）", required: false },
  ],

  // 只探测、不代装：缺了相关测试会如实 env-skip（探针行可见）。
  probe: [
    { name: "node", why: "前端冒烟与 node 模块工具" },
    { name: "python", why: "python 模块工具与真实 MCP 端到端" },
  ],
};
