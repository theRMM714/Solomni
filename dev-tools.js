/**
 * 开发环境清单：**唯一真相**——setup-dev.js 按它装，run-tests.js 按它查 / 跳过。
 * 管：开发与门禁需要的 Rust 组件、交叉 target、项目内工具（crates）与只探测的运行时。
 * 不管：运行环境（那是 setup-runtime.js 与 env.js 的事）；安装方式（在 setup-dev.js 一处）。
 * 约束：条目只写「名字 + 为什么」；新增一项先想清楚「门禁是不是真的需要它」。
 */
"use strict";

module.exports = {
  // rustup component add：缺了对应门禁项会 env-skip，不硬失败。
  components: [
    { name: "rustfmt", why: "T0 格式（cargo fmt --check）" },
    { name: "clippy", why: "T0 静态检查（cargo clippy -D warnings）" },
    { name: "llvm-tools-preview", why: "覆盖率发现模式（node run-tests.js --coverage）" },
  ],

  // rustup target add + cargo check --target：平台专属 #[cfg] 本机不编译，靠它兜类型错误。
  // macOS 两项在缺 Apple SDK / 交叉 C 工具链的机器上仍会 env-skip（真机检查归该平台真机 CI）。
  targets: [
    { triple: "x86_64-pc-windows-msvc", why: "Windows 专属 #[cfg] 的类型检查" },
    { triple: "aarch64-apple-darwin", why: "macOS 专属 #[cfg] 的类型检查" },
    { triple: "x86_64-apple-darwin", why: "macOS 专属 #[cfg] 的类型检查" },
  ],

  // cargo install --locked --root .tools：落 .tools/bin，门禁从 PATH 找；缺了对应项 env-skip。
  crates: [
    { name: "cargo-audit", why: "T0 供应链（cargo audit）" },
    { name: "cargo-deny", why: "T0 供应链（cargo deny check）" },
    { name: "cargo-llvm-cov", why: "覆盖率发现模式（--coverage）" },
  ],

  // 只探测、不代装：缺了相关测试会如实 env-skip（探针行可见）。
  probe: [
    { name: "node", why: "前端冒烟与 node 模块工具" },
    { name: "python", why: "python 模块工具与真实 MCP 端到端" },
  ],
};
