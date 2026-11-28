//! 目的：会话配置的读取、改配两段与权限档提交——配置视图、改配准备、提交落盘。
//! 管：配置视图、改配准备（新 meta 与子树上浮）、提交落盘、围栏撤权、权限档写入、配置记录。
//! 不管：建工作与 guest 启动（`create.rs`）、报告与档位（`report.rs`）。
//! 联动：api 层的配置接口与 `work_tools` 经这里调用；落盘走 history 与 workspace。
//!   本方法族只碰 probe / history / sessions / log 这四个字段。

use super::*;
use crate::units::business::conversation::session::api::{
    config_owner_of, flat_modules, guest_scope, subtree_of, tree_meta, tree_meta_with, work_root,
};

impl Work {
    /// 目的：配置视图：把「能改什么、现在是什么、缺什么」如实给出（每次读取都重扫模块清单与包库）。
    pub fn session_config(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        sid: &str,
    ) -> Result<SessionConfig, String> {
        let (meta, events) = self.history_open(sid)?;
        // 过期内容替换与保底是**工作根**的会话级事实：配置界面读根上的值。
        let root = work_root(self.history.as_ref(), sid).unwrap_or_else(|_| sid.to_string());
        let root_meta = self.history.meta(&root)?;
        // 机器能力（与某条会话无关）：界面据此决定 guest 侧能不能**选**；虚拟根是会话级，另行校验。
        let machine = self
            .policies
            .machine(crate::units::business::sandbox::api::VM_POLICY);
        let vm_available = machine.ready();
        let vm_reason = self
            .policies
            .machine_refusal(crate::units::business::sandbox::api::VM_POLICY)
            .unwrap_or_default();
        let vm_requirements = machine.requirements;
        // 会话级判据（机器能力 + 这条会话选的虚拟根）只算一次：两处读同一个结论，不各算一遍。
        let readiness = self.policies.readiness(&meta.sandbox, &meta.target());
        Ok(SessionConfig {
            sid: meta.name.clone(),
            kind: meta.spec.kind.clone(),
            started: session_started(&events),
            agents: meta
                .agents
                .iter()
                .map(|a| ConfigAgent {
                    name: a.name.clone(),
                    modules: a.modules.clone(),
                    model: a.model.clone().unwrap_or_default(),
                    chara: a.chara.clone(),
                })
                .collect(),
            sandbox: meta.sandbox.policy.clone(),
            sandbox_locked: meta.parent.is_some(),
            guest: meta.facts.guest.clone(),
            guests: self.provision.guest_bases()?,
            net: registry.app().sandbox.policy.network,
            permission: self.permission_view(turn, registry, &meta)?,
            thinking: self.thinking_view_for_root(registry, &root),
            context_freshness: root_meta.context_freshness,
            freshness_keep_percent: root_meta.freshness_keep_percent,
            pins: meta.sandbox.pins.clone(),
            runtime: self.runtime_report_for(probe, &meta.sandbox),
            runtimes_dir: crate::kernel::api::path::slash(&self.packages.runtimes_dir()),
            sandbox_ready: readiness.ready(),
            sandbox_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
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
    pub fn edit_session(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        probe: &Arc<dyn HostProbe + Send + Sync>,
        sid: &str,
        edit: SessionEdit,
    ) -> Result<(), String> {
        let (prepared, job) = self.prepare_edit(turn, registry, probe, sid, edit)?;
        if let Some(job) = job {
            // 失败什么都不改：meta 还没落盘，现有会话原样还在。
            job.run()?;
        }
        self.commit_edit(turn, registry, prepared)
    }

    /// 目的：改配的第一段（核心，毫秒级）——全部校验 + 出计划，并给出"要等 guest"的那一段（可能没有）。
    /// 返回：准备结果 + 可选的那段活（切入 / 留在虚拟机档要起或重建；切回本机档要停旧的）。
    pub(crate) fn prepare_edit(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        probe: &Arc<dyn HostProbe + Send + Sync>,
        sid: &str,
        edit: SessionEdit,
    ) -> Result<(PreparedEdit, Option<EditGuest>), String> {
        let host: &dyn HostProbe = probe.as_ref();
        // 改配要重建整棵子树的档位/guest 关联：任一在跑都拒绝（先停再改）。
        // 权限是根级动作：切档要求整棵 work tree 空闲，避免子会话用旧快照继续跑。
        let guard = if edit.permission.is_some() {
            work_root(self.history.as_ref(), sid).unwrap_or_else(|_| sid.to_string())
        } else {
            sid.to_string()
        };
        for s in subtree_of(self.history.as_ref(), &guard) {
            if turn.is_running(&s) {
                return Err(Turn::running_refusal(&s));
            }
        }
        let (meta, events) = self.history_open(sid)?;
        // 代理会话没有名单可编辑（决定权整块交给核心）：如实说明。
        if meta.is_proxy() {
            return Err("代理会话没有可编辑的名单：它不是一个 agent 工作".to_string());
        }
        // 共享 guest 的子会话也可以编辑：写回该 agent 的唯一配置，保存后重新加入同一台 guest。
        // 旧 guest / 旧树的装配在下面与编辑后的树比对，只有装配真的变了才重建。
        let owner = config_owner_of(self.history.as_ref(), &meta)?;
        let old_scope = guest_scope(self.history.as_ref(), &meta)?;
        let old_tree = match &old_scope {
            Some((_, root)) => Some(tree_meta(self.history.as_ref(), root, None)?),
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
        // 校验：模块与模型真实存在；同一模块可以跨 agent 共用（各自独立沙箱与工具面）。
        let roster = self.modules.roster();
        let reserved = self.systools.reserved_agents();
        let mut metas: Vec<AgentMeta> = Vec::new();
        for a in &edit.agents {
            crate::units::pure::registry_model::api::validate_name(&a.name)?;
            crate::units::pure::registry_model::api::check_reserved(&a.name, &reserved)?;
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
            }
            if !a.model.is_empty() {
                if !registry.has_model(&a.model) {
                    return Err(format!("无此模型：{}", a.model));
                }
                registry.resolve(&a.model)?;
            }
            metas.push(AgentMeta::normalized(
                a.name.clone(),
                a.modules.clone(),
                Some(a.model.clone()),
                a.chara.clone(),
                !registry.has_agent(&a.name),
            ));
        }
        if metas.is_empty() {
            return Err("至少要有一个 agent".to_string());
        }
        if !meta.spec.driven() && metas.len() != 1 {
            return Err("没有流程的会话只接受一个 agent（模块数不限）".to_string());
        }
        // 子会话的名单是主名单的一份视图：只能改这一席的模块 / 模型 / 权限，不能在子会话里增删或改名。
        if owner.name != meta.name {
            let old: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
            let new: Vec<String> = metas.iter().map(|a| a.name.clone()).collect();
            if old != new {
                return Err("子会话不能改 agent 名单：要换人请改主会话或新建会话".to_string());
            }
        }
        // 执行环境：与「开始」同一把尺子——策略不认识、选型不成立（多版本未定版 / 定版不存在 / 路径冲突）如实拒绝。
        let policy = if edit.sandbox.trim().is_empty() {
            meta.sandbox.policy.clone()
        } else {
            edit.sandbox.trim().to_string()
        };
        let mut decl = crate::units::business::sandbox::api::SandboxDecl::of(&policy);
        decl.pins = edit.pins.clone();
        if let Some(g) = edit.guest.as_deref().filter(|g| !g.is_empty()) {
            decl.params = serde_json::json!({ "guest": g });
        }
        let facts = self.policies.facts(&decl)?;
        let target = decl.target(&facts);
        // 子会话的执行环境只随主会话：这里只锁策略与 guest，模块与模型仍按各自规则（后续再收口）。
        if meta.parent.is_some()
            && (decl.policy != meta.sandbox.policy || facts.guest != meta.facts.guest)
        {
            return Err(
                "子会话的执行环境随主会话：不能单独改 sandbox / guest（改主会话即可）".to_string(),
            );
        }
        // 承载校验：前置条件不具备时**不允许改入 guest 侧**（用户环境问题，不是选型问题）；界面上的"能不能选"由
        // SessionConfig 的 sandbox_ready 说同一件事。已在 guest 上的会话只校验它自己那几项（基础根等），不拦模块/模型/定版/网络。
        let staying_vm = meta.facts.on_guest() && facts.on_guest();
        if staying_vm {
            // 留在 vm 档：只校验用户这次填的基础根（填错路径就是填错路径），
            // 不拿"本机能不能提供 vm 档"去拦一条已经存在的会话。
            let base_item = self
                .policies
                .readiness(&decl, &target)
                .requirements
                .into_iter()
                .find(|r| r.id == "base");
            if let Some(item) = base_item
                && !item.met
            {
                return Err(format!("{}：{}", item.detail, item.how));
            }
        } else if let Some(why) = self.policies.refusal(&decl, &target) {
            return Err(why);
        }
        let module_ids = flat_modules(&metas);
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| module_ids.iter().any(|id| id == &m.manifest.id))
            .cloned()
            .collect();
        // 入口按**目标环境**判：虚拟机档在 guest 里跑（guest 的 os），本机档在宿主；
        // 跨平台时还要看文件形式——宿主编出来的产物（如 Windows 的 .exe）在 linux guest 里跑不了。
        let guest = crate::units::business::sandbox::api::guest_os_of(&self.root, host, &target)
            .map(|g| g.as_str());
        let target_os = guest.unwrap_or(std::env::consts::OS);
        let missing: Vec<crate::units::business::catalog::modules::api::MissingEntry> =
            crate::units::business::catalog::modules::api::missing_entries_for(
                &session_modules,
                target_os,
                &|p| {
                    host.is_executable(p)
                        && (target_os == std::env::consts::OS
                            || crate::units::business::catalog::modules::api::runs_on_target(
                                p, target_os,
                            ))
                },
            );
        crate::units::business::catalog::packages::api::plan_for_spec(
            &self.root,
            host,
            &self.packages.library(),
            &target,
            &session_modules,
            &missing,
            facts.guest.as_deref(),
            registry.app().sandbox.policy.network,
        )?;

        let mut new_meta = meta.clone();
        new_meta.modules = flat_modules(&metas);
        new_meta.agents = metas.clone();
        new_meta.sandbox = decl.clone();
        new_meta.facts = facts.clone();
        if let Some(p) = &edit.permission {
            if meta.parent.is_none() {
                // 根：只更新 ceiling；子会话的 narrowing 原样保留，出计划/装配时再求交。
                turn.permissions_for(registry, &meta)?.apply_root(p)?;
                new_meta.permission.ceiling = Some(p.clone());
                new_meta.permission.narrowing = None;
            } else {
                // 子：只能收窄；父的生效权限是校验基线。
                let parent = self
                    .history
                    .meta(meta.parent.as_deref().unwrap_or_default())?;
                turn.permissions_for(registry, &parent)?
                    .apply_narrowing(p)?;
                new_meta.permission.narrowing = Some(p.clone());
                new_meta.permission.ceiling = None;
            }
        }
        if let Some(s) = &edit.shared {
            if meta.parent.is_none() {
                // 根：读 / 写范围整体覆盖，黑名单追加；先按当前生效范围校验写法。
                turn.shared_scope_for(registry, &meta)?.apply_root(s)?;
            } else {
                // 子：只能收窄；父的生效范围是校验基线。
                let parent = self
                    .history
                    .meta(meta.parent.as_deref().unwrap_or_default())?;
                turn.shared_scope_for(registry, &parent)?
                    .apply_narrowing(s)?;
            }
            // 会话只搬声明：这一层覆盖进参数，怎么解释归工作台单元。
            new_meta.workspace.params = crate::units::business::workspace::api::edit_params(s)?;
        }
        // 同一 agent 全树只有一份配置：子会话编辑写回所属名单；主会话 / proxy 子工作编辑同步整棵子树。
        let mut new_children: Vec<SessionMeta> = Vec::new();
        if owner.name != meta.name {
            let mut new_owner = owner.clone();
            for a in &metas {
                if let Some(slot) = new_owner.agents.iter_mut().find(|x| x.name == a.name) {
                    *slot = a.clone();
                }
            }
            new_owner.modules = flat_modules(&new_owner.agents);
            new_children.push(new_owner);
        } else {
            for child in subtree_of(self.history.as_ref(), sid) {
                if child == sid {
                    continue;
                }
                let mut cm = self.history.meta(&child)?;
                cm.sandbox = decl.clone();
                cm.facts = facts.clone();
                for ca in &mut cm.agents {
                    if let Some(na) = new_meta.agents.iter().find(|a| a.name == ca.name) {
                        *ca = na.clone();
                    }
                }
                cm.modules = flat_modules(&cm.agents);
                new_children.push(cm);
            }
        }
        let was_vm = meta.facts.on_guest();
        let is_vm = new_meta.facts.on_guest();
        // 共享 guest 的装配按整棵树算：把这次的新值覆盖到作用域根的子树上，再出计划与沙箱。
        let (lease, tree, tree_plan) = if is_vm {
            let (lease, scope_meta) = guest_scope(self.history.as_ref(), &new_meta)?
                .ok_or_else(|| "guest 侧的会话没有解析出 guest 作用域".to_string())?;
            let mut extras: Vec<&SessionMeta> = vec![&new_meta];
            extras.extend(new_children.iter());
            let tree = tree_meta_with(self.history.as_ref(), &scope_meta, &extras)?;
            let plan = self.plan_for_meta_with(registry, host, &tree, &roster)?;
            self.log.info(
                "work::edit_session",
                &format!(
                    "sid={}；{}",
                    sid,
                    crate::units::business::catalog::packages::api::plan_summary(&plan)
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
            let sandboxes = turn.sandboxes(registry, host, tree, &roster)?;
            Some(EditGuest::Boot(Box::new(self.guest_boot(
                registry,
                probe,
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
                new_meta,
                new_children,
            },
            job,
        ))
    }

    /// 目的：用户快速切档——只改网络档位，共享区归属原样保留。
    /// 约束：根级动作；整棵 work tree 有会话在生成时拒绝；切换在下一次权限判定生效。
    pub(crate) fn set_permission_profile(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        sid: &str,
        profile: crate::units::pure::permission::api::ProfileId,
    ) -> Result<(), String> {
        let (meta, _) = self.history_open(sid)?;
        let guard = work_root(self.history.as_ref(), sid).unwrap_or_else(|_| sid.to_string());
        for s in subtree_of(self.history.as_ref(), &guard) {
            if turn.is_running(&s) {
                return Err(Turn::running_refusal(&s));
            }
        }
        let mut layer = if meta.parent.is_none() {
            meta.permission.ceiling.clone()
        } else {
            meta.permission.narrowing.clone()
        }
        .unwrap_or_default();
        layer.profile = Some(profile);
        if meta.parent.is_none() {
            turn.permissions_for(registry, &meta)?.apply_root(&layer)?;
        } else {
            let parent = self
                .history
                .meta(meta.parent.as_deref().unwrap_or_default())?;
            turn.permissions_for(registry, &parent)?
                .apply_narrowing(&layer)?;
        }
        let mut new_meta = meta.clone();
        if meta.parent.is_none() {
            new_meta.permission.ceiling = Some(layer);
            new_meta.permission.narrowing = None;
        } else {
            new_meta.permission.narrowing = Some(layer);
            new_meta.permission.ceiling = None;
        }
        let changed = subtree_of(self.history.as_ref(), sid);
        self.history.save_meta(&new_meta)?;
        self.record_config(turn, registry, sid, &new_meta);
        for s in &changed {
            self.release_fences(s);
        }
        for s in changed {
            turn.take(&s);
        }
        Ok(())
    }

    /// 目的：改配的第二段（核心）——写回 meta、记旁路配置、丢掉内存会话。
    pub(crate) fn commit_edit(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        prepared: PreparedEdit,
    ) -> Result<(), String> {
        let PreparedEdit {
            sid,
            new_meta,
            new_children,
            ..
        } = prepared;
        self.history.save_meta(&new_meta)?;
        for child in &new_children {
            self.history.save_meta(child)?;
        }
        // 权限 / 模块可能变了：按会话租约撤掉旧配置写下的授权与 profile；下一次调用按新 reach 惰性重授。
        self.release_fences(&sid);
        for child in &new_children {
            self.release_fences(&child.name);
        }
        self.record_config(turn, registry, &sid, &new_meta);
        // 内存里那份是按旧配置装的：丢掉主会话与整棵子树，下一次访问按新配置从转录重建。
        turn.take(&sid);
        for child in &new_children {
            turn.take(&child.name);
        }
        Ok(())
    }

    /// 目的：按会话租约撤掉围栏授权与 profile（权限 / 模块变更、会话删除时调用）。
    /// 约束：失败如实记日志，不阻断改配；残留由下一次启动对账按死归属回收。
    pub(crate) fn release_fences(&self, lease: &str) {
        if let Err(e) = self.tools.release_by_lease(lease) {
            self.log.warn(
                "work::release_fences",
                &format!("按会话撤销围栏授权未完成（{}）：{}", lease, e),
            );
        }
    }

    /// 目的：追加一条旁路配置记录：只作呈现与审计（不进模型上下文，回放与状态派生都跳过它）。
    pub(crate) fn record_config(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        sid: &str,
        meta: &SessionMeta,
    ) {
        let ev = serde_json::json!({
            "type": "config",
            "ts": crate::kernel::api::procs::unix_seconds(),
            "kind": meta.spec.kind,
            "sandbox": meta.sandbox.policy,
            "side": meta.facts.side.as_str(),
            "guest": meta.facts.guest,
            "net": registry.app().sandbox.policy.network,
            "pins": meta.sandbox.pins,
            "thinking": meta.thinking,
            "context_freshness": meta.context_freshness,
            "freshness_keep_percent": meta.freshness_keep_percent,
            "permission": serde_json::to_value(&meta.permission).unwrap_or(serde_json::Value::Null),
            "permission_effective": turn.permissions_for(registry, meta).ok().map(|p| serde_json::json!({
                "profile": p.profile,
                "shared": turn.shared_scope_for(registry, meta).ok(),
            })),
            "agents": meta.agents.iter().map(|a| serde_json::json!({
                "name": a.name,
                "modules": a.modules,
                "model": a.model,
            })).collect::<Vec<_>>(),
        });
        let content = crate::units::business::transcript::api::Content::of_events(&[ev]);
        if let Err(e) = self.history.append(sid, &content) {
            self.log
                .warn("work::record_config", &format!("配置记录落盘失败：{}", e));
        }
    }
}
