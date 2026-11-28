//! 目的：沙箱单元的入站面——声明、事实、执行端口、注册表与策略端口。
//! 管：对其它单元与呈现层暴露的形状与入口（镜像契约、执行端口、就绪判定、策略注册表）。
//! 不管：实现（在各策略所在的单元）；装配（组合根）。
//! 联动：`session_model` 存 `SandboxDecl`；`work` 与呈现层读 `SandboxFacts` / `PolicyDecl`。

#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::decl::{HOST_POLICY, SandboxDecl, VM_POLICY};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::exec::{
    ExecKind, ExecOutcome, ExecRequest, Sandbox, refused,
};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::facts::{PolicyDecl, SandboxFacts};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::guest::{GuestArch, GuestOs};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::image::{
    GuestAgent, GuestClients, GuestImage, classify_base, guest_platform, write_image,
};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::policy::{OpenRequest, SandboxPolicy};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::domain::readiness::{Machine, Readiness, Requirement};
#[allow(unused_imports)]
pub use crate::units::business::sandbox::service::SandboxPolicies;
