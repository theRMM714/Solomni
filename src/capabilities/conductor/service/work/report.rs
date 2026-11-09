//! 目的：Conductor 的运行包报告与档位视图——模块声明的能力、包库可用版本、缺失清单、平台与库、计划与权限档。
//! 管：报告数据装配、档位选择、guest 平台与库判定、模块可用性、权限视图。
//! 不管：配置读写（`config.rs`）、建工作（`create.rs`）、调用参数（`model.rs`）。
//! 联动：api 层的视图接口经这些方法取数；builds / packages 的清单在这里被读。
//!   本方法族只碰 probe / packages / modules / root 这四个字段。

use super::*;

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
        let guest_runtimes = self.guest_runtimes_of(spec);
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
            let plan = self.host_plan_for(spec, &roster.modules, pkgs::PinPolicy::Require);
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

    /// 目的：这份规格的 guest 镜像自带哪些运行能力（本机档或判不出来 = 空）。
    pub(crate) fn guest_runtimes_of(&self, spec: &crate::kernel::api::ExecSpec) -> Vec<String> {
        self.guest_image_of(spec)
            .map(|image| image.runtimes)
            .unwrap_or_default()
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

    /// 目的：这次会话的 guest 镜像（本机档或选定虚拟根判不出来 = None）——平台名与 guest OS 都从它派生。
    /// 约束：虚拟根名先经 `resolve_guest` 解析成那一格目录，再按布局判定；两者任一失败都给 None。
    fn guest_image_of(
        &self,
        spec: &crate::kernel::api::ExecSpec,
    ) -> Option<crate::capabilities::vm::api::GuestImage> {
        if spec.tier != crate::kernel::api::Tier::Vm {
            return None;
        }
        let name = spec.guest.as_deref()?;
        let dir = crate::capabilities::vm::api::resolve_guest(&self.root, name).ok()?;
        crate::capabilities::vm::api::classify_base(&dir, self.probe.as_ref()).ok()
    }

    /// 目的：这次虚拟机档会话的 guest 平台名（os-arch）；本机档或判不出来给空串。
    fn guest_platform_of(&self, spec: &crate::kernel::api::ExecSpec) -> String {
        self.guest_image_of(spec)
            .map(|image| crate::capabilities::vm::api::guest_platform(image.os, image.arch))
            .unwrap_or_default()
    }

    /// 目的：这场会话的路径翻译器（虚拟机档 guest↔宿主；本机档 None = 恒等）。
    /// 约束：它只把模型给的写法译成宿主路径**再送寻址**；安全判断仍在 `workspace::Sandbox::resolve`。
    pub(crate) fn path_codec_of(
        &self,
        spec: &crate::kernel::api::ExecSpec,
    ) -> Option<std::sync::Arc<dyn crate::kernel::ports::PathCodec>> {
        let guest = self.guest_os_of(spec)?;
        Some(std::sync::Arc::new(
            crate::capabilities::vm::api::PathBridge::for_session(
                std::env::consts::OS,
                Some(guest.dialect()),
            ),
        ))
    }

    /// 目的：这次虚拟机档会话的 guest OS（本机档或判不出来 = None）——工作环境块按它选 guest 档。
    pub(crate) fn guest_os_of(
        &self,
        spec: &crate::kernel::api::ExecSpec,
    ) -> Option<crate::capabilities::vm::api::GuestOs> {
        self.guest_image_of(spec).map(|image| image.os)
    }

    /// 目的：这次会话该用的包库——按 guest 平台筛过（分平台包只留与 guest 匹配的那份）。
    fn library_for(&self, spec: &crate::kernel::api::ExecSpec) -> pkgs::Library {
        pkgs::library_for_guest(&self.packages.library(), &self.guest_platform_of(spec))
    }

    /// 目的：本机档的包计划：按宿主目标选包、产出 PATH/只读根；会话用 Require，无会话面用 First。
    pub(crate) fn host_plan_for(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        modules: &[Module],
        policy: pkgs::PinPolicy,
    ) -> pkgs::PackagePlan {
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let has_exe = |name: &str| self.probe.has_exe(name);
        let bin_ok = |path: &std::path::Path| self.probe.is_dir(path);
        let base = self.packages.base();
        let facts = pkgs::Facts {
            has_exe: &has_exe,
            bin_ok: &bin_ok,
            current_path: &current_path,
            base: &base.active,
            host_fallback: &base.fallback,
        };
        pkgs::plan_host(spec, modules, &self.packages.library(), policy, &facts)
    }

    /// 目的：按一份执行规格与模块清单派生计划：入口判据先于包选型（与旧 `plan` 的错误顺序一致）。
    pub(crate) fn plan_for_spec(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        modules: &[Module],
        missing: &[crate::capabilities::vm::api::MissingEntry],
        guest_root: Option<&str>,
    ) -> Result<crate::capabilities::vm::api::ExecPlan, String> {
        // 一次把话说全：入口判据在前，包选型问题在后（与旧的单一计划函数同一顺序）。
        let entry_diags =
            crate::capabilities::vm::api::plan(spec, Vec::new(), missing, self.network_allowed())
                .err()
                .unwrap_or_default();
        let package_plan = if spec.tier == crate::kernel::api::Tier::Vm {
            pkgs::plan_guest(
                spec,
                modules,
                &self.library_for(spec),
                &self.guest_runtimes_of(spec),
            )
        } else {
            pkgs::PackagePlan::default()
        };
        let mut parts: Vec<String> = Vec::new();
        if !entry_diags.is_empty() {
            parts.push(crate::capabilities::vm::api::diagnose_text_for(
                &entry_diags,
                guest_root,
            ));
        }
        if !package_plan.problems.is_empty() {
            parts.push(pkgs::diagnose_text(&package_plan.problems));
        }
        if !parts.is_empty() {
            return Err(parts.join("；"));
        }
        crate::capabilities::vm::api::plan(spec, package_plan.packages, &[], self.network_allowed())
            .map_err(|diags| crate::capabilities::vm::api::diagnose_text_for(&diags, guest_root))
    }

    pub(crate) fn plan_for_meta_with(
        &self,
        meta: &SessionMeta,
        roster: &crate::capabilities::modules::api::Roster,
    ) -> Result<crate::capabilities::vm::api::ExecPlan, String> {
        let mut ids: Vec<&str> = meta
            .agents
            .iter()
            .flat_map(|a| a.modules.iter().map(String::as_str))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| ids.iter().any(|id| *id == m.manifest.id))
            .cloned()
            .collect();
        let guest = self.guest_os_of(&meta.exec).map(|g| g.as_str());
        let target_os = guest.unwrap_or(std::env::consts::OS);
        let missing: Vec<crate::capabilities::vm::api::MissingEntry> =
            crate::capabilities::vm::api::missing_entries_for(&session_modules, target_os, &|p| {
                self.probe.is_executable(p)
                    && (target_os == std::env::consts::OS
                        || crate::capabilities::vm::api::runs_on_target(p, target_os))
            });
        self.plan_for_spec(
            &meta.exec,
            &session_modules,
            &missing,
            meta.exec.guest.as_deref(),
        )
    }

    /// 目的：本档位下不能执行工具的模块（模块 id → 缺的能力名）：建会话与重建时收口给工具环境。
    pub(crate) fn unavailable_modules(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        modules: &[Module],
    ) -> BTreeMap<String, Vec<String>> {
        if spec.tier == crate::kernel::api::Tier::Vm {
            pkgs::plan_guest(
                spec,
                modules,
                &self.library_for(spec),
                &self.guest_runtimes_of(spec),
            )
            .unusable
        } else {
            self.host_plan_for(spec, modules, pkgs::PinPolicy::Require)
                .unusable
        }
    }

    /// 目的：人直接跑模块工具（无会话）时不能执行工具的模块——它跑在宿主，按**宿主**目标判。
    /// 约束：与成员循环同一把尺子（`packages::api::plan_host` 的 `unusable`）；没有定版入口，多版本取稳定序第一个。
    pub fn unavailable_modules_now(&self) -> BTreeMap<String, Vec<String>> {
        let roster = self.modules.roster();
        self.host_plan_for(
            &crate::kernel::api::ExecSpec::default(),
            &roster.modules,
            pkgs::PinPolicy::First,
        )
        .unusable
    }

    /// 目的：把一条会话的生效权限变成配置视图（含四档可选值与收窄提示）。
    pub(crate) fn permission_view(
        &self,
        meta: &SessionMeta,
    ) -> Result<crate::capabilities::conductor::api::PermissionView, String> {
        let effective = self.permissions_for(meta)?;
        let parent_profile = match &meta.parent {
            Some(parent) => Some(self.permissions_for(&self.history.meta(parent)?)?.profile),
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
