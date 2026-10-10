//! 目的：隐秘字段的机制实现（只有组合根能构造）。
//! 管：文件落盘实现 YamlSecrets。
//! 不管：端口定义（在 ports）；声明解析（在 workspace）。
//! 联动：由 service.rs 经 SecretStore 端口消费。

pub mod yaml_secrets;

pub use yaml_secrets::YamlSecrets;
