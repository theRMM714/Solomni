//! 目的：版本库单元——一次工作里的共享区怎么变：提交、拉取、推送、状态、差异、回退、留档与回收。
//! 管：`api`（入站契约 `Versions` 与版本化视图）、`ports`（落盘端口）、`domain`（纯规则）、`service`（用例与底座原语）、`detail`（落盘机制）。
//! 不管：目录布局与文件寻址（问 `workspace`）；哪一席能碰哪些路径（`workspace` 的权限语义）；工具面与授权（`agentloop`）。
//! 联动：组合根注入 `Arc<dyn workspace::api::Workspace>`（回答"在哪儿"）与 `FsWorkStore`（落盘）；`work`、`turn`、`tools` 经 `api` 取。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
