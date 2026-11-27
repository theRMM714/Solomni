//! 目的：沙箱单元的契约——一个执行环境策略要回答什么。
//! 管：`SandboxPolicy`（自我介绍、声明校验、事实、开一席的执行句柄）、`OpenRequest`（开的时候要的东西）。
//! 不管：实现（本机在 `tools`、虚拟机在 `vm`）；注册（`service`）。
//! 联动：组合根把实现登记进 `SandboxPolicies`；装配期按声明 `open` 出执行句柄，引擎只认 `Sandbox`。

use std::path::Path;
use std::sync::Arc;

use crate::units::business::sandbox::domain::decl::SandboxDecl;
use crate::units::business::sandbox::domain::exec::Sandbox;
use crate::units::business::sandbox::domain::facts::{PolicyDecl, SandboxFacts};

/// 目的：开一席执行句柄时要的东西——会话、租约、声明、事实与运行期的宿主根。
/// 约束：这里只有装配期就已知的值；实现不得再去读会话或工作区。
pub struct OpenRequest<'a> {
    /// 目的：会话 id（也是租约名）。
    /// 约束：端口契约——策略若要按会话区分（如每会话一台机），从这里取；今天的两个实现不用。
    #[allow(dead_code)]
    pub session: &'a str,
    /// 目的：这一席的租约（客机按它复用同一台机；本机档为空）。
    pub lease: &'a str,
    /// 目的：这一席的执行环境声明。
    pub decl: &'a SandboxDecl,
    /// 目的：这份声明的事实（哪一侧、客机是什么）。
    /// 约束：端口契约——策略若要按事实区分（如客机 OS），从这里取；今天的两个实现不用。
    #[allow(dead_code)]
    pub facts: &'a SandboxFacts,
    /// 目的：运行期宿主根（模块运行包要用；没有就不给）。
    pub runtime_path: Option<&'a Path>,
}

/// 目的：执行环境策略——会话只写策略 id 与参数，解释权全在实现方。
pub trait SandboxPolicy: Send + Sync {
    /// 目的：策略的自我介绍（id、名字、说明、参数声明）。
    fn decl(&self) -> PolicyDecl;

    /// 目的：这份声明对这个策略合不合法（空 = 合法）。
    fn check(&self, decl: &SandboxDecl) -> Vec<String>;

    /// 目的：这份声明对应的事实（哪一侧、客机是什么）。
    /// 错误：策略 id 不是自己的，或参数缺了必填项。
    fn facts(&self, decl: &SandboxDecl) -> Result<SandboxFacts, String>;

    /// 目的：按这份声明开一席的执行句柄。
    /// 错误：这一侧缺了跑起来必需的东西（如客机没有租约、虚拟根不可用）。
    fn open(&self, req: &OpenRequest<'_>) -> Result<Arc<dyn Sandbox>, String>;
}
