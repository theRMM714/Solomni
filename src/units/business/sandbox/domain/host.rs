//! 目的：本机策略——不套客机，直接在这一侧跑。
//! 管：`HostPolicy`（登记项、参数校验、事实）。
//! 不管：本机侧怎么隔离（内核围栏，见 `tools` 与 `kernel`）。
//! 联动：注册表 `Sandboxes` 的第一个登记项；`SandboxDecl::host()` 对应它。

use crate::units::business::sandbox::domain::decl::{HOST_POLICY, SandboxDecl};
use crate::units::business::sandbox::domain::facts::{PolicyDecl, SandboxFacts};
use crate::units::business::sandbox::domain::policy::SandboxPolicy;

/// 目的：本机执行环境策略（没有客机，也没有参数）。
pub struct HostPolicy;

impl SandboxPolicy for HostPolicy {
    fn decl(&self) -> PolicyDecl {
        PolicyDecl {
            id: HOST_POLICY.to_string(),
            name: "本机".to_string(),
            hint: "直接在本机跑（不套客机）".to_string(),
            params: None,
        }
    }

    fn check(&self, decl: &SandboxDecl) -> Vec<String> {
        let mut out = Vec::new();
        if !decl.params.is_null() {
            out.push("本机策略没有参数：params 留空".to_string());
        }
        if !decl.pins.is_empty() {
            out.push("本机策略没有定版项：pins 留空".to_string());
        }
        out
    }

    fn facts(&self, _decl: &SandboxDecl) -> Result<SandboxFacts, String> {
        Ok(SandboxFacts::host())
    }
}
