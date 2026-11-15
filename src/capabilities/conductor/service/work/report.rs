//! 目的：Conductor 的运行包报告与档位视图——模块声明的能力、包库可用版本、缺失清单、档位与权限档。
//! 管：报告数据装配、档位选择（默认档与虚拟机档承载探针）、会话的权限视图。
//! 不管：配置读写（`config.rs`）、建工作（`create.rs`）、调用参数（`model.rs`）、
//!   guest 平台与库、包计划与模块可用性判定（归 turn 的会话本体，见 `turn/service/plan.rs`）。
//! 联动：api 层的视图接口经这些方法取数；builds / packages 的清单在这里被读。
//!   本方法族只碰 probe / packages / modules / turn 这几个字段。

use super::*;
use std::collections::BTreeMap;

impl Conductor {
    /// 目的：运行能力报告：模块声明的能力、包库里的可用版本、缺失清单与虚拟机档诊断。
    ///  「清单即事实」：每次调用重扫模块清单与包库；本机档按宿主目标选包，missing 只保留本档位确实用不了的能力。
    pub fn runtime_report(&self, tier: crate::kernel::api::Tier) -> RuntimeReport {
        self.runtime_report_for(&crate::kernel::api::ExecSpec {
            tier,
            ..crate::kernel::api::ExecSpec::default()
        })
    }

    /// 目的：这份执行规格下的运行能力报告——会话配置用（选定虚拟根在场时把 guest 自带的运行能力并进来）。
    pub(crate) fn runtime_report_for(&self, spec: &crate::kernel::api::ExecSpec) -> RuntimeReport {
        let roster = self.modules.roster();
        let lib = self.packages.library();
        let guest_runtimes = self.turn.guest_runtimes_of(spec, self.probe());
        let vm = spec.tier == crate::kernel::api::Tier::Vm;
        let (diagnoses, mut missing, available) = if vm {
            let diagnoses = pkgs::diagnoses(&roster.modules, &lib, spec, &guest_runtimes)
                .into_iter()
                .map(crate::capabilities::conductor::api::RuntimeDiagnosis::Package)
                .collect();
            // 「缺什么」按**这份规格**算：guest 自带的运行能力不算缺（与 plan / unavailable 同一把尺子）。
            let mut missing = pkgs::absent(&roster.modules, &lib);
            for caps in missing.values_mut() {
                caps.retain(|c| !guest_runtimes.iter().any(|g| g == c));
            }
            (diagnoses, missing, lib.capability_versions())
        } else {
            let plan = self.turn.host_plan_for(
                spec,
                &roster.modules,
                pkgs::PinPolicy::Require,
                self.probe(),
            );
            let diagnoses = plan
                .problems
                .iter()
                .cloned()
                .map(crate::capabilities::conductor::api::RuntimeDiagnosis::Package)
                .collect();
            let available = lib.for_target(&pkgs::Target::host()).capability_versions();
            (diagnoses, plan.unusable, available)
        };
        missing.retain(|_, caps| !caps.is_empty());
        let qemu = self.qemu_path();
        let share_tool = self.share_tool_path();
        let readiness = crate::capabilities::vm::api::tier_readiness(
            &self.root,
            spec,
            qemu.as_deref(),
            share_tool.as_deref(),
            self.probe.as_ref(),
        );
        RuntimeReport {
            tier: spec.tier.as_str().to_string(),
            declared: declared(&roster.modules),
            available,
            missing,
            diagnoses,
            tier_ready: readiness.ready(),
            tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
            rejected: roster.rejected.clone(),
            rejected_packages: lib.rejected.clone(),
        }
    }

    /// 目的：新建工作的**档位选择**（默认档 + 虚拟机档可用性与逐项前置）：与「开始」的校验同源。
    ///  为什么单独一条读面：创建向导还没有 sid，拿不到会话配置视图；而设置里的默认档与
    ///  虚拟机档的承载探针是**与某条会话无关**的事实。
    pub fn tier_choices(&self) -> TierChoices {
        let default = self.registry.app().tier;
        // 这是**与某条会话无关的机器能力**：虚拟根属于会话选型，不在这里判（创建时另行收集）。
        let qemu = self.qemu_path();
        let share_tool = self.share_tool_path();
        let inputs = crate::capabilities::vm::api::VmInputs {
            qemu: qemu.as_deref(),
            share_tool: share_tool.as_deref(),
            probe: self.probe.as_ref(),
        };
        let cap = crate::capabilities::vm::api::vm_capability(&inputs);
        let available = cap.ready();
        TierChoices {
            default: default.as_str().to_string(),
            vm_available: available,
            vm_unavailable_reason: crate::capabilities::vm::api::capability_refusal(&inputs)
                .unwrap_or_default(),
            vm_requirements: cap.requirements,
            // 虚拟根清单是**事实**：读不到就空表（界面据此说"没有可选虚拟根"，不编造）。
            guests: self.guest_bases().unwrap_or_default(),
            // 包库可用能力同样是事实：界面按它 + 虚拟根自带能力预判缺口，判据仍以后端为准。
            available: self.packages.library().capability_versions(),
        }
    }

    /// 目的：人直接跑模块工具（无会话）时不能执行工具的模块——它跑在宿主，按**宿主**目标判。
    /// 约束：与成员循环同一把尺子（`packages::api::plan_host` 的 `unusable`）；没有定版入口，多版本取稳定序第一个。
    pub fn unavailable_modules_now(&self) -> BTreeMap<String, Vec<String>> {
        let roster = self.modules.roster();
        self.turn
            .host_plan_for(
                &crate::kernel::api::ExecSpec::default(),
                &roster.modules,
                pkgs::PinPolicy::First,
                self.probe(),
            )
            .unusable
    }

    /// 目的：把一条会话的生效权限变成配置视图（含四档可选值与收窄提示）。
    pub(crate) fn permission_view(
        &self,
        meta: &SessionMeta,
    ) -> Result<crate::capabilities::conductor::api::PermissionView, String> {
        let effective = self.turn.permissions_for(meta, self.registry())?;
        let parent_profile = match &meta.parent {
            Some(parent) => Some(
                self.turn
                    .permissions_for(&self.history.meta(parent)?, self.registry())?
                    .profile,
            ),
            None => None,
        };
        let warning = if effective.is_no_access() {
            "当前根档位与本次收窄没有交集：这个会话目前没有可用权限。".to_string()
        } else {
            String::new()
        };
        Ok(crate::capabilities::conductor::api::PermissionView {
            profile: effective.profile,
            label: effective.profile.label().to_string(),
            summary: effective.profile.summary().to_string(),
            reach: effective.reach.to_vec(),
            decisions: crate::capabilities::conductor::api::PermissionDecisionsView {
                read: effective.decisions.read,
                write: effective.decisions.write,
                exec_sandbox: effective.decisions.exec_sandbox,
                exec_external: effective.decisions.exec_external,
            },
            shared: crate::capabilities::conductor::api::PermissionSharedView {
                read: effective.shared_read.clone(),
                write: effective.shared_write.clone(),
                deny: effective.deny.clone(),
            },
            tools: effective.tools.clone(),
            narrowed: meta.parent.is_some() && meta.sandbox.narrowing.is_some(),
            parent_profile,
            warning,
            profiles: crate::capabilities::permission::api::ProfileId::all()
                .into_iter()
                .map(|p| p.spec())
                .collect(),
        })
    }
}
