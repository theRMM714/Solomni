//! 目的：会话配置的读取、改配两段与权限档提交——配置视图、改配准备、提交落盘。
//! 管：配置视图、改配准备（新 meta 与子树上浮）、提交落盘、围栏撤权、权限档写入、配置记录。
//! 不管：建工作与 guest 启动（`create.rs`）、报告与档位（`report.rs`）。
//! 联动：api 层的配置接口与 `work_tools` 经这里调用；落盘走 history 与 workspace。
//!   本方法族只碰 probe / history / sessions / log 这四个字段。

use super::*;

impl Conductor {
    /// 目的：配置视图：把「能改什么、现在是什么、缺什么」如实给出（每次读取都重扫模块清单与包库）。
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
                    chara: a.chara.clone(),
                })
                .collect(),
            tier: tier.as_str().to_string(),
            tier_locked: meta.parent.is_some(),
            guest: meta.exec.guest.clone(),
            guests: self.guest_bases()?,
            net: self.network_allowed(),
            permission: self.permission_view(&meta)?,
            thinking: self
                .thinking_view_for_root(&self.work_root(sid).unwrap_or_else(|_| sid.to_string())),
            pins: meta.exec.pins.clone(),
            runtime: self.runtime_report_for(&meta.exec),
            runtimes_dir: crate::kernel::api::slash(&self.packages.runtimes_dir()),
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
            // 失败什么都不改：meta 还没落盘，现有会话原样还在。
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
        // 权限是根级动作：切档要求整棵 work tree 空闲，避免子会话用旧快照继续跑。
        let guard = if edit.permission.is_some() {
            self.work_root(sid).unwrap_or_else(|_| sid.to_string())
        } else {
            sid.to_string()
        };
        for s in self.subtree_of(&guard) {
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
                ));
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
        // 校验：模块与模型真实存在；同一模块可以跨 agent 共用（各自独立沙箱与工具面）。
        let roster = self.scan();
        let reserved = self.reserved_names();
        let mut metas: Vec<AgentMeta> = Vec::new();
        if let Some(p) = &edit.permission {
            crate::capabilities::permission::api::validate_layer(p)?;
        }
        for a in &edit.agents {
            crate::capabilities::registry::api::validate_name(&a.name)?;
            crate::capabilities::registry::api::check_reserved(&a.name, &reserved)?;
            for id in &a.modules {
                if !roster.modules.iter().any(|m| &m.manifest.id == id) {
                    return Err(format!("无此模块：{}", id));
                }
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
                modules: dedup_modules(&a.modules),
                model: if a.model.is_empty() {
                    None
                } else {
                    Some(a.model.clone())
                },
                chara: a.chara.trim().to_string(),
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
        };
        // 子会话的档位只随主会话：这里只锁 tier / guest，模块与模型仍按各自规则（后续再收口）。
        if meta.parent.is_some() && (spec.tier != meta.exec.tier || spec.guest != meta.exec.guest) {
            return Err(
                "子会话的档位随主会话：不能单独改 tier / guest（改主会话即可）".to_string(),
            );
        }
        // 承载校验：前置条件不具备时**不允许改入虚拟机档**（用户环境问题，不是选型问题）；界面上的"能不能选"由
        // SessionConfig 的 tier_ready 说同一件事。已在虚拟机档上的会话只校验它自己那几项（基础根等），不拦模块/模型/定版/网络。
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
            if let Some(item) = base_item
                && !item.met
            {
                return Err(format!("{}：{}", item.detail, item.how));
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
        let module_ids = flat_modules(&metas);
        let session_modules: Vec<Module> = roster
            .modules
            .iter()
            .filter(|m| module_ids.iter().any(|id| id == &m.manifest.id))
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
        self.plan_for_spec(&spec, &session_modules, &missing, spec.guest.as_deref())?;

        let mut new_meta = meta.clone();
        new_meta.modules = flat_modules(&metas);
        new_meta.agents = metas.clone();
        new_meta.exec = spec.clone();
        if let Some(p) = &edit.permission {
            if meta.parent.is_none() {
                // 根：只更新 ceiling；子会话的 narrowing 原样保留，出计划/装配时再求交。
                self.permissions_for(&meta)?.apply_root(p)?;
                new_meta.sandbox.ceiling = Some(p.clone());
                new_meta.sandbox.narrowing = None;
            } else {
                // 子：只能收窄；父的生效权限是校验基线。
                let parent = self
                    .history
                    .meta(meta.parent.as_deref().unwrap_or_default())?;
                self.permissions_for(&parent)?.apply_narrowing(p)?;
                new_meta.sandbox.narrowing = Some(p.clone());
                new_meta.sandbox.ceiling = None;
            }
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
            for child in self.subtree_of(sid) {
                if child == sid {
                    continue;
                }
                let mut cm = self.history.meta(&child)?;
                cm.exec = spec.clone();
                for ca in &mut cm.agents {
                    if let Some(na) = new_meta.agents.iter().find(|a| a.name == ca.name) {
                        *ca = na.clone();
                    }
                }
                cm.modules = flat_modules(&cm.agents);
                new_children.push(cm);
            }
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
                new_meta,
                new_children,
            },
            job,
        ))
    }

    /// 目的：用户快速切档——只改 profile（重置 reach / decisions / 逐工具决定），保留共享区范围与黑名单。
    /// 约束：根级动作；整棵 work tree 有会话在生成时拒绝；切换在下一次权限判定生效。
    pub(crate) fn set_permission_profile(
        &mut self,
        sid: &str,
        profile: crate::capabilities::permission::api::ProfileId,
    ) -> Result<(), String> {
        let (meta, _) = self.history_open(sid)?;
        let guard = self.work_root(sid).unwrap_or_else(|_| sid.to_string());
        for s in self.subtree_of(&guard) {
            if self.running.contains(&s) {
                return Err(Self::running_refusal(&s));
            }
        }
        let mut layer = if meta.parent.is_none() {
            meta.sandbox.ceiling.clone()
        } else {
            meta.sandbox.narrowing.clone()
        }
        .unwrap_or_default();
        layer.profile = Some(profile);
        layer.reach = None;
        layer.decisions = None;
        layer.tools.clear();
        if meta.parent.is_none() {
            self.permissions_for(&meta)?.apply_root(&layer)?;
        } else {
            let parent = self
                .history
                .meta(meta.parent.as_deref().unwrap_or_default())?;
            self.permissions_for(&parent)?.apply_narrowing(&layer)?;
        }
        let mut new_meta = meta.clone();
        if meta.parent.is_none() {
            new_meta.sandbox.ceiling = Some(layer);
            new_meta.sandbox.narrowing = None;
        } else {
            new_meta.sandbox.narrowing = Some(layer);
            new_meta.sandbox.ceiling = None;
        }
        let changed = self.subtree_of(sid);
        self.history.save_meta(&new_meta)?;
        self.record_config(sid, &new_meta);
        for s in &changed {
            self.release_fences(s);
        }
        for s in changed {
            self.sessions.remove(&s);
        }
        Ok(())
    }

    /// 目的：改配的第二段（核心）——写回 meta、记旁路配置、丢掉内存会话。
    pub(crate) fn commit_edit(&mut self, prepared: PreparedEdit) -> Result<(), String> {
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
        self.record_config(&sid, &new_meta);
        // 内存里那份是按旧配置装的：丢掉主会话与整棵子树，下一次访问按新配置从转录重建。
        self.sessions.remove(&sid);
        for child in &new_children {
            self.sessions.remove(&child.name);
        }
        Ok(())
    }

    /// 目的：按会话租约撤掉围栏授权与 profile（权限 / 模块变更、会话删除时调用）。
    /// 约束：失败如实记日志，不阻断改配；残留由下一次启动对账按死归属回收。
    pub(crate) fn release_fences(&self, lease: &str) {
        if let Err(e) = self.tools.release_by_lease(lease) {
            self.log.warn(
                "conductor::release_fences",
                &format!("按会话撤销围栏授权未完成（{}）：{}", lease, e),
            );
        }
    }

    /// 目的：追加一条旁路配置记录：只作呈现与审计（不进模型上下文，回放与状态派生都跳过它）。
    pub(crate) fn record_config(&self, sid: &str, meta: &SessionMeta) {
        let ev = serde_json::json!({
            "type": "config",
            "ts": now_ts(),
            "mode": meta.mode,
            "tier": meta.exec.tier.as_str(),
            "guest": meta.exec.guest,
            "net": self.network_allowed(),
            "pins": meta.exec.pins,
            "thinking": meta.thinking,
            "context_freshness": meta.context_freshness,
            "permission": serde_json::to_value(&meta.sandbox).unwrap_or(serde_json::Value::Null),
            "permission_effective": self.permissions_for(meta).ok().map(|p| serde_json::json!({
                "profile": p.profile,
                "reach": p.reach,
                "decisions": p.decisions,
                "shared": { "read": p.shared_read, "write": p.shared_write, "deny": p.deny },
                "tools": p.tools,
            })),
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
}
