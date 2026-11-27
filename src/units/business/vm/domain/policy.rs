//! 目的：虚拟机策略——把「客机」这条执行环境登记进 `sandbox` 的注册表。
//! 管：`VmPolicy`（自我介绍、声明校验、客机事实）。
//! 不管：guest 怎么起停（`vm::service`）；会话怎么用它（`work`）。
//! 联动：组合根在装配期经 `vm::api::vm_policy` 建实例并登记。

use crate::kernel::ports::HostProbe;
use crate::units::business::sandbox::api::{
    PolicyDecl, SandboxDecl, SandboxFacts, SandboxPolicy, VM_POLICY,
};
use crate::units::pure::exec_model::api::{Side, TargetSpec};
use crate::units::pure::registry_model::api::{Param, ParamType};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 目的：虚拟机策略：在本机起一台 guest，把这一席的命令送进去跑。
pub struct VmPolicy {
    root: PathBuf,
    probe: Arc<dyn HostProbe + Send + Sync>,
}

impl VmPolicy {
    /// 目的：按项目根与宿主探针建策略（判客机镜像要用它们）。
    pub fn new(root: PathBuf, probe: Arc<dyn HostProbe + Send + Sync>) -> VmPolicy {
        VmPolicy { root, probe }
    }

    fn guest_of(decl: &SandboxDecl) -> Result<String, String> {
        decl.param_str("guest")
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(|g| g.to_string())
            .ok_or_else(|| "虚拟机策略要一个虚拟根：params 里写 guest".to_string())
    }
}

impl SandboxPolicy for VmPolicy {
    fn decl(&self) -> PolicyDecl {
        let mut params = BTreeMap::new();
        params.insert(
            "guest".to_string(),
            Param {
                ty: ParamType::String,
                required: true,
                non_empty: true,
                desc: "虚拟根名（provision 装好的那些之一）".to_string(),
                default: None,
                min: None,
                max: None,
                positional: None,
                rest: false,
                values: None,
                optional_value: false,
            },
        );
        PolicyDecl {
            id: VM_POLICY.to_string(),
            name: "客机".to_string(),
            hint: "在虚拟机里跑（镜像与运行包按 guest 走）".to_string(),
            params: Some(params),
        }
    }

    fn check(&self, decl: &SandboxDecl) -> Vec<String> {
        let mut out = Vec::new();
        match &decl.params {
            serde_json::Value::Null | serde_json::Value::Object(_) => {}
            _ => out.push("虚拟机策略的参数要写成对象：{ guest: <虚拟根名> }".to_string()),
        }
        if let serde_json::Value::Object(map) = &decl.params {
            for key in map.keys() {
                if key != "guest" {
                    out.push(format!("虚拟机策略没有这个参数：{}（只有 guest）", key));
                }
            }
        }
        if let Err(e) = Self::guest_of(decl) {
            out.push(e);
        }
        out
    }

    fn facts(&self, decl: &SandboxDecl) -> Result<SandboxFacts, String> {
        let guest = Self::guest_of(decl)?;
        let target = TargetSpec {
            side: Side::Guest,
            guest: Some(guest.clone()),
            pins: decl.pins.clone(),
        };
        let image = crate::units::business::vm::api::guest_image_of(
            &self.root,
            self.probe.as_ref(),
            &target,
        )
        .ok_or_else(|| {
            format!(
                "没有这个虚拟根：{}（跑 solomni --provision --ensure --yes 装好之后会列出可选的那些）",
                guest
            )
        })?;
        Ok(SandboxFacts {
            policy: VM_POLICY.to_string(),
            side: Side::Guest,
            guest: Some(guest),
            guest_os: Some(image.os.as_str().to_string()),
            guest_arch: Some(image.arch.as_str().to_string()),
        })
    }
}
