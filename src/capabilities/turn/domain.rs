//! 目的：turn 的纯逻辑——会话装配里可单独验的派生规则。
//! 管：`assemble` 一个子模块（角色工具面过滤、已就绪服务操作归组、模块工具行与服务参数行）。
//! 不管：IO、端口读取与最终对象拼装（在 `service/assemble.rs`）。
//! 联动：`turn::service::assemble` 装配一席工具面时调用。

pub mod assemble;
