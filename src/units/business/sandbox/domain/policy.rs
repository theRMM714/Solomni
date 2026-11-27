//! 目的：沙箱单元的契约——一个执行环境策略要回答什么。
//! 管：`SandboxPolicy`（自我介绍、声明校验、事实）。
//! 不管：实现（本机在 `sandbox`、虚拟机在 `vm`）；注册（`service`）。
//! 联动：组合根把实现登记进 `SandboxPolicies`；消费方只经 `api` 取。

use crate::units::business::sandbox::domain::decl::SandboxDecl;
use crate::units::business::sandbox::domain::facts::{PolicyDecl, SandboxFacts};

/// 目的：执行环境策略——会话只写策略 id 与参数，解释权全在实现方。
pub trait SandboxPolicy: Send + Sync {
    /// 目的：策略的自我介绍（id、名字、说明、参数声明）。
    fn decl(&self) -> PolicyDecl;

    /// 目的：这份声明对这个策略合不合法（空 = 合法）。
    fn check(&self, decl: &SandboxDecl) -> Vec<String>;

    /// 目的：这份声明对应的事实（哪一侧、客机是什么）。
    /// 错误：策略 id 不是自己的，或参数缺了必填项。
    fn facts(&self, decl: &SandboxDecl) -> Result<SandboxFacts, String>;
}
