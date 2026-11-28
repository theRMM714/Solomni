//! 目的：沙箱单元的契约——一个执行环境策略要回答什么。
//! 管：`SandboxPolicy`（自我介绍、声明校验、事实、就绪与自带能力、开一席的执行句柄）、`OpenRequest`（开的时候要的东西）。
//! 不管：实现（本机在 `tools`、虚拟机在 `vm`）；注册（`service`）。
//! 联动：组合根把实现登记进 `SandboxPolicies`；装配期按声明 `open` 出执行句柄，引擎只认 `Sandbox`。

use std::path::Path;
use std::sync::Arc;

use crate::units::business::sandbox::domain::decl::SandboxDecl;
use crate::units::business::sandbox::domain::exec::Sandbox;
use crate::units::business::sandbox::domain::facts::{PolicyDecl, SandboxFacts};
use crate::units::business::sandbox::domain::readiness::{Machine, Readiness};
use crate::units::pure::exec_model::api::{Fixer, TargetSpec};

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

    /// 目的：这台机器上这个策略的机器能力（与某条会话无关）：空表 = 什么额外前置都没有。
    fn machine(&self) -> Machine {
        let decl = self.decl();
        Machine {
            policy: decl.id,
            name: decl.name,
            requirements: Vec::new(),
        }
    }

    /// 目的：这条执行目标在这一侧能不能承载（逐项前置；空表 = 齐了）。
    /// 约束：判据只有一份——机器能力（`machine`）加上这条目标自己的选型（虚拟根等）。
    fn readiness(&self, target: &TargetSpec) -> Readiness {
        let _ = target;
        Readiness::default()
    }

    /// 目的：这条目标的执行环境自带哪些运行能力（本机侧为空）。
    fn provided(&self, target: &TargetSpec) -> Vec<String> {
        let _ = target;
        Vec::new()
    }

    /// 目的：承载不成立时的可读拒绝理由（能承载 = None）。
    fn refusal(&self, target: &TargetSpec) -> Option<String> {
        let readiness = self.readiness(target);
        if readiness.ready() {
            return None;
        }
        Some(format!(
            "这侧执行环境现在不可用（{}）：请换另一侧，或先把缺的补上",
            readiness.missing().join("；")
        ))
    }

    /// 目的：机器能力不成立时的可读拒绝理由（能选 = None）。
    fn machine_refusal(&self) -> Option<String> {
        let machine = self.machine();
        if machine.ready() {
            return None;
        }
        Some(format!(
            "这侧执行环境现在不可用（{}）：请换另一侧，或先把缺的补上",
            machine.missing().join("；")
        ))
    }

    /// 目的：这个策略的补齐端口（能补 = Some）：缺什么由 `machine` 说，补什么都由它做。
    /// 约束：默认 None——补不了不假装配一个按钮；`vm` 由组合根注入 `provision` 的实现。
    fn fixer(&self) -> Option<Arc<dyn Fixer>> {
        None
    }

    /// 目的：按这份声明开一席的执行句柄。
    /// 错误：这一侧缺了跑起来必需的东西（如客机没有租约、虚拟根不可用）。
    fn open(&self, req: &OpenRequest<'_>) -> Result<Arc<dyn Sandbox>, String>;
}
