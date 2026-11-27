//! 目的：版本库的纯逻辑——资产判定、内容指纹、版本新鲜度判定、读取事实、`work_*` 回执拼装、三方比较与提交记录。
//! 管：`asset` / `freshness` / `hash` / `readfact` / `work_tools` / `workstore` 六个子模块。
//! 不管：IO 与适配（读写落盘在 `detail/`）；端口定义（在 `ports.rs`）；目录在哪（问 `workspace`）。
//! 联动：由本单元 `service.rs` 消费；`api.rs` 对外重导出其中的共享类型。

pub mod asset;
pub mod freshness;
pub mod hash;
pub mod readfact;
pub mod work_tools;
pub mod workstore;

/// 目的：跨会话写闸的持有物——拿到它期间，同一外部工作区不能被别的会话写。
/// 约束：只是 `std::fs::File` 的别名；放这里是为了让 service 的签名不直接写盘类型。
pub type SharedGuard = std::fs::File;
