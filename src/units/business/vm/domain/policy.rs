//! 目的：虚拟机策略——把「客机」这条执行环境登记进 `sandbox` 的注册表。
//! 管：`VmPolicy`（自我介绍、声明校验、客机事实、就绪与自带能力、开句柄）、`VmSandbox`（把命令送进 guest）。
//! 不管：guest 怎么起停（`vm::service`）；会话怎么用它（`work`）。
//! 联动：组合根在装配期经 `vm::api::vm_policy` 建实例并登记。

use crate::kernel::ports::HostProbe;
use crate::units::business::sandbox::api::{
    ExecKind, ExecOutcome, ExecRequest, Machine, OpenRequest, PolicyDecl, Readiness, Requirement,
    Sandbox, SandboxDecl, SandboxFacts, SandboxPolicy, VM_POLICY, guest_image_of,
    guest_runtimes_of,
};
use crate::units::business::vm::api::{GuestRun, OutputLimit, Vm, VmInputs, VmTools};
use crate::units::pure::exec_model::api::{Side, TargetSpec};
use crate::units::pure::registry_model::api::{Param, ParamType};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 目的：虚拟机策略：在本机起一台 guest，把这一席的命令送进去跑。
pub struct VmPolicy {
    root: PathBuf,
    probe: Arc<dyn HostProbe + Send + Sync>,
    vm: Arc<dyn Vm + Send + Sync>,
    tools: Arc<dyn Fn() -> VmTools + Send + Sync>,
}

impl VmPolicy {
    /// 目的：按项目根、宿主探针、客机执行面与外部工具位置事实建策略（判镜像与就绪要用它们）。
    pub fn new(
        root: PathBuf,
        probe: Arc<dyn HostProbe + Send + Sync>,
        vm: Arc<dyn Vm>,
        tools: Arc<dyn Fn() -> VmTools + Send + Sync>,
    ) -> VmPolicy {
        VmPolicy {
            root,
            probe,
            vm,
            tools,
        }
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
        let image = guest_image_of(&self.root, self.probe.as_ref(), &target)
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

    fn machine(&self) -> Machine {
        let tools = (self.tools)();
        Machine {
            policy: VM_POLICY.to_string(),
            name: self.decl().name,
            requirements: crate::units::business::vm::api::vm_requirements(&VmInputs {
                tools: &tools,
                probe: self.probe.as_ref(),
            })
            .into_iter()
            .map(requirement_of)
            .collect(),
        }
    }

    fn readiness(&self, target: &TargetSpec) -> Readiness {
        let tools = (self.tools)();
        Readiness {
            requirements: crate::units::business::vm::api::tier_readiness(
                &self.root,
                target,
                &tools,
                self.probe.as_ref(),
            )
            .requirements
            .into_iter()
            .map(requirement_of)
            .collect(),
        }
    }

    fn provided(&self, target: &TargetSpec) -> Vec<String> {
        guest_runtimes_of(&self.root, self.probe.as_ref(), target)
    }

    fn refusal(&self, target: &TargetSpec) -> Option<String> {
        let tools = (self.tools)();
        crate::units::business::vm::api::tier_refusal(
            &self.root,
            target,
            &tools,
            self.probe.as_ref(),
        )
    }

    fn machine_refusal(&self) -> Option<String> {
        let tools = (self.tools)();
        crate::units::business::vm::api::capability_refusal(&VmInputs {
            tools: &tools,
            probe: self.probe.as_ref(),
        })
    }

    fn open(&self, req: &OpenRequest<'_>) -> Result<Arc<dyn Sandbox>, String> {
        Ok(Arc::new(VmSandbox {
            vm: Arc::clone(&self.vm),
            lease: req.lease.to_string(),
        }))
    }
}

/// 目的：客机侧的执行句柄——命令送进这个会话的 guest，本机不跑任何东西。
pub struct VmSandbox {
    vm: Arc<dyn Vm + Send + Sync>,
    lease: String,
}

impl Sandbox for VmSandbox {
    fn precheck(&self, req: &ExecRequest<'_>) -> Option<String> {
        if self.lease.is_empty() {
            return Some(match req.kind {
                ExecKind::Shell { .. } => {
                    "这条虚拟机档会话没有 guest 租约，shell 没有执行。".to_string()
                }
                ExecKind::Module { .. } => {
                    "这条虚拟机档会话没有 guest 租约，工具没有执行。".to_string()
                }
            });
        }
        if req.fence.net.is_local() {
            return Some(
                "local_ports 只在本机档生效：虚拟机档的 guest 回环是它自己的，不映射到宿主端口；这次调用没有执行。"
                    .to_string(),
            );
        }
        None
    }

    fn run(&self, req: &ExecRequest<'_>) -> ExecOutcome {
        match &req.kind {
            ExecKind::Shell { script, .. } => {
                let run = GuestRun {
                    command: script.to_string(),
                    args_json: String::new(),
                    env: Vec::new(),
                    cwd: req.cwd.display().to_string(),
                    timeout_secs: req.timeout.as_secs(),
                    output_limit: OutputLimit::Shell,
                };
                ExecOutcome::plain(self.vm.guest_run_fenced_cancellable(
                    &self.lease,
                    &run,
                    &req.fence,
                    req.cancel,
                ))
            }
            ExecKind::Module {
                module,
                command,
                args_json,
                env,
                ..
            } => {
                // 共享盘不承载执行位：模块声明的相对入口先落进 guest 本地副本，再把首 token 换成它。
                let declared = command.trim_start();
                let dispatched =
                    match crate::units::business::catalog::modules::api::local_entry(declared) {
                        Some(entry) => {
                            match self.vm.stage_exec(
                                &self.lease,
                                module,
                                &req.cwd.join(entry),
                                entry,
                            ) {
                                Ok(staged) => format!("{}{}", staged, &declared[entry.len()..]),
                                Err(why) => {
                                    return ExecOutcome::plain(
                                        crate::kernel::api::types::ToolOutcome::new(false, why),
                                    );
                                }
                            }
                        }
                        None => declared.to_string(),
                    };
                let run = GuestRun {
                    command: dispatched,
                    args_json: args_json.to_string(),
                    env: env.to_vec(),
                    cwd: req.cwd.display().to_string(),
                    timeout_secs: 30,
                    output_limit: OutputLimit::Head,
                };
                ExecOutcome::plain(self.vm.guest_run_fenced(&self.lease, &run, &req.fence))
            }
        }
    }
}

/// 目的：把虚拟机档的一项前置翻成执行环境的通用前置（形状一样，id 从静态串换成字符串）。
fn requirement_of(req: crate::units::business::vm::api::VmRequirement) -> Requirement {
    Requirement {
        id: req.id.to_string(),
        met: req.met,
        detail: req.detail,
        how: req.how,
    }
}
