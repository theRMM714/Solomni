//! **工作与会话配置**：运行包报告、会话配置的读改（session_config / edit_session）、建工作与上传、文件视图。
//!
//! 工作名唯一化与会话视图（在世会话 × 历史的并集）也在这里。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate)（或 pub）供兄弟族与 conductor/api.rs 调用。

use super::guest_job::{EditGuest, GuestBoot};
use super::*;

/// 目的：一次创建的准备结果——"已经建完"或"待提交"（后者等调用方线程把 guest 起起来）。
pub(crate) enum PreparedWork {
    /// 已经建完（代理形态：没有 guest 可等）。
    Ready(WorkOpened),
    /// 待提交：核心已占名并备好工作区与计划，慢的那一段由调用方跑。
    Pending {
        meta: Box<SessionMeta>,
        mode: WorkMode,
        metas: Vec<AgentMeta>,
        task: Option<String>,
        delegate: bool,
        name: String,
        agent_names: Vec<String>,
        sandboxes: crate::capabilities::workspace::api::Sandboxes,
        /// 失败时要撤掉的工作区空目录（只对顶层会话给：子会话与父共用目录）。
        discard: Option<(String, Vec<String>)>,
    },
}

/// 目的：一次改配的准备结果——旧 meta（撤围栏用）与新的（落盘用）。
pub(crate) struct PreparedEdit {
    pub(crate) sid: String,
    pub(crate) old_meta: SessionMeta,
    pub(crate) new_meta: SessionMeta,
    /// 目的：主会话改档位时要同步的子树（旧值用于撤围栏，新值用于落盘）。
    pub(crate) old_children: Vec<SessionMeta>,
    pub(crate) new_children: Vec<SessionMeta>,
}
impl Conductor {
    /// 运行能力报告：模块声明的能力、包库里的可用版本、缺失清单与虚拟机档诊断。
    /// 「清单即事实」：每次调用重扫模块清单与包库；本机档不装载运行包，missing 只作事实呈现。
    pub fn runtime_report(&self, tier: crate::kernel::api::Tier) -> RuntimeReport {
        self.runtime_report_for(&crate::kernel::api::ExecSpec {
            tier,
            ..crate::kernel::api::ExecSpec::default()
        })
    }

    /// 目的：这份执行规格下的运行能力报告——会话配置用（选定虚拟根在场时把 guest 自带的运行能力并进来）。
    pub(crate) fn runtime_report_for(&self, spec: &crate::kernel::api::ExecSpec) -> RuntimeReport {
        let roster = self.modules.roster();
        let lib = self.vm.library();
        let guest_runtimes = self.guest_runtimes_of(spec);
        let vm = spec.tier == crate::kernel::api::Tier::Vm;
        let diagnoses = if vm {
            crate::capabilities::vm::api::vm_diagnoses(&roster.modules, &lib, spec, &guest_runtimes)
        } else {
            Vec::new()
        };
        // 「缺什么」按**这份规格**算：guest 自带的运行能力不算缺（与 plan / unavailable 同一把尺子）。
        let mut missing = crate::capabilities::vm::api::absent(&roster.modules, &lib);
        if vm {
            for caps in missing.values_mut() {
                caps.retain(|c| !guest_runtimes.iter().any(|g| g == c));
            }
            missing.retain(|_, caps| !caps.is_empty());
        }
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
            declared: crate::capabilities::vm::api::declared(&roster.modules),
            available: lib.capability_versions(),
            missing,
            diagnoses,
            tier_ready: readiness.ready(),
            tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
            rejected: roster.rejected.clone(),
            rejected_packages: lib.rejected.clone(),
        }
    }

    /// 目的：这份规格的 guest 镜像自带哪些运行能力（本机档或判不出来 = 空）。
    fn guest_runtimes_of(&self, spec: &crate::kernel::api::ExecSpec) -> Vec<String> {
        self.guest_image_of(spec)
            .map(|image| image.runtimes)
            .unwrap_or_default()
    }

    /// 新建工作的**档位选择**（默认档 + 虚拟机档可用性与逐项前置）：与「开始」的校验同源。
    /// 为什么单独一条读面：创建向导还没有 sid，拿不到会话配置视图；而设置里的默认档与
    /// 虚拟机档的承载探针是**与某条会话无关**的事实。
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
            available: self.vm.library().capability_versions(),
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
    fn library_for(
        &self,
        spec: &crate::kernel::api::ExecSpec,
    ) -> crate::capabilities::vm::api::Library {
        crate::capabilities::vm::api::library_for_guest(
            &self.vm.library(),
            &self.guest_platform_of(spec),
        )
    }

    fn plan_for_meta_with(
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
        crate::capabilities::vm::api::plan(
            &meta.exec,
            &session_modules,
            &self.library_for(&meta.exec),
            &self.guest_runtimes_of(&meta.exec),
            &missing,
        )
        .map_err(|diags| crate::capabilities::vm::api::diagnose_text_for(&diags, guest))
    }

    /// 本档位下不能执行工具的模块（模块 id → 缺的能力名）：建会话与重建时收口给工具环境。
    pub(crate) fn unavailable_modules(
        &self,
        spec: &crate::kernel::api::ExecSpec,
        modules: &[Module],
    ) -> BTreeMap<String, Vec<String>> {
        let lib = self.library_for(spec);
        let guest_runtimes = self.guest_runtimes_of(spec);
        crate::capabilities::vm::api::unavailable(spec, modules, &lib, &guest_runtimes)
    }

    /// 目的：**当前设置档位**下不能执行工具的模块（人直接跑与动作目录共用）：本机档不装载运行包，一律可用。
    /// 约束：与成员循环同一把尺子（`vm::api::unavailable`，档位感知）——不用档位无关的 `absent`，
    ///   否则本机档会因"没装运行包"误拒本机解释器就能跑的工具。
    pub fn unavailable_modules_now(&self) -> BTreeMap<String, Vec<String>> {
        let spec = crate::kernel::api::ExecSpec {
            tier: self.registry.app().tier,
            ..crate::kernel::api::ExecSpec::default()
        };
        let roster = self.modules.roster();
        self.unavailable_modules(&spec, &roster.modules)
    }

    /// 配置视图：把「能改什么、现在是什么、缺什么」如实给出（每次读取都重扫模块清单与包库）。
    pub fn session_config(&self, sid: &str) -> Result<SessionConfig, String> {
        let (meta, events) = self.history_open(sid)?;
        let tier = meta.exec.tier;
        // 机器能力（与某条会话无关）：界面据此决定虚拟机档能不能**选**；虚拟根是会话级，另行校验。
        let qemu = self.qemu_path();
        let share_tool = self.share_tool_path();
        let inputs = crate::capabilities::vm::api::VmInputs {
            qemu: qemu.as_deref(),
            share_tool: share_tool.as_deref(),
            probe: self.probe.as_ref(),
        };
        let cap = crate::capabilities::vm::api::vm_capability(&inputs);
        let vm_available = cap.ready();
        let vm_reason =
            crate::capabilities::vm::api::capability_refusal(&inputs).unwrap_or_default();
        let vm_requirements = cap.requirements;
        // 会话级判据（机器能力 + 这条会话选的虚拟根）只算一次：两处读同一个结论，不各算一遍。
        let readiness = crate::capabilities::vm::api::tier_readiness(
            &self.root,
            &meta.exec,
            qemu.as_deref(),
            share_tool.as_deref(),
            self.probe.as_ref(),
        );
        Ok(SessionConfig {
            sid: meta.name.clone(),
            mode: meta.mode.clone(),
            started: session_started(&events),
            agents: meta
                .agents
                .iter()
                .map(|a| ConfigAgent {
                    name: a.name.clone(),
                    modules: a.modules.clone(),
                    model: a.model.clone().unwrap_or_default(),
                    // 视图要把现值给出去；编辑回包缺了它 = 保留现值（见 SessionEdit 的字段说明）。
                    permissions: Some(a.permissions.clone()),
                })
                .collect(),
            tier: tier.as_str().to_string(),
            tier_locked: meta.parent.is_some(),
            guest: meta.exec.guest.clone(),
            guests: self.guest_bases()?,
            net: meta.exec.net,
            pins: meta.exec.pins.clone(),
            runtime: self.runtime_report_for(&meta.exec),
            runtimes_dir: crate::kernel::api::slash(&self.vm.runtimes_dir()),
            tier_ready: readiness.ready(),
            tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
            // 虚拟机档能不能选**与当前档位无关**：本机档会话也要如实告诉用户 vm 现在不可用（界面据此禁用）。
            // 逐项清单一起给出：界面照抄"缺哪几项、每项怎么补"，不自己编话。
            vm_available,
            vm_unavailable_reason: vm_reason,
            vm_requirements,
        })
    }

    /// 目的：测试用的同步改配入口（准备 → 停/起 guest → 提交，全在当前线程）。
    /// 约束：生产走句柄的两段（`prepare_edit` / `commit_edit`）。
    #[cfg(test)]
    pub fn edit_session(&mut self, sid: &str, edit: SessionEdit) -> Result<(), String> {
        let (prepared, job) = self.prepare_edit(sid, edit)?;
        if let Some(job) = job {
            // 失败什么都不改：meta 还没落盘，旧会话原样还在。
            job.run()?;
        }
        self.commit_edit(prepared)
    }

    /// 目的：改配的第一段（核心，毫秒级）——全部校验 + 出计划，并给出"要等 guest"的那一段（可能没有）。
    /// 返回：准备结果 + 可选的那段活（切入 / 留在虚拟机档要起或重建；切回本机档要停旧的）。
    pub(crate) fn prepare_edit(
        &mut self,
        sid: &str,
        edit: SessionEdit,
    ) -> Result<(PreparedEdit, Option<EditGuest>), String> {
        // 改配要重建整棵子树的档位/guest 关联：任一在跑都拒绝（先停再改）。
        for s in self.subtree_of(sid) {
            if self.running.contains(&s) {
                return Err(Self::running_refusal(&s));
            }
        }
        let (meta, events) = self.history_open(sid)?;
        match meta.mode.as_str() {
            "single" | "collab" => {}
            // 代理会话没有名单可编辑（决定权整块交给核心）：如实说明，不报"未知形态"。
            "proxy" => return Err("代理会话没有可编辑的名单：它不是一个 agent 工作".to_string()),
            other => {
                return Err(format!(
                    "未知会话形态：{}（只认 single / collab / proxy）",
                    other
                ))
            }
        }
        // 共享 guest 的子会话也可以编辑：写回该 agent 的唯一配置，保存后重新加入同一台 guest。
        // 旧 guest / 旧树的装配在下面与编辑后的树比对，只有装配真的变了才重建。
        let owner = self.config_owner_of(&meta)?;
        let old_scope = self.guest_scope(&meta)?;
        let old_tree = match &old_scope {
            Some((_, root)) => Some(self.tree_meta(root, None)?),
            None => None,
        };
        if session_started(&events) {
            let old: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
            let new: Vec<String> = edit.agents.iter().map(|a| a.name.clone()).collect();
            if old != new {
                return Err(
                    "这轮会话已经开过：agent 名单与形态冻结（要换人请新建会话）".to_string()
                );
            }
        }
        // 校验：模块与模型真实存在；同一模块不得同属两个 agent（沙箱与发言归属会歧义）。
        let roster = self.scan();
        let reserved = self.reserved_names();
        let mut seen: Vec<String> = Vec::new();
        let mut metas: Vec<AgentMeta> = Vec::new();
        // 旧权限按名字留档：这次编辑没给某个 agent 的 permissions（None）= 保留它的现值。
        let old_permissions: std::collections::BTreeMap<
            String,
            crate::capabilities::permission::api::PermissionsOverride,
        > = meta
            .agents
            .iter()
            .map(|x| (x.name.clone(), x.permissions.clone()))
            .collect();
        for a in &edit.agents {
            crate::capabilities::registry::api::validate_name(&a.name)?;
            crate::capabilities::registry::api::check_reserved(&a.name, &reserved)?;
            if let Some(p) = &a.permissions {
                crate::capabilities::permission::api::validate_override(p)?;
            }
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
                if seen.iter().any(|x| x == id) {
                    return Err(format!(
                        "模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent",
                        id
                    ));
                }
                seen.push(id.clone());
            }
            if !a.model.is_empty() {
                if !self.registry.has_model(&a.model) {
                    return Err(format!("无此模型：{}", a.model));
                }
                self.registry.resolve(&a.model)?;
            }
            metas.push(AgentMeta {
                name: a.name.clone(),
                transient: !self.registry.has_agent(&a.name),
                modules: a.modules.clone(),
                model: if a.model.is_empty() {
                    None
                } else {
                    Some(a.model.clone())
                },
                permissions: a
                    .permissions
                    .clone()
                    .unwrap_or_else(|| old_permissions.get(&a.name).cloned().unwrap_or_default()),
            });
        }
        if metas.is_empty() {
            return Err("至少要有一个 agent".to_string());
        }
        if meta.mode == "single" && metas.len() != 1 {
            return Err("单 agent 形态只接受一个 agent（模块数不限）".to_string());
        }
        // 子会话的名单是主名单的一份视图：只能改这一席的模块 / 模型 / 权限，不能在子会话里增删或改名。
        if owner.name != meta.name {
            let old: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
            let new: Vec<String> = metas.iter().map(|a| a.name.clone()).collect();
            if old != new {
                return Err("子会话不能改 agent 名单：要换人请改主会话或新建会话".to_string());
            }
        }
        // 档位：与「开始」同一把尺子——虚拟机档的选型不成立（多版本未定版 / 定版不存在 / 路径冲突）如实拒绝。
        let tier = match edit.tier.as_str() {
            "host" => crate::kernel::api::Tier::Host,
            "vm" => crate::kernel::api::Tier::Vm,
            other => return Err(format!("未知执行档位：{}（只认 host / vm）", other)),
        };
        let spec = crate::kernel::api::ExecSpec {
            tier,
            guest: edit.guest.clone(),
            pins: edit.pins.clone(),
            net: edit.net,
        };
        // 子会话的档位只随主会话：这里只锁 tier / guest，模块与模型仍按各自规则（后续再收口）。
        if meta.parent.is_some() && (spec.tier != meta.exec.tier || spec.guest != meta.exec.guest) {
            return Err(
                "子会话的档位随主会话：不能单独改 tier / guest（改主会话即可）".to_string(),
            );
        }
        // 承载校验：前置条件不具备时**不允许改入虚拟机档**（用户环境问题，不是选型问题）。
        // 界面上的"能不能选"由 SessionConfig 的 tier_ready 说同一件事，两处不会各说各话。
        // 已经在虚拟机档上的会话只校验**它自己那几项**（基础根等）：改模块、改模型、定版、开网络都不该被拦住——
        // 一条已存在的会话连改都不让改，是拿用户自己的记录当人质。
        let staying_vm =
            meta.exec.tier == crate::kernel::api::Tier::Vm && tier == crate::kernel::api::Tier::Vm;
        if staying_vm {
            // 留在 vm 档：只校验用户这次填的基础根（填错路径就是填错路径），
            // 不拿"本机能不能提供 vm 档"去拦一条已经存在的会话。
            let qemu = self.qemu_path();
            let share_tool = self.share_tool_path();
            let base_item = crate::capabilities::vm::api::tier_readiness(
                &self.root,
                &spec,
                qemu.as_deref(),
                share_tool.as_deref(),
                self.probe.as_ref(),
            )
            .requirements
            .into_iter()
            .find(|r| r.id == "base");
            if let Some(item) = base_item {
                if !item.met {
                    return Err(format!("{}：{}", item.detail, item.how));
                }
            }
        } else if let Some(why) = crate::capabilities::vm::api::tier_refusal(
            &self.root,
            &spec,
            self.qemu_path().as_deref(),
            self.share_tool_path().as_deref(),
            self.probe.as_ref(),
        ) {
            return Err(why);
        }
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| seen.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        // 入口按**目标环境**判：虚拟机档在 guest 里跑（guest 的 os），本机档在宿主；
        // 跨平台时还要看文件形式——宿主编出来的产物（如 Windows 的 .exe）在 linux guest 里跑不了。
        let guest = self.guest_os_of(&spec).map(|g| g.as_str());
        let target_os = guest.unwrap_or(std::env::consts::OS);
        let missing: Vec<crate::capabilities::vm::api::MissingEntry> =
            crate::capabilities::vm::api::missing_entries_for(&session_modules, target_os, &|p| {
                self.probe.is_executable(p)
                    && (target_os == std::env::consts::OS
                        || crate::capabilities::vm::api::runs_on_target(p, target_os))
            });
        crate::capabilities::vm::api::plan(
            &spec,
            &session_modules,
            &self.library_for(&spec),
            &self.guest_runtimes_of(&spec),
            &missing,
        )
        .map_err(|diags| crate::capabilities::vm::api::diagnose_text_for(&diags, guest))?;

        let mut new_meta = meta.clone();
        new_meta.modules = metas.iter().flat_map(|a| a.modules.clone()).collect();
        new_meta.agents = metas.clone();
        new_meta.exec = spec.clone();
        // 同一 agent 全树只有一份配置：子会话编辑写回所属名单；主会话 / proxy 子工作编辑同步整棵子树。
        let mut old_children = Vec::new();
        let mut new_children = Vec::new();
        if owner.name != meta.name {
            let mut new_owner = owner.clone();
            for a in &metas {
                if let Some(slot) = new_owner.agents.iter_mut().find(|x| x.name == a.name) {
                    *slot = a.clone();
                }
            }
            new_owner.modules = new_owner
                .agents
                .iter()
                .flat_map(|a| a.modules.clone())
                .collect();
            old_children.push(owner.clone());
            new_children.push(new_owner);
        } else {
            for child in self.subtree_of(sid) {
                if child == sid {
                    continue;
                }
                let mut cm = self.history.meta(&child)?;
                old_children.push(cm.clone());
                cm.exec = spec.clone();
                for ca in &mut cm.agents {
                    if let Some(na) = new_meta.agents.iter().find(|a| a.name == ca.name) {
                        *ca = na.clone();
                    }
                }
                cm.modules = cm.agents.iter().flat_map(|a| a.modules.clone()).collect();
                new_children.push(cm);
            }
        }
        // 模块归属是整棵树的规则（与档位无关）：先按编辑后的名单查一遍重复。
        let (check_root, check_extra): (&SessionMeta, Vec<&SessionMeta>) =
            if owner.name != meta.name {
                (
                    new_children.first().expect("子会话编辑要写回所属名单"),
                    vec![&new_meta],
                )
            } else {
                (&new_meta, new_children.iter().collect())
            };
        let check_tree = self.tree_meta_with(check_root, &check_extra)?;
        if let Some((m, a, b)) = duplicate_tree_module(&check_tree) {
            return Err(format!(
                "模块 {} 同时属于 agent {} 与 {}：同一模块在一条会话树里只能属于一个 agent",
                m, a, b
            ));
        }
        let was_vm = meta.exec.tier == crate::kernel::api::Tier::Vm;
        let is_vm = new_meta.exec.tier == crate::kernel::api::Tier::Vm;
        // 共享 guest 的装配按整棵树算：把这次的新值覆盖到作用域根的子树上，再出计划与沙箱。
        let (lease, tree, tree_plan) = if is_vm {
            let (lease, scope_meta) = self
                .guest_scope(&new_meta)?
                .ok_or_else(|| "虚拟机档会话没有解析出 guest 作用域".to_string())?;
            let mut extras: Vec<&SessionMeta> = vec![&new_meta];
            extras.extend(new_children.iter());
            let tree = self.tree_meta_with(&scope_meta, &extras)?;
            let plan = self.plan_for_meta_with(&tree, &roster)?;
            self.log.info(
                "conductor::edit_session",
                &format!(
                    "sid={}；{}",
                    sid,
                    crate::capabilities::vm::api::plan_summary(&plan)
                ),
            );
            (Some(lease), Some(tree), Some(plan))
        } else {
            (None, None, None)
        };
        // 只有组装输入变了才重建 guest：模块 / 环境变了要重建；只改模型 / 权限不重建。
        let needs_guest = is_vm
            && match (&old_tree, &tree) {
                (Some(old), Some(tree)) => guest_assembly_changed(old, tree),
                _ => true,
            };
        let job = if needs_guest {
            let tree = tree.as_ref().expect("is_vm 已准备 tree");
            let tree_plan = tree_plan.as_ref().expect("is_vm 已准备 plan");
            let sandboxes = self.sandboxes(tree, &roster)?;
            Some(EditGuest::Boot(Box::new(self.guest_boot(
                tree,
                tree_plan,
                &sandboxes,
                was_vm,
                lease.as_deref().expect("is_vm 已准备 lease"),
            ))))
        } else if was_vm && !is_vm {
            let old_lease = old_scope
                .as_ref()
                .map(|(l, _)| l.clone())
                .unwrap_or_else(|| meta.name.clone());
            Some(EditGuest::Stop(self.guest_stop(&old_lease)))
        } else {
            None
        };
        Ok((
            PreparedEdit {
                sid: sid.to_string(),
                old_meta: meta,
                new_meta,
                old_children,
                new_children,
            },
            job,
        ))
    }

    /// 目的：改配的第二段（核心）——写回 meta、撤旧围栏授权、记旁路配置、丢掉内存会话。
    pub(crate) fn commit_edit(&mut self, prepared: PreparedEdit) -> Result<(), String> {
        let PreparedEdit {
            sid,
            old_meta,
            new_meta,
            old_children,
            new_children,
        } = prepared;
        self.history.save_meta(&new_meta)?;
        for child in &new_children {
            self.history.save_meta(child)?;
        }
        // 权限或模块可能变了：撤掉按**旧配置**写下的围栏授权；下一次工具执行按新配置重授（即时生效）。
        self.release_session_fences(&old_meta);
        for child in &old_children {
            self.release_session_fences(child);
        }
        self.record_config(&sid, &new_meta);
        // 内存里那份是按旧配置装的：丢掉主会话与整棵子树，下一次访问按新配置从转录重建。
        self.sessions.remove(&sid);
        for child in &new_children {
            self.sessions.remove(&child.name);
        }
        Ok(())
    }

    /// 撤掉一次会话按**给定 meta** 写下的围栏授权（权限或模块变更后调用）。
    /// 其它平台没有持久授权（release 是空操作）；Windows 撤 ACE，下一次 `prepare_fence` 按实际 ACE 重授。
    /// 约束：**不在这里停 guest**——销毁要等 guest 落盘（秒级），那一段交给调用方线程（见 GuestBoot / GuestStop）。
    fn release_session_fences(&self, meta: &SessionMeta) {
        let roster = self.modules.roster();
        match self.sandboxes(meta, &roster) {
            Ok(sandboxes) => {
                for sb in &sandboxes.list {
                    let spec = crate::kernel::api::FenceSpec::from_sandbox(sb, meta.exec.net)
                        .with_read_only(self.fence_read_roots());
                    if let Err(e) = self.tools.release_fence(&spec) {
                        self.log.warn(
                            "conductor::release_session_fences",
                            &format!("撤销围栏授权未完成：{}", e),
                        );
                    }
                }
            }
            Err(e) => self.log.warn(
                "conductor::release_session_fences",
                &format!("取沙箱失败，未撤销授权：{}", e),
            ),
        }
    }

    /// 追加一条旁路配置记录：只作呈现与审计（不进模型上下文，回放与状态派生都跳过它）。
    pub(crate) fn record_config(&self, sid: &str, meta: &SessionMeta) {
        let ev = serde_json::json!({
            "type": "config",
            "ts": now_ts(),
            "mode": meta.mode,
            "tier": meta.exec.tier.as_str(),
            "guest": meta.exec.guest,
            "net": meta.exec.net,
            "pins": meta.exec.pins,
            "agents": meta.agents.iter().map(|a| serde_json::json!({
                "name": a.name,
                "modules": a.modules,
                "model": a.model,
            })).collect::<Vec<_>>(),
        });
        if let Err(e) = self.history.append(sid, &[ev]) {
            self.log.warn(
                "conductor::record_config",
                &format!("配置记录落盘失败：{}", e),
            );
        }
    }

    /// 工作名的缺省与唯一化（命名策略在 `session`；这里只提供"存在吗"）。
    pub fn unique_work_name(&self, base: &str, fallback: &str) -> String {
        crate::capabilities::session::api::unique_work_name(base, fallback, |n| {
            self.sessions.contains_key(n) || self.history.load(n).is_ok()
        })
    }

    /// 本次模型调用的通道参数：**预算与"能不能流式"都取全局设置**（讨论、执行、验收、单 agent 共用一份）。
    /// `want_stream` 是调用方这一次的意愿（呈现层按回包形状给）：**设置是上限，调用方可以在本次放弃流式**；
    /// 设置关掉时一律非流式。两处各判一次迟早会打架，所以判据只在这里。
    pub(crate) fn llm_opts(&self, want_stream: bool) -> crate::capabilities::llm::api::LlmOpts {
        let app = self.registry.app();
        crate::capabilities::llm::api::LlmOpts {
            stream: app.streaming && want_stream,
            timeout_secs: app.llm_timeout_secs,
        }
    }

    /// 目的：本机实际要用的 QEMU 可执行文件：设置里登记了就用它，没登记就采用依赖补齐装进项目内的那一个。
    /// 约束：**每次判据取用时现算**（不是启动时定一次）——补齐作业装完即生效，不需要重启进程；
    ///   显式登记永远优先，这里不改用户的 `settings.yaml`。
    pub(crate) fn qemu_path(&self) -> Option<String> {
        self.registered_or_installed("qemu_path", "vm.qemu.")
    }

    /// 目的：同 `qemu_path`，virtiofs 守护进程。
    pub(crate) fn virtiofsd_path(&self) -> Option<String> {
        self.registered_or_installed("virtiofsd_path", "vm.virtiofsd.")
    }

    /// 目的：同 `qemu_path`，SMB helper。
    pub(crate) fn smbd_path(&self) -> Option<String> {
        self.registered_or_installed("smbd_path", "vm.smbd.")
    }

    /// 目的：本平台共享档要用的外部工具路径（Windows = smbd；其余 = virtiofsd）；都不在场给 None。
    /// 约束：组合选择与就绪检测共用这一份，避免"说要验 A、实际跑 B"。
    pub(crate) fn share_tool_path(&self) -> Option<String> {
        if cfg!(windows) {
            self.smbd_path()
        } else {
            self.virtiofsd_path()
        }
    }

    /// 目的：一个外部工具路径的**唯一**取法：设置里登记优先，空了就用补齐面报的"已装在项目内"的那个。
    /// 参数：which = 设置字段名（只用于说明）；prefix = 清单 id 前缀（`vm.qemu.` 这类，按平台后缀区分）。
    /// 约束：每次现算——补完立刻能被判据看见，也就没有"装完还得重启"这一步。
    fn registered_or_installed(&self, which: &str, prefix: &str) -> Option<String> {
        let app = self.registry.app();
        let registered = match which {
            "virtiofsd_path" => app.virtiofsd_path.trim(),
            "smbd_path" => app.smbd_path.trim(),
            _ => app.qemu_path.trim(),
        };
        if !registered.is_empty() {
            return Some(registered.to_string());
        }
        // 没登记：按补齐面报的在场条目取（**绝对路径**——清单里的 probe 是相对产品根的写法）。
        self.provision
            .status()
            .ok()?
            .iter()
            .find(|d| d.present && d.id.starts_with(prefix))
            .map(|d| self.root.join(&d.probe).to_string_lossy().into_owned())
    }

    /// 用户显式授权的只读根（`settings.yaml` 的 `fence_read`）。
    /// 策略层只带事实：哪些目录只读可达由用户定，只读位怎么落由适配层定。
    /// 空 = 一个都不放行（默认不动本机任何权限项）。
    pub(crate) fn fence_read_roots(&self) -> Vec<std::path::PathBuf> {
        self.registry
            .app()
            .fence_read
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .collect()
    }

    // ---- 会话中心（前端只持 id） ----

    /// 目的：上下文「继续」是否该出现——判据只在后端（前端不按末条/模式猜）。
    /// 约束：正在跑、已关闭、代理、或没有真实断点时不出现；协作有挂起卡时由卡承担，不给继续。
    fn continue_action(
        &self,
        sid: &str,
        running: bool,
        mode: &str,
        run: &str,
        has_pending: bool,
    ) -> Option<ContinueView> {
        if running || run == "closed" || mode == "proxy" {
            return None;
        }
        match mode {
            "single" => {
                let user_last =
                    matches!(self.sessions.get(sid), Some(Session::Single(s)) if s.last_is_user());
                if user_last {
                    Some(ContinueView {
                        kind: "turn".to_string(),
                        label: "继续上一轮".to_string(),
                        reason: "上一轮还没有回答".to_string(),
                    })
                } else {
                    None
                }
            }
            "collab" if !has_pending => Some(ContinueView {
                kind: "collab".to_string(),
                label: "恢复协作".to_string(),
                reason: "协作泵没有在跑".to_string(),
            }),
            _ => None,
        }
    }

    /// 目的：一条会话的权威视图（启用回包 / 正在生成时附着用）：状态、运行态与上下文继续。
    pub(crate) fn session_view(&self, sid: &str) -> Result<SessionView, String> {
        self.session_views(&self.history_list())
            .into_iter()
            .find(|v| v.sid == sid)
            .ok_or_else(|| format!("无此会话：{}", sid))
    }

    /// 会话列表视图（进行中的工作）。形态取落盘 meta（单一真相，不在内存里留影子状态）。
    pub fn session_views(&self, history: &[HistoryView]) -> Vec<SessionView> {
        // 在表里的会话 + **正在生成的会话**（后者对象在工作线程上，但它确实存在、也确实在跑）。
        // 漏掉它们会让界面以为会话不见了。
        let running: Vec<(String, bool)> = self
            .running
            .iter()
            .map(|sid| (sid.clone(), false))
            .collect();
        let running_now: std::collections::BTreeSet<String> =
            self.running.iter().cloned().collect();
        let listed: Vec<(String, bool)> = self
            .sessions
            .iter()
            .map(|(sid, s)| {
                let done = match s {
                    Session::Collab(c) => c.is_done(),
                    Session::Single(_) => false,
                };
                (sid.clone(), done)
            })
            .chain(running)
            .collect();
        listed
            .into_iter()
            .map(|(sid, done)| {
                let entry = history.iter().find(|h| h.name == sid);
                let mode = entry.map(|h| h.mode.clone()).unwrap_or_default();
                // 运行态取落盘事实（不在内存里留影子状态）；拿不到（生成中 / 未落盘）按正常运行。
                let run = entry
                    .map(|h| h.run.as_str())
                    .unwrap_or("active")
                    .to_string();
                // 记的档位来自落盘 meta（权威）：环境变了也要如实提示——**不拦打开**（记录是用户的）。
                let exec = entry.map(|h| h.exec.clone()).unwrap_or_default();
                let qemu = self.qemu_path();
                let share_tool = self.share_tool_path();
                let readiness = crate::capabilities::vm::api::tier_readiness(
                    &self.root,
                    &exec,
                    qemu.as_deref(),
                    share_tool.as_deref(),
                    self.probe.as_ref(),
                );
                // 「改需求」能力位：有本次需求行才给。在表里看会话种类；不在表里（生成中/未打开）
                // 看落盘 meta 的形态（那是名单与形态的单一真相）。
                let can_update_task = match self.sessions.get(&sid) {
                    Some(Session::Collab(_)) => true,
                    Some(Session::Single(_)) => false,
                    None => mode == "collab",
                };
                // 待裁决：**判据只有队本身**（核心各关卡与工具级确认排在同一条队上）——
                // 正在生成的会话（对象在工作线程手里）照样取得到；推的卡片事件与它是同一份。
                let pending = self.desk.peek(&sid).and_then(|d| d.json());
                let running = running_now.contains(&sid);
                let continue_action =
                    self.continue_action(&sid, running, &mode, &run, pending.is_some());
                SessionView {
                    running,
                    sid,
                    mode,
                    done,
                    tier: exec.tier.as_str().to_string(),
                    tier_ready: readiness.ready(),
                    tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
                    can_update_task,
                    run,
                    pending,
                    continue_action,
                }
            })
            .collect()
    }

    pub fn session_exists(&self, name: &str) -> bool {
        self.sessions.contains_key(name)
    }

    /// 目的：测试与内部编排用的同步入口（准备 → 起 guest → 提交，全在当前线程）。
    /// 约束：生产走句柄的两段（`prepare_work` / `commit_work`），核心线程不陪着等 QEMU。
    #[cfg(test)]
    pub fn create_work(&mut self, spec: WorkSpec) -> Result<WorkOpened, String> {
        self.create_work_inner(spec, None)
    }

    /// 这棵子树里已经用过的 agent 实例名（实例名就是沙箱目录名，必须**全树唯一**）。
    /// 顶层会话名可以重复（落点是自己的目录），但共用同一个 work/ 时同名 agent 会撞同一个沙箱。
    fn subtree_agent_names(&self, root: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for sid in self.subtree_of(root) {
            if let Ok(m) = self.history.meta(&sid) {
                for a in &m.agents {
                    if !out.iter().any(|x| x == &a.name) {
                        out.push(a.name.clone());
                    }
                }
            }
        }
        out
    }

    /// 目的：建工作的同步实现（测试与内部编排用）：准备 → 起 guest → 提交。
    /// 约束：生产走句柄的两段；`parent` = 编排归属（代理建的**子工作**，与父会话共用顶层那个 work/）。
    #[cfg(test)]
    pub(crate) fn create_work_inner(
        &mut self,
        spec: WorkSpec,
        parent: Option<&str>,
    ) -> Result<WorkOpened, String> {
        let (prepared, boot) = self.prepare_work(spec, parent)?;
        if let Some(boot) = boot {
            if let Err(e) = boot.run() {
                self.abort_work(prepared);
                return Err(e);
            }
        }
        self.commit_work(prepared)
    }

    /// 目的：创建的第一段（核心，毫秒级）——校验 / 占名 / 建工作区 / 出计划；要起 guest 时把那一段交出去。
    /// 返回：准备结果 + 可选的那段"要等 guest"的活（虚拟机档才有）。
    /// 错误：形态、名字、档位、名单、选型不成立时如实拒绝，什么都不留下。
    pub(crate) fn prepare_work(
        &mut self,
        spec: WorkSpec,
        parent: Option<&str>,
    ) -> Result<(PreparedWork, Option<GuestBoot>), String> {
        validate_work_name(&spec.name)?;
        if self.sessions.contains_key(&spec.name)
            || self.history.load(&spec.name).is_ok()
            || self.starting.contains(&spec.name)
        {
            return Err(format!("工作名已存在：{}", spec.name));
        }
        // **代理形态**（第三人形态）：没有名单、没有需求——用户选这一形态就是**授予全权**。
        // 它不是一个 agent 工作，装配与其余形态没有共同点，所以在这里就地分岔、不往下走。
        if spec.mode == WorkMode::Proxy {
            if !spec.agents.is_empty() || spec.task.is_some() {
                return Err(
                    "代理形态不接受 agent 名单或本次需求：决定权是整块交给核心的".to_string(),
                );
            }
            if parent.is_some() {
                return Err("子工作不能建成代理形态：代理不能往里套代理".to_string());
            }
            let (sid, facts) = self.create_proxy_with_facts(
                &spec.name,
                now_ts(),
                spec.tier,
                spec.guest.as_deref(),
            )?;
            return Ok((
                PreparedWork::Ready(WorkOpened {
                    sid,
                    agents: Vec::new(),
                    facts,
                }),
                None,
            ));
        }
        // 代拟路径（协作、未给 agent）允许先空着，由核心按需求拟名单；其余形态必须有 agent。
        if spec.agents.is_empty() && !spec.delegate {
            return Err("至少要有一个 agent".to_string());
        }
        // **单模式的组合语义**：点名了多个 agent = 把它们的模块并成一个**临时组合**（去重、保序；
        // 模型取核心默认）。规则只有这一处——前端不再自己拼（见 ARCHITECTURE.md §一）。
        let mut spec = spec;
        if spec.mode == WorkMode::Single && spec.agents.len() > 1 {
            spec.agents = vec![merge_into_one(&spec.agents, &spec.name)];
        }
        let roster = self.scan();
        let reserved = self.reserved_names();
        // 校验 agent：名字合法、模块与模型真实存在；同一模块不得同时属于两个 agent（沙箱会歧义）
        let mut seen_modules: Vec<String> = Vec::new();
        for a in &spec.agents {
            crate::capabilities::registry::api::validate_name(&a.name)?;
            crate::capabilities::registry::api::check_reserved(&a.name, &reserved)?;
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
                if seen_modules.iter().any(|x| x == id) {
                    return Err(format!(
                        "模块 {} 被多个 agent 同时使用；同一模块只能属于一个 agent",
                        id
                    ));
                }
                seen_modules.push(id.clone());
            }
            if let Some(mid) = &a.model {
                if !self.registry.has_model(mid) {
                    return Err(format!("无此模型：{}", mid));
                }
                self.registry.resolve(mid)?;
            }
        }
        // 形态约束（模块可以为空，上面只校验真实存在与归属不冲突）
        match spec.mode {
            WorkMode::Single => {
                if spec.agents.len() != 1 {
                    return Err("单 agent 形态只接受一个 agent（模块数不限）".to_string());
                }
            }
            // 代理形态在函数开头就返回了（它没有 agent 名单）；这里只为穷尽，不产生行为。
            WorkMode::Proxy => {}
            WorkMode::Collab => {
                if spec.task.as_deref().unwrap_or("").trim().is_empty() {
                    return Err("协作模式必须填写本次需求".to_string());
                }
            }
        }
        // 工作根：顶层会话就是自己；子会话沿父链走到顶（整棵树只有一个 work/）。
        let root = match parent {
            Some(p) => self.work_root(p)?,
            None => spec.name.clone(),
        };
        // 实例名在**整棵子树**里唯一：共用同一个 work/ 时同名 agent 会撞同一个沙箱目录。
        // 重名 → 尾号（用户不改名时的兜底），绝不重名。顶层根还不存在时子树名单为空。
        let mut taken: Vec<String> = self.subtree_agent_names(&root);
        let mut metas: Vec<AgentMeta> = Vec::new();
        for a in &spec.agents {
            let name = crate::capabilities::registry::api::unique_instance_name(&a.name, &taken);
            taken.push(name.clone());
            metas.push(AgentMeta {
                name,
                transient: a.transient,
                modules: a.modules.clone(),
                model: a.model.clone(),
                permissions: Default::default(),
            });
        }
        // 模块扁平清单（展示用；顺序按 agent 名单展开）
        let module_ids: Vec<String> = metas.iter().flat_map(|m| m.modules.clone()).collect();
        // 代拟：给了 agent 就不再代拟（名单已经有了，不让核心盖掉用户的选择）。
        let delegate = spec.delegate && metas.is_empty();

        let name = spec.name.clone();
        let agent_names: Vec<String> = metas.iter().map(|m| m.name.clone()).collect();
        let meta = SessionMeta {
            name: name.clone(),
            mode: mode_str(spec.mode).to_string(),
            delegate,
            modules: module_ids.clone(),
            task: spec.task.clone(),
            ts: now_ts(),
            agents: metas.clone(),
            // 编排归属（代理建的子工作 = Some(父)）；节点子会话由 spawn_sub_session 另建。
            parent: parent.map(|s| s.to_string()),
            node: None,
            delegation: None,
            // 环境属于主会话：子会话整段沿用父会话（档位 / 虚拟根 / 网络 / 定版）；
            // 顶层来自**用户在创建向导里的选择**（默认 = 设置里的档位）。承载不了由下面如实拒绝。
            exec: match parent {
                Some(p) => self.history.meta(p)?.exec,
                None => crate::kernel::api::ExecSpec {
                    tier: spec.tier,
                    guest: spec.guest.clone(),
                    ..crate::kernel::api::ExecSpec::default()
                },
            },
            run: RunState::Active,
        };
        // 承载校验：默认档位的前置条件不具备时**不允许创建虚拟机档会话**（用户环境问题，不是选型问题）。
        // 必须在建工作区之前收口——拒绝就该什么都不留下。
        if let Some(why) = crate::capabilities::vm::api::tier_refusal(
            &self.root,
            &meta.exec,
            self.qemu_path().as_deref(),
            self.share_tool_path().as_deref(),
            self.probe.as_ref(),
        ) {
            return Err(why);
        }
        // 工作区：整棵树只有顶层一个 work/；各 agent 沙箱按实例名建在它下面。
        // （失败即失败，不假装已建；代拟确认名单时再补建。）
        self.workspace.prepare(&root, &agent_names)?;
        let sandboxes = self.sandboxes(&meta, &roster)?;
        // 扫描事实如实埋点：本会话用到的模块里，哪些声明的运行包不在包库（缺包同样会在下面按计划拒绝，文件与 guest 声明都看）。
        // 包库只服务虚拟机档：本机档不装载运行包，这条警告在那里没有意义（宿主自备解释器）。
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| module_ids.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        if meta.exec.tier == crate::kernel::api::Tier::Vm {
            for (id, caps) in
                crate::capabilities::vm::api::absent(&session_modules, &self.vm.library())
            {
                self.log.warn(
                    "conductor::create_work",
                    &format!("模块 {} 声明的运行包不在包库：{}", id, caps.join("、")),
                );
            }
        }
        // 执行选型完整性（「开始」即冻结）：选型不成立或**缺包**都如实拒绝（没有工具，用户完不成工作）。
        // 入口**按目标环境**判：虚拟机档按 guest 的 os 解析名字，跨平台还看文件形式（宿主编的产物跑不了）。
        let guest = self.guest_os_of(&meta.exec).map(|g| g.as_str());
        let target_os = guest.unwrap_or(std::env::consts::OS);
        let missing: Vec<crate::capabilities::vm::api::MissingEntry> =
            crate::capabilities::vm::api::missing_entries_for(&session_modules, target_os, &|p| {
                self.probe.is_executable(p)
                    && (target_os == std::env::consts::OS
                        || crate::capabilities::vm::api::runs_on_target(p, target_os))
            });
        let plan = crate::capabilities::vm::api::plan(
            &meta.exec,
            &session_modules,
            &self.library_for(&meta.exec),
            &self.guest_runtimes_of(&meta.exec),
            &missing,
        )
        .map_err(|diags| crate::capabilities::vm::api::diagnose_text_for(&diags, guest))?;
        self.log.info(
            "conductor::create_work",
            &crate::capabilities::vm::api::plan_summary(&plan),
        );

        // 占名：准备与提交之间隔着一次 guest 启动，同名并发创建不能都通过上面那道检查。
        self.starting.insert(name.clone());
        // 子会话与父会话共用同一个 session/ 目录：只对顶层会话记得"失败要撤掉空目录"。
        let discard = if parent.is_none() {
            Some((name.clone(), agent_names.clone()))
        } else {
            None
        };
        // 代拟名单还没确认：沙箱/模块都不知道，先不起 guest；第一次真正生成前 ensure_guest 再起。
        let boot = if meta.exec.tier == crate::kernel::api::Tier::Vm
            && !(meta.delegate && meta.agents.is_empty())
        {
            // 共享 guest：按整棵树装配（proxy 子工作都进代理根那一台），租约取作用域根。
            let (lease, scope_meta) = self
                .guest_scope(&meta)?
                .ok_or_else(|| "虚拟机档会话没有解析出 guest 作用域".to_string())?;
            let tree = self.tree_meta(&scope_meta, Some(&meta))?;
            if let Some((m, a, b)) = duplicate_tree_module(&tree) {
                return Err(format!(
                    "模块 {} 同时属于 agent {} 与 {}：同一模块在一条会话树里只能属于一个 agent",
                    m, a, b
                ));
            }
            let tree_plan = self.plan_for_meta_with(&tree, &roster)?;
            let tree_sandboxes = self.sandboxes(&tree, &roster)?;
            // 运行包不做动态加载：新子会话的包已被这台 guest 装载就直接加入；
            // 引入新包时要等树空闲后按聚合后的 meta 重建这台共享 guest。
            let want: Vec<String> = tree_plan
                .packages
                .iter()
                .map(|p| format!("{}@{}", p.id, p.version))
                .collect();
            let active = self.vm.guest_leases().iter().any(|l| l == &lease);
            let have = self.vm.guest_packages(&lease);
            let restart = needs_guest_packages_rebuild(active, &want, have.as_deref());
            if restart {
                let running = self
                    .subtree_of(&scope_meta.name)
                    .into_iter()
                    .find(|s| self.running.contains(s));
                if let Some(s) = running {
                    return Err(format!(
                        "子会话引入了这台 guest 还没装载的运行包：先停掉正在生成的 {}，再重建共享 guest",
                        s
                    ));
                }
            }
            Some(self.guest_boot(&tree, &tree_plan, &tree_sandboxes, restart, &lease))
        } else {
            None
        };
        Ok((
            PreparedWork::Pending {
                meta: Box::new(meta),
                mode: spec.mode,
                metas,
                task: spec.task,
                delegate,
                name,
                agent_names,
                sandboxes,
                discard,
            },
            boot,
        ))
    }

    /// 目的：创建的第二段（核心，毫秒级）——装配会话对象、落盘身份、开场事实。
    /// 约束：guest 已经起好了（或本机档不需要）；落盘失败要撤掉占名与刚建的空工作区目录。
    pub(crate) fn commit_work(&mut self, prepared: PreparedWork) -> Result<WorkOpened, String> {
        let (meta, mode, metas, task, delegate, name, agent_names, sandboxes, discard) =
            match prepared {
                // 代理形态在准备段里就建完了：这里原样交回。
                PreparedWork::Ready(opened) => return Ok(opened),
                PreparedWork::Pending {
                    meta,
                    mode,
                    metas,
                    task,
                    delegate,
                    name,
                    agent_names,
                    sandboxes,
                    discard,
                } => (
                    *meta,
                    mode,
                    metas,
                    task,
                    delegate,
                    name,
                    agent_names,
                    sandboxes,
                    discard,
                ),
            };
        let roster = self.scan();

        let (session, mut events) = match mode {
            // 代理形态在函数开头就建好返回了：它没有 agent 名单，不走这条装配路。
            WorkMode::Proxy => return Err("代理形态没有 agent 名单，不经这条装配路".to_string()),
            // 单 agent（模块数不限）。
            WorkMode::Single => {
                let a = metas.first().ok_or("至少要有一个 agent")?;
                let sb = sandboxes
                    .for_agent(&a.name)
                    .cloned()
                    .ok_or_else(|| format!("缺少 agent {} 的沙箱", a.name))?;
                let chosen: Vec<Module> = a
                    .modules
                    .iter()
                    .filter_map(|id| {
                        roster
                            .modules
                            .iter()
                            .find(|m| &m.manifest.id == id)
                            .cloned()
                    })
                    .collect();
                let channel = self.channel_of(a.model.as_deref());
                let unavailable = self.unavailable_modules(&meta.exec, &chosen);
                let guest = self.guest_os_of(&meta.exec);
                let (s, opened) = self.build_single(
                    a,
                    &chosen,
                    channel,
                    &sb,
                    unavailable,
                    meta.exec.net,
                    meta.exec.tier,
                    guest,
                    self.guest_lease_for(&meta)?,
                );
                (Session::Single(s), opened)
            }
            WorkMode::Collab => {
                let task = task.as_deref().unwrap_or("").trim().to_string();
                // 裁决队按会话 id 取：核心各关卡与这一席的工具级确认共用同一条队。
                let door = self.desk_of(&name);
                let mut cs = CollabSession::start(
                    Arc::clone(&self.llm),
                    Arc::clone(&self.vm),
                    Arc::clone(&self.modules),
                    self.registry.snapshot(),
                    Arc::clone(&self.prompt),
                    Arc::clone(&self.systools),
                    Arc::clone(&self.tools),
                    Arc::clone(&self.log),
                    meta.exec.clone(),
                    self.guest_lease_for(&meta)?,
                    metas.clone(),
                    delegate,
                    sandboxes.clone(),
                    door,
                )?;
                let mut out = Vec::new();
                cs.set_task(&task, &mut |e| out.push(e));
                (Session::Collab(cs), out)
            }
        };
        // 会话身份落盘：失败即失败（名字即目录，这是承诺，不假装已保存）。
        self.starting.remove(&name);
        if let Err(e) = self.history.create(&meta) {
            if let Some((session, agents)) = discard {
                let _ = self.workspace.discard(&session, &agents);
            }
            return Err(e);
        }
        self.sessions.insert(name.clone(), session);
        self.record_events(&name, &mut events);
        // 事实（开场转录 + 提示）随结果交出：核心不持有事件台，发布是 api 层的职责。
        Ok(WorkOpened {
            sid: name,
            agents: agent_names,
            facts: events,
        })
    }

    /// 目的：创建失败（guest 起不来 / 落盘失败）时撤销准备段占下的东西：名字占位 + 这次建出来的空工作区目录。
    pub(crate) fn abort_work(&mut self, prepared: PreparedWork) {
        if let PreparedWork::Pending { name, discard, .. } = prepared {
            self.starting.remove(&name);
            if let Some((session, agents)) = discard {
                let _ = self.workspace.discard(&session, &agents);
            }
        }
    }

    /// 目的：把"停 guest"这一段打包成交给调用方线程的活（销毁要等它落盘，秒级）。
    fn guest_stop(&self, lease: &str) -> super::guest_job::GuestStop {
        super::guest_job::GuestStop::new(Arc::clone(&self.vm), vec![lease.to_string()])
    }

    /// 目的：按一条会话的现状准备"确保 guest"那一段（启用、发言、成员回合共用）。
    /// 返回：None = 本机档 / proxy 父会话 / 已有活 guest / 代拟名单还没确认；有 = 要在调用方线程跑。
    pub(crate) fn prepare_guest_boot(
        &self,
        sid: &str,
    ) -> Result<Option<super::guest_job::GuestBoot>, String> {
        let meta = self.history.meta(sid)?;
        if meta.run == RunState::Closed {
            return Err(format!("会话 {} 已关闭：终态，不能再启用", sid));
        }
        let Some((lease, scope_meta)) = self.guest_scope(&meta)? else {
            return Ok(None);
        };
        if self.vm.guest_leases().iter().any(|l| l == &lease) {
            return Ok(None);
        }
        // 代拟名单还没确认：沙箱/模块都还不知道，等第一次真正生成前再起（见 guest_scope / spawn）。
        if scope_meta.agents.is_empty() && scope_meta.delegate {
            return Ok(None);
        }
        let roster = self.scan();
        // 共享 guest 的装配按整棵树算：proxy 根自己没有名单，子工作的 agent / 模块 / 包都要进来。
        let tree = self.tree_meta(&scope_meta, None)?;
        let plan = self.plan_for_meta_with(&tree, &roster)?;
        let sandboxes = self.sandboxes(&tree, &roster)?;
        Ok(Some(
            self.guest_boot(&tree, &plan, &sandboxes, false, &lease),
        ))
    }

    /// 目的：把"起 guest"这一段打包成交给调用方线程的活——核心只准备参数，不在这里拉起。
    fn guest_boot(
        &self,
        meta: &SessionMeta,
        plan: &crate::capabilities::vm::api::ExecPlan,
        sandboxes: &crate::capabilities::workspace::api::Sandboxes,
        restart: bool,
        lease: &str,
    ) -> GuestBoot {
        let share_tool = self.share_tool_path();
        GuestBoot::new(
            Arc::clone(&self.vm),
            self.root.clone(),
            Arc::clone(&self.probe),
            Arc::clone(&self.log),
            self.registry.app().max_guests,
            self.fence_read_roots(),
            self.qemu_path(),
            share_tool.clone(),
            crate::capabilities::vm::api::share_chain(
                std::env::consts::OS,
                share_tool.as_deref(),
                self.probe.as_ref(),
            ),
            lease.to_string(),
            meta.exec.net,
            plan.clone(),
            sandboxes.clone(),
            restart,
        )
    }
    /// 界面投喂：把文件写进本次工作的 work/。
    /// 返回 Ok(false) = 同名文件已存在且未选择覆盖（交前端让用户决定：覆盖/改名/取消）。
    pub fn work_upload(
        &mut self,
        sid: &str,
        name: &str,
        bytes: &[u8],
        overwrite: bool,
    ) -> Result<bool, String> {
        // 策略在 conductor：先净化文件名，再交给工作区端口（机制只看已净化的名字）。
        let name = crate::capabilities::workspace::api::safe_file_name(name)?;
        if !self.sessions.contains_key(sid) && self.history.load(sid).is_err() {
            return Err(format!("无此会话：{}", sid));
        }
        // 先恢复未完成的版本库事务，再做同名文件判定：否则 pending 里的文件会被当成不存在。
        let work = self.work_root(sid).unwrap_or_else(|_| sid.to_string());
        self.workspace.work_recover(&work)?;
        if self.workspace.work_has(&work, &name) && !overwrite {
            return Ok(false);
        }
        // 用户投喂 = 一次**权威提交**（作者 user）：共享区只有提交这一条写路径。
        // 锚到主会话当时的下一条行号，回档才能把这次投喂算进某个转录点。
        let line = match self.sessions.get(sid) {
            Some(Session::Single(s)) => s.next_line,
            Some(Session::Collab(c)) => c.next_line,
            None => 0,
        };
        self.workspace
            .work_commit_user(&work, &name, bytes, now_ts(), line)?;
        Ok(true)
    }

    /// 本工作可引用的文件清单 + 真实根（前端 @ 菜单与「长路径缩写」用）。
    /// 名单取自会话 meta（活动会话与历史会话都以 meta.agents 为权威）；列目录的机制在 Workspace 端口，
    /// 根取自沙箱（与文件清单同源）：agents 与 roots.agents 同序同名。
    pub fn files_view(&self, sid: &str) -> Result<FilesView, String> {
        let (meta, _) = self
            .history
            .load(sid)
            .map_err(|_| format!("无此会话：{}", sid))?;
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        let files = self.workspace.files(sid, &names)?;
        let usage = self.workspace.usage(sid, &names)?;
        let roster = self.scan();
        let sandboxes = self.sandboxes(&meta, &roster)?;
        let mut agents: Vec<FilesAgentView> = Vec::new();
        let mut agent_roots: Vec<FilesAgentRootView> = Vec::new();
        for a in &meta.agents {
            agents.push(FilesAgentView {
                name: a.name.clone(),
                files: files.agents.get(&a.name).cloned().unwrap_or_default(),
            });
            agent_roots.push(FilesAgentRootView {
                name: a.name.clone(),
                root: sandboxes
                    .for_agent(&a.name)
                    .map(|s| crate::kernel::api::slash(&s.private))
                    .unwrap_or_default(),
            });
        }
        Ok(FilesView {
            work: files.work,
            agents,
            roots: FilesRootsView {
                work: crate::kernel::api::slash(&sandboxes.shared),
                agents: agent_roots,
            },
            usage,
        })
    }
}

/// 目的：新子会话要不要为运行包重建共享 guest——只有活 guest 且它缺其中某个包时才要。
pub(crate) fn needs_guest_packages_rebuild(
    active: bool,
    want: &[String],
    have: Option<&[String]>,
) -> bool {
    if !active {
        return false;
    }
    match have {
        Some(have) => want.iter().any(|p| !have.contains(p)),
        None => !want.is_empty(),
    }
}

/// 目的：一个模块在整棵树里只能属于一个 agent；返回（模块, 先出现的 agent, 后出现的 agent）。
fn duplicate_tree_module(meta: &SessionMeta) -> Option<(String, String, String)> {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for a in &meta.agents {
        for m in &a.modules {
            if let Some(prev) = seen.insert(m.clone(), a.name.clone()) {
                return Some((m.clone(), prev, a.name.clone()));
            }
        }
    }
    None
}

/// 目的：guest 装配只认"agent → 模块集合"与环境；模型 / 权限变化不该触发重建。
fn agent_modules(meta: &SessionMeta) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for a in &meta.agents {
        let mut modules = a.modules.clone();
        modules.sort();
        modules.dedup();
        out.insert(a.name.clone(), modules);
    }
    out
}

/// 目的：这次改配要不要重建共享 guest——模块集合或 ExecSpec（档位 / 虚拟根 / 网络 / 定版）变了才要。
fn guest_assembly_changed(old: &SessionMeta, new: &SessionMeta) -> bool {
    old.exec.tier != new.exec.tier
        || old.exec.guest != new.exec.guest
        || old.exec.net != new.exec.net
        || old.exec.pins != new.exec.pins
        || agent_modules(old) != agent_modules(new)
}
