//! 目的：隐秘字段的入站能力面——统一 API SecretOps。
//! 管：字段视图、置值 / 清值、按模块解析注入项、按已知值脱敏；未注入时的空实现 NoSecrets。
//! 不管：存储机制（在 ports/detail）；模块声明解析（在 workspace）；实际注入子进程（消费方按 resolve 取）。
//! 联动：实现见 `service.rs`；由 `main.rs` 装配进 `Ops`。

/// 目的：一个隐秘字段的视图（给设置面看；**只有标识，没有值**）。
#[derive(Debug, Clone)]
pub struct SecretView {
    pub module: String,
    pub name: String,
    /// 目的：注入该模块工具/服务进程的环境变量名。
    pub env: String,
    pub desc: String,
    /// 目的：用户配置了没有（值本身永不进视图）。
    pub configured: bool,
}

/// 目的：隐秘字段的**唯一管理 API**——整个项目只从这里调用。
/// 返回：declared 给设置面；resolve 给起进程的一侧（env 名 → 值）；redact 给回执/日志出口。
/// 约束：值永不进提示词、转录、日志、模块工作区与命令行；本 API 不返回给模型看的形状。
pub trait SecretOps: Send + Sync {
    /// 目的：所有模块声明的隐秘字段（含配置了没有），按模块与字段名排序稳定。
    fn declared(&self) -> Result<Vec<SecretView>, String>;
    /// 目的：给一个字段置值（字段必须已被某个模块声明）。
    fn set(&self, module: &str, name: &str, value: &str) -> Result<(), String>;
    /// 目的：清掉一个字段的值（声明还在）。
    fn clear(&self, module: &str, name: &str) -> Result<(), String>;
    /// 目的：某模块已配置字段的注入项（env 名 → 值）；只给起进程的一侧用。
    fn resolve(&self, module: &str) -> Result<Vec<(String, String)>, String>;
    /// 目的：把这段文本里该模块的已知值替换成掩码（回执与日志出口的尽力脱敏）。
    fn redact(&self, module: &str, text: &str) -> Result<String, String>;
}

/// 目的：没有配置隐秘字段存储时的空实现（组合根总是注入真实实现，它供测试装配与未注入兜底）。
#[allow(dead_code)] // 生产路径总注入真实实现；空实现只被测试装配使用。
pub struct NoSecrets;

impl SecretOps for NoSecrets {
    fn declared(&self) -> Result<Vec<SecretView>, String> {
        Ok(Vec::new())
    }
    fn set(&self, _module: &str, _name: &str, _value: &str) -> Result<(), String> {
        Err("没有配置隐秘字段的存储".to_string())
    }
    fn clear(&self, _module: &str, _name: &str) -> Result<(), String> {
        Ok(())
    }
    fn resolve(&self, _module: &str) -> Result<Vec<(String, String)>, String> {
        Ok(Vec::new())
    }
    fn redact(&self, _module: &str, text: &str) -> Result<String, String> {
        Ok(text.to_string())
    }
}
