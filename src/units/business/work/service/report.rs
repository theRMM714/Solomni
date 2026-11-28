//! 目的：Core 的运行包报告与执行环境视图——模块声明的能力、包库可用版本、缺失清单、会话执行计划的入口、权限档。
//! 管：报告数据装配、执行环境选择、会话清单到执行计划的入口（`plan_for_meta_with`）、权限视图。
//! 不管：配置读写（`config.rs`）、建工作（`create.rs`）、调用参数（`model.rs`）；
//!   平台 / 库判定与包选型在 `vm::api` 的 `plan` 模块（这里只消费它）。
//! 联动：api 层的视图接口经这些方法取数；builds / packages 的清单在这里被读。
//!   本方法族只碰 probe / packages / modules / root / registry 这几个字段。

use super::*;

impl Work {
    /// 目的：运行能力报告：模块声明的能力、包库里的可用版本、缺失清单与 guest 侧诊断。
    ///  「清单即事实」：每次调用重扫模块清单与包库；host 侧按宿主目标选包，missing 只保留这一侧确实用不了的能力。
    pub fn runtime_report(
        &self,
        probe: &dyn HostProbe,
        sandbox: &crate::units::business::sandbox::api::SandboxDecl,
    ) -> RuntimeReport {
        self.runtime_report_for(probe, sandbox)
    }

    /// 目的：这份执行规格下的运行能力报告——会话配置用（选定虚拟根在场时把 guest 自带的运行能力并进来）。
    pub(crate) fn runtime_report_for(
        &self,
        probe: &dyn HostProbe,
        sandbox: &crate::units::business::sandbox::api::SandboxDecl,
    ) -> RuntimeReport {
        let roster = self.modules.roster();
        let lib = self.packages.library();
        let facts = self.policies.facts(sandbox).unwrap_or_default();
        let spec = &sandbox.target(&facts);
        let guest_runtimes = self.policies.provided(sandbox, spec);
        let vm = spec.side.on_guest();
        let (diagnoses, mut missing, available) = if vm {
            let diagnoses = pkgs::diagnoses(&roster.modules, &lib, spec, &guest_runtimes)
                .into_iter()
                .map(RuntimeDiagnosis::Package)
                .collect();
            // 「缺什么」按**这份规格**算：guest 自带的运行能力不算缺（与 plan / unavailable 同一把尺子）。
            let mut missing = pkgs::absent(&roster.modules, &lib);
            for caps in missing.values_mut() {
                caps.retain(|c| !guest_runtimes.iter().any(|g| g == c));
            }
            (diagnoses, missing, lib.capability_versions())
        } else {
            let plan = crate::units::business::catalog::packages::api::host_plan_for(
                &self.packages.library(),
                &self.packages.base(),
                probe,
                spec,
                &roster.modules,
                pkgs::PinPolicy::Require,
            );
            let diagnoses = plan
                .problems
                .iter()
                .cloned()
                .map(RuntimeDiagnosis::Package)
                .collect();
            let available = lib.for_target(&pkgs::Target::host()).capability_versions();
            (diagnoses, plan.unusable, available)
        };
        missing.retain(|_, caps| !caps.is_empty());
        let readiness = self.policies.readiness(sandbox, spec);
        RuntimeReport {
            side: spec.side.as_str().to_string(),
            declared: declared(&roster.modules),
            available,
            missing,
            diagnoses,
            sandbox_ready: readiness.ready(),
            sandbox_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
            rejected: roster.rejected.clone(),
            rejected_packages: lib.rejected.clone(),
        }
    }

    /// 目的：新建工作的**执行环境选择**（默认策略 + guest 侧可用性与逐项前置）：与「开始」的校验同源。
    ///  为什么单独一条读面：创建向导还没有 sid，拿不到会话配置视图；而设置里的默认策略与
    ///  guest 侧的承载探针是**与某条会话无关**的事实。
    pub fn sandbox_choices(&self, registry: &dyn Registry) -> SandboxChoices {
        // 设置里写的策略可能已不在注册表里：以注册表为准回落，不让界面拿到点不动的默认值。
        let configured = registry.app().default_sandbox.policy.clone();
        let default = if self.policies.of(&configured).is_some() {
            configured
        } else {
            self.policies.default_id()
        };
        SandboxChoices {
            default,
            policies: self.policy_views(),
            // 虚拟根清单是**事实**：读不到就空表（界面据此说"没有可选虚拟根"，不编造）。
            guests: self.provision.guest_bases().unwrap_or_default(),
            // 包库可用能力同样是事实：界面按它 + 虚拟根自带能力预判缺口，判据仍以后端为准。
            capabilities: self.packages.library().capability_versions(),
        }
    }

    /// 目的：某个执行环境策略缺的依赖怎么补（逐条下载清单）；`ids` 为空 = 现在缺的那些。
    /// 约束：补齐能力来自策略自己（`SandboxPolicy::fixer`）：没登记策略 / 策略没有补齐器都如实拒绝。
    pub fn fix_plan(
        &self,
        policy: &str,
        ids: &[String],
    ) -> Result<Vec<crate::units::pure::exec_model::api::FixStep>, String> {
        self.fixer_of(policy)?.plan(ids)
    }

    /// 目的：按策略起补齐作业（`ids` 为空 = 现在缺的那些）。
    pub fn fix_start(&self, policy: &str, ids: Vec<String>) -> Result<(), String> {
        self.fixer_of(policy)?.start(ids)
    }

    /// 目的：取某个策略登记的补齐器；没有就如实说明是"没这个策略"还是"这个策略不用装东西"。
    fn fixer_of(
        &self,
        policy: &str,
    ) -> Result<std::sync::Arc<dyn crate::units::pure::exec_model::api::Fixer>, String> {
        if self.policies.of(policy).is_none() {
            return Err(format!("没有登记这个执行环境策略：{}", policy));
        }
        self.policies.fixer(policy).ok_or_else(|| {
            format!(
                "执行环境策略「{}」不需要补齐依赖（它没有登记补齐器）",
                policy
            )
        })
    }

    /// 目的：把注册表里的策略逐个翻成呈现层的一项：自我介绍、逐项前置、能不能选、能不能补。
    /// 约束：这一层不产生判断——可用性来自策略自己的 `machine`，补齐能力来自它自己的 `fixer`；
    ///   取值域（`PolicyDecl::choices`）在这里解析成实际清单（认不出的域留空 = 手填）。
    pub(crate) fn policy_views(&self) -> Vec<SandboxChoice> {
        // 虚拟根域：只列装齐了的（没装齐的选了也起不来；缺什么另由 provision 面板说）。
        let guests: Vec<String> = self
            .provision
            .guest_bases()
            .unwrap_or_default()
            .into_iter()
            .filter(|g| g.ready)
            .map(|g| g.guest)
            .collect();
        let mut domains: BTreeMap<String, Vec<String>> = BTreeMap::new();
        domains.insert("guests".to_string(), guests);
        self.policies
            .catalog()
            .into_iter()
            .map(|view| {
                let mut decl = view.decl;
                let declared = decl.choices.clone();
                if let Some(params) = decl.params.as_mut() {
                    for (name, domain) in &declared {
                        if let (Some(values), Some(param)) =
                            (domains.get(domain), params.get_mut(name))
                        {
                            param.values = Some(values.clone());
                        }
                    }
                }
                // 能补 = 这个策略自己登记了补齐器（它列清单、它起作业）；缺项里哪些能装、哪些只能改本机,
                // 由补齐器的清单与每条 requirement 的 fix 如实说明，界面不预判。
                let fixable = self.policies.fixer(&decl.id).is_some();
                let available = view.machine.ready();
                let refusal = self.policies.machine_refusal(&decl.id).unwrap_or_default();
                SandboxChoice {
                    decl,
                    needs: view.machine.requirements,
                    available,
                    refusal,
                    fixable,
                }
            })
            .collect()
    }

    /// 目的：这份会话的执行计划（清单先按会话用到的模块收窄，再交给 `catalog::packages::api::plan_for_spec` 派生）。
    /// 约束：模块集合只取清单里、且该会话某个 agent 声明过的；缺什么按 guest 目标判（与计划同一把尺子）。
    pub(crate) fn plan_for_meta_with(
        &self,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        meta: &SessionMeta,
        roster: &crate::units::business::catalog::modules::api::Roster,
    ) -> Result<crate::units::business::catalog::packages::api::ExecPlan, String> {
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
        let guest =
            crate::units::business::sandbox::api::guest_os_of(&self.root, probe, &meta.target())
                .map(|g| g.as_str());
        let target_os = guest.unwrap_or(std::env::consts::OS);
        let missing: Vec<crate::units::business::catalog::modules::api::MissingEntry> =
            crate::units::business::catalog::modules::api::missing_entries_for(
                &session_modules,
                target_os,
                &|p| {
                    probe.is_executable(p)
                        && (target_os == std::env::consts::OS
                            || crate::units::business::catalog::modules::api::runs_on_target(
                                p, target_os,
                            ))
                },
            );
        crate::units::business::catalog::packages::api::plan_for_spec(
            &self.root,
            probe,
            &self.packages.library(),
            &meta.target(),
            &session_modules,
            &missing,
            meta.facts.guest.as_deref(),
            registry.app().sandbox.policy.network,
        )
    }

    /// 目的：人直接跑模块工具（无会话）时的宿主包计划——按**宿主**目标选包，没有定版入口，多版本取稳定序第一个。
    /// 约束：与成员循环同一把尺子（`catalog::packages::api::host_plan_for`）；api 层只经这里取无会话的包计划，自己碰不到包库与探针。
    pub(crate) fn host_plan_now(
        &self,
        probe: &dyn HostProbe,
        modules: &[Module],
    ) -> pkgs::PackagePlan {
        crate::units::business::catalog::packages::api::host_plan_for(
            &self.packages.library(),
            &self.packages.base(),
            probe,
            &crate::units::pure::exec_model::api::TargetSpec::default(),
            modules,
            pkgs::PinPolicy::First,
        )
    }

    /// 目的：人直接跑模块工具（无会话）时不能执行工具的模块——它跑在宿主，按**宿主**目标判。
    /// 约束：与成员循环同一把尺子（`packages::api::plan_host` 的 `unusable`）；没有定版入口，多版本取稳定序第一个。
    pub fn unavailable_modules_now(&self, probe: &dyn HostProbe) -> BTreeMap<String, Vec<String>> {
        let roster = self.modules.roster();
        self.host_plan_now(probe, &roster.modules).unusable
    }

    /// 目的：把一条会话的生效授权变成配置视图（网络档位 + 共享区归属 + 可选档位）。
    pub(crate) fn permission_view(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        meta: &SessionMeta,
    ) -> Result<PermissionView, String> {
        let effective = turn.permissions_for(registry, meta)?;
        let parent_profile = match &meta.parent {
            Some(parent) => Some(
                turn.permissions_for(registry, &self.history.meta(parent)?)?
                    .profile,
            ),
            None => None,
        };
        let scope = turn.shared_scope_for(registry, meta)?;
        Ok(PermissionView {
            profile: effective.profile,
            label: effective.profile.label().to_string(),
            summary: effective.profile.summary().to_string(),
            shared: PermissionSharedView {
                read: scope.read.clone(),
                write: scope.write.clone(),
                deny: scope.deny.clone(),
            },
            narrowed: meta.parent.is_some() && meta.permission.narrowing.is_some(),
            parent_profile,
            profiles: crate::units::pure::permission::api::ProfileId::all()
                .into_iter()
                .map(|p| p.spec())
                .collect(),
        })
    }
}
