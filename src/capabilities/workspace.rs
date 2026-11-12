//! 目的：工作区能力——沙箱寻址（含越界判定）、建工作与上传、文件视图、共享区版本化。
//! 管：**能碰到哪儿**（work 共享区与各 agent 私有沙箱）与**工作怎么建、怎么提交与还原**
//!   （内容寻址的提交记录、拉取基线、文件级三方比较与按提交点还原）。
//! 不管：目录布局与落盘机制（在 `detail/`）；会话 meta 的事实（在 `session`）。
//! 联动：其它能力经 `api` 使用；`tools` 用 `Place` / `Sandbox` 定位读写落点。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
