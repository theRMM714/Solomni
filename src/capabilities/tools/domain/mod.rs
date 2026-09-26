//! 纯逻辑：工具放行与寻址、参数契约、补丁解析、角色表、围栏策略。没有 IO，也不加 trait。

pub mod fence;
pub mod patch;
pub mod roles;
pub mod schema;
pub mod systool;
