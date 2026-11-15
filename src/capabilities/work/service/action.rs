//! 目的：动作的核心实现——建会话（人经呈现层与核心代理共用的唯一一条）。
//! 管：把规整后的建会话载荷变成一个真正的会话（名字 / 档位 / 名单的规整）、环境解析与两段启用、代理形态工作的会话产出。
//! 不管：会话装配本身（`create.rs` 的 `create_work_inner`）；生成怎么跑（在 `conductor` 的句柄层）。
//! 联动：载荷形状见 `proxy/domain/proxy.rs`；声明与 callers 在 `systools/tools.yaml`。

use super::*;
use crate::capabilities::proxy::api as d;
use crate::capabilities::session::api::Delegation;
use crate::capabilities::work::service::guest_job::GuestBoot;
use crate::kernel::api::{ExecSpec, Tier, now_ts};

impl Work {
    /// 目的：**建会话的唯一实现**——父会话由 `spec.parent` 给出（None = 顶层会话，人经呈现层建的）。
    /// 参数：`spec` 是规整后的载荷（名字 / 档位 / 名单 / 本次需求）。
    /// 返回：建出的会话与名单 + 开场事实（事实由 api 层发布到事件台）。
    /// 错误：形态、名字、档位或名单不成立时如实拒绝，不留半成品。
    #[cfg(test)]
    pub(crate) fn create_session(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        spec: &d::NewSession,
    ) -> Result<(d::Created, Vec<SessionEvent>), String> {
        let (prepared, boot) = self.prepare_session(turn, registry, spec)?;
        if let Some(boot) = boot
            && let Err(e) = boot.run()
        {
            self.abort_session(turn, prepared);
            return Err(e);
        }
        self.commit_session(turn, registry, prepared)
    }

    /// 目的：规整一次建会话的**目标环境**（档位 / 虚拟根）：有父沿用父会话的执行段，顶层取载荷或设置默认。
    /// 参数：`spec` = 规整后的载荷。
    /// 返回：(档位, 虚拟根名)。
    /// 错误：档位名不合法。
    /// 约束：与准备段同一份判据（准备段也调它）——创建前的补编因此按同一个目标环境编。
    pub(crate) fn resolve_env(
        &self,
        registry: &dyn Registry,
        spec: &d::NewSession,
    ) -> Result<(Tier, Option<String>), String> {
        let parent_exec = match &spec.parent {
            Some(p) => Some(self.history.meta(p)?.exec),
            None => None,
        };
        let tier = match &parent_exec {
            Some(exec) => exec.tier,
            None => match spec
                .tier
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                Some(t) => Tier::parse(t)?,
                None => registry.app_settings().tier,
            },
        };
        let guest = match &parent_exec {
            Some(exec) => exec.guest.clone(),
            None => spec
                .guest
                .as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .map(|b| b.to_string()),
        };
        Ok((tier, guest))
    }

    /// 目的：建会话的第一段（核心，毫秒级）——规整载荷、占名、备好工作区与计划。
    /// 返回：准备结果 + 可选的那段"要等 guest"的活（虚拟机档才有）。
    pub(crate) fn prepare_session(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        spec: &d::NewSession,
    ) -> Result<(PreparedSession, Option<GuestBoot>), String> {
        let mode = match spec.mode {
            d::SessionMode::Single => WorkMode::Single,
            d::SessionMode::Collab => WorkMode::Collab,
            d::SessionMode::Proxy => WorkMode::Proxy,
        };
        let parent = spec.parent.clone();
        if mode == WorkMode::Proxy && parent.is_some() {
            return Err("代理不能往里套代理".to_string());
        }
        let agents: Vec<AgentInstance> = spec
            .agents
            .iter()
            .map(|a| AgentInstance {
                name: a.name.clone(),
                transient: a.transient,
                modules: a.modules.clone(),
                model: a.model.clone(),
                chara: a.chara.clone(),
            })
            .collect();
        // 名字：点名就用它（撞名加尾号），否则派生（子会话按父 + agent 名，顶层按 agent 名）。
        let base = spec
            .agents
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| format!("w-{}", spec.request_id));
        let derived = match &parent {
            Some(p) => format!("{}--{}", p, base),
            None => base,
        };
        let name = match spec
            .name
            .as_ref()
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
        {
            Some(n) => self.unique_work_name(turn, n, &derived),
            None => self.unique_work_name(turn, &derived, "work"),
        };
        // 环境属于主会话：有父就整段沿用父会话的 ExecSpec（档位 / 虚拟根 / 网络 / 定版），
        // 不给子会话留任何单独覆盖的入口。顶层仍由人经呈现层给档位与虚拟根。
        let (tier, vm_guest) = self.resolve_env(registry, spec)?;
        // 本次需求：协作必须有；单 agent 的"开头"由调用方在开工那一刻给（见 create_session_call）。
        let task = if mode == WorkMode::Collab {
            let t = spec.task.trim();
            if t.is_empty() {
                return Err("协作模式必须填写本次需求（task）".to_string());
            }
            Some(t.to_string())
        } else {
            None
        };
        // 委托代拟由形态派生：协作、又没给名单 = 交给核心按需求拟名单（不再是一个参数）。
        let delegate = mode == WorkMode::Collab && agents.is_empty();
        let work = WorkSpec {
            name,
            mode,
            agents,
            task,
            delegate,
            tier,
            permission: spec.permission,
            guest: vm_guest,
            workspace: spec
                .workspace
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from),
            workspace_grant: spec
                .workspace_grant
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(crate::capabilities::session::api::WorkspaceGrant::parse)
                .transpose()?,
        };
        // 有父 = 子工作（`create_work_inner` 直接给 parent）；没有父 = 顶层会话，走同一个入口包装。
        let prepared = match parent.as_deref() {
            Some(p) => self.prepare_work(turn, registry, work, Some(p))?,
            None => self.prepare_work(turn, registry, work, None)?,
        };
        Ok((PreparedSession { work: prepared.0 }, prepared.1))
    }

    /// 目的：建会话的第二段（核心）——装配会话对象、落盘身份、开场事实。
    pub(crate) fn commit_session(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        prepared: PreparedSession,
    ) -> Result<(d::Created, Vec<SessionEvent>), String> {
        let opened = self.commit_work(turn, registry, prepared.work)?;
        Ok((
            d::Created {
                session: opened.sid,
                agents: opened.agents,
            },
            opened.facts,
        ))
    }

    /// 目的：建会话失败时撤销准备段（占名与空工作区目录）。
    pub(crate) fn abort_session(&mut self, turn: &mut Turn, prepared: PreparedSession) {
        self.abort_work(turn, prepared.work);
    }
}

/// 目的：一次建会话的准备结果（两段之间的交接物）。
pub(crate) struct PreparedSession {
    work: super::PreparedWork,
}

impl Work {
    /// 目的：建代理会话的真身：返回（会话引用, **开场事实**）——事实交给 api 层发布到事件台
    ///  （核心不持有事件台；与 `create_work` 同一条口径）。
    ///  `tier` 是用户在向导里选的档位：它既是这条会话自己的工作区档位，也是它**建子工作时的默认档位**
    ///  （`proxy_create` 取 `pmeta.exec.tier`）。承载不了就如实拒绝，不偷偷降级。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_proxy_with_facts(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        name: &str,
        granted_at: i64,
        tier: crate::kernel::api::Tier,
        guest: Option<&str>,
        profile: crate::capabilities::permission::api::ProfileId,
        workspace: Option<crate::capabilities::session::api::WorkspaceBinding>,
    ) -> Result<(String, Vec<SessionEvent>), String> {
        validate_work_name(name)?;
        if turn.holds(name) || self.history.load(name).is_ok() {
            return Err(format!("工作名已存在：{}", name));
        }
        let exec = ExecSpec {
            tier,
            guest: guest
                .map(|g| g.trim().to_string())
                .filter(|g| !g.is_empty()),
            ..ExecSpec::default()
        };
        // 与其余形态同一把尺子：虚拟机档的承载不成立就不建（用户环境问题，不是选型问题）。
        if let Some(why) = crate::capabilities::vm::api::tier_refusal(
            &self.root,
            &exec,
            self.qemu_path(registry).as_deref(),
            self.share_tool_path(registry).as_deref(),
            self.probe.as_ref(),
        ) {
            return Err(why);
        }
        // 先解析核心通道：没有模型就不建（工作区都还没有，失败什么都不留）。
        let channel = registry.core_channel()?;
        let meta = SessionMeta {
            context_freshness: false,
            name: name.to_string(),
            mode: "proxy".to_string(),
            delegate: false,
            modules: Vec::new(),
            task: None,
            ts: now_ts(),
            agents: Vec::new(),
            exec,
            sandbox: crate::capabilities::permission::api::SessionSandbox {
                ceiling: Some(crate::capabilities::permission::api::PermissionLayer {
                    profile: Some(profile),
                    ..Default::default()
                }),
                narrowing: None,
            },
            parent: None,
            node: None,
            delegation: Some(Delegation { granted_at }),
            thinking: None,
            workspace: workspace.clone(),
            run: RunState::Active,
        };
        self.workspace
            .bind(name, workspace.as_ref().map(|w| w.root.as_path()))?;
        self.workspace.prepare(
            name,
            &[crate::capabilities::turn::api::PROXY_PRIVATE_DIR.to_string()],
        )?;
        if workspace.is_some()
            && let Err(e) = self.workspace.work_import_root(name, now_ts())
        {
            self.log
                .warn("work::create_proxy", &format!("工作区初始导入失败：{}", e));
        }
        let mut session = turn.build_proxy_with(&meta, &channel, registry, &*self.handlers)?;
        let mut events = Vec::new();
        // 授予事实既是 meta 的 `delegation`（可重建），也留一条**可回放记录**：
        // 打开这条会话的人应当看得见"决定权是整块交出去的"，而不是只能去读 meta。
        events
            .extend(session.note_notice("[授予] 用户选定了代理形态——决定权整块交给核心（全权）。"));
        self.history.create(&meta)?;
        turn.put(name, Session::Single(session));
        self.record_events(name, &mut events);
        Ok((name.to_string(), events))
    }
}
