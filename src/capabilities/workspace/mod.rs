//! 工作区能力：运行包库、执行档位与沙箱寻址、共享区版本化。
//!
//! 它回答：**这次能跑什么**（依赖文件夹里的运行包 + 模块声明的运行能力，模块事实来自 `modules`）与
//! **能碰到哪儿**（work 共享区与各 agent 私有沙箱）。目录布局与落盘机制在适配层。

pub mod api;
pub mod detail;
pub mod domain;
pub mod ports;
pub mod service;
