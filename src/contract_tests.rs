//! T2：端口与适配器契约测试（唯一权威见 TESTING.md §六 端口测试矩阵）。
//! 替身语义在这里统一验收（成功 / 失败 / 空 / 边界 / 重复 / 清理）；真实适配器边界用隔离根跑，跑完清理。
//! 硬规矩：不碰真实 `.home/`、真实 `session/`、真实权限或外部网络；只绑本地环回。
//!
//! 长期迁移目标（记在 tests/gaps.yaml）：本模块与 src/tests.rs 合并为 src/tests/ 目录
//! （doubles / ports / fakes / adapters 各一文件），替身与契约测试同处一层。

mod adapters;
mod api;
mod fakes;
mod ports;

/// 隔离落点：`target/test-scratch/contract/<name>`（target/ 不入库）。每次先清空再建。
pub(crate) fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-scratch")
        .join("contract")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("建契约测试隔离根");
    d
}
