//! 目的：会话装配要取的**事实**——共享 guest 作用域与租约、沙箱清单、权限面、上下文预算。
//! 管：从 registry / history / workspace 取事实后转调 `env` 能力派生沙箱清单 / 权限面 / 围栏 / 进程环境 / 租约。
//! 不管：一席工具面与单 agent 会话对象怎么装（归 `turn` 的装配构件）；会话怎么落盘（history.rs）、
//!   协作链怎么推进（flow.rs）、建工作的两段（work/create.rs）。
//! 联动：建工作与 turn 的准备经这里取事实；装配本身在 `turn::api`。
use super::*;
use std::collections::BTreeMap;

impl Conductor {
    /// 目的：这台会话的**共享 guest 作用域**——(guest 租约, 该租约对应的会话 meta)。
    /// 约束：一个主会话树一台 guest；proxy 父会话没有 guest，它的每个子工作临时各占一台。
    ///   host 档 / proxy 父会话 = None；其余 = (租约, 作用域根 meta)。
    pub(crate) fn guest_scope(
        &self,
        meta: &SessionMeta,
    ) -> Result<Option<(String, SessionMeta)>, String> {
        if meta.exec.tier != crate::kernel::api::Tier::Vm {
            return Ok(None);
        }
        // 顶层工作名：子会话沿 parent 走到顶（整棵树只有一个 work/）。
        let root = self.work_root_of(meta)?;
        let top = if root == meta.name {
            meta.clone()
        } else {
            self.history.meta(&root)?
        };
        // proxy 父会话没有 guest；其余（单 agent / 协作 / 代理子工作）共用顶层那台的租约。
        Ok(crate::capabilities::env::api::guest_lease(meta, &top).map(|lease| (lease, top)))
    }

    /// 目的：这台会话的工具执行要用的 guest 租约（空 = 本机档 / 没有 guest）。
    pub(crate) fn guest_lease_for(&self, meta: &SessionMeta) -> Result<String, String> {
        Ok(self
            .guest_scope(meta)?
            .map(|(lease, _)| lease)
            .unwrap_or_default())
    }

    /// 目的：一次 guest 只读诊断的准备（毫秒级）：定位这条会话的 guest 租约。
    /// 约束：本机档没有 guest；代理父会话自己没有 guest，要诊断它下面的某个工作。
    pub(crate) fn prepare_guest_diagnose(
        &self,
        sid: &str,
    ) -> Result<super::guest_job::GuestDiagnose, String> {
        let meta = self.history.meta(sid)?;
        if meta.exec.tier != crate::kernel::api::Tier::Vm {
            return Err("这条会话是本机档，没有 guest 可诊断".to_string());
        }
        match self.guest_scope(&meta)? {
            Some((lease, _)) => Ok(super::guest_job::GuestDiagnose::new(
                Arc::clone(&self.vm),
                lease,
            )),
            None if meta.mode == "proxy" && meta.parent.is_none() => {
                Err("代理父会话本身没有 guest，请对它下面某个工作诊断".to_string())
            }
            None => Err("这条会话没有可诊断的 guest".to_string()),
        }
    }

    /// 目的：一个 agent 的配置归属哪份名单：协作成员会话的名单在主会话，
    ///   proxy 子工作（父是代理）或顶层会话自己就是名单所有者；嵌套协作继续向上找。
    /// 约束：同一 agent 全树只有一份配置——子会话编辑写回这里，不留两份会漂的 meta。
    pub(crate) fn config_owner_of(&self, meta: &SessionMeta) -> Result<SessionMeta, String> {
        let mut owner = meta.clone();
        while let Some(p) = owner.parent.clone() {
            let pm = self.history.meta(&p)?;
            if pm.mode == "collab" {
                owner = pm;
            } else {
                break;
            }
        }
        Ok(owner)
    }

    /// 目的：一台共享 guest 的**树级装配 meta**——把作用域根与其整棵子树的 agent 并成一份。
    /// 约束：同一 agent 名只留一份（后代覆盖祖先，便于子会话编辑后按新值装配）；exec / 形态 / 工作名取作用域根。
    ///   extra 用于"还没落盘"的待建子会话（建工作与改配的准备段要它）。
    pub(crate) fn tree_meta(
        &self,
        root: &SessionMeta,
        extra: Option<&SessionMeta>,
    ) -> Result<SessionMeta, String> {
        let extras: Vec<&SessionMeta> = extra.into_iter().collect();
        self.tree_meta_with(root, &extras)
    }

    /// 目的：同上，但是可带多份尚未落盘的 meta（改配时主会话与子树的新值要一起参与装配）。
    pub(crate) fn tree_meta_with(
        &self,
        root: &SessionMeta,
        extra: &[&SessionMeta],
    ) -> Result<SessionMeta, String> {
        let mut agents: BTreeMap<String, crate::capabilities::session::api::AgentMeta> =
            BTreeMap::new();
        let mut push = |m: &SessionMeta| {
            for a in &m.agents {
                agents.insert(a.name.clone(), a.clone());
            }
        };
        push(root);
        for sid in self.subtree_of(&root.name) {
            if sid == root.name {
                continue;
            }
            if let Ok(m) = self.history.meta(&sid) {
                push(&m);
            }
        }
        for m in extra {
            push(m);
        }
        let mut out = root.clone();
        out.agents = agents.into_values().collect();
        out.modules = super::flat_modules(&out.agents);
        Ok(out)
    }

    /// 目的：组装本次工作的沙箱清单：工作根来自 Workspace 端口，模块目录来自清单。
    ///  权限策略在此收口：模块目录只对使用它的 agent 可达；同一模块可被多个 agent 共用（各自独立沙箱与工具面）。
    ///  每个 agent 的沙箱按 meta.agents 建；共享区根任何时候都有（代拟还没名单时也有，@ 改写要用）。
    pub(crate) fn sandboxes(
        &self,
        meta: &SessionMeta,
        roster: &crate::capabilities::modules::api::Roster,
    ) -> Result<crate::capabilities::workspace::api::Sandboxes, String> {
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        // 沙箱锚在**顶层工作**上：整棵树不论嵌套多少层只有一个 work/（见 Conductor::work_root）。
        let work = self.work_root_of(meta)?;
        let roots = self.workspace.roots(&work, &names)?;
        // 虚拟机档给 guest↔宿主翻译器：system 工具跑在宿主侧，但收模型给的 guest 写法；
        // **同一份**也交给 @ 改写（挂载点、命令装配与用户引用用同一座桥，三处不会各给一种写法）。
        let codec = self.path_codec_of(&meta.exec);
        // 外部工作区的授权粒度：只影响围栏派生（共享区是否要每会话重授），不改权限面。
        let grant = self.workspace_grant(meta)?;
        crate::capabilities::env::api::sandboxes(
            &crate::capabilities::env::api::SandboxesFacts {
                work: &work,
                session: &meta.name,
                agents: &meta.agents,
                roots: &roots,
                roster,
                codec,
                grant,
            },
            &|agent| self.permission_for_agent(meta, agent),
        )
    }

    /// 目的：一条会话的权限链——根在前，自己最后（沿 parent 走到顶）。
    fn permission_chain(&self, meta: &SessionMeta) -> Result<Vec<SessionMeta>, String> {
        let mut chain = vec![meta.clone()];
        let mut cur = meta.clone();
        while let Some(parent) = cur.parent.clone() {
            let pm = self.history.meta(&parent)?;
            chain.push(pm.clone());
            cur = pm;
        }
        chain.reverse();
        Ok(chain)
    }

    /// 目的：一条会话的**生效权限**：全局默认 → 根 ceiling → 每层 narrowing → 全局硬黑名单。
    pub(crate) fn permissions_for(
        &self,
        meta: &SessionMeta,
    ) -> Result<crate::capabilities::permission::api::PermissionSet, String> {
        let chain = self.permission_chain(meta)?;
        crate::capabilities::env::api::permissions(&chain, self.registry.app())
    }

    /// 目的：这条会话生效的外部工作区授权粒度：沿权限链找最近一份绑定（创建时只写在根），
    ///   没有绑定就是 `session` 缺省。
    pub(crate) fn workspace_grant(
        &self,
        meta: &SessionMeta,
    ) -> Result<crate::capabilities::session::api::WorkspaceGrant, String> {
        let chain = self.permission_chain(meta)?;
        Ok(crate::capabilities::env::api::workspace_grant(&chain))
    }

    /// 目的：某个 agent 的生效权限：协作主会话里的发言席优先取它自己的子会话 narrowing。
    fn permission_for_agent(
        &self,
        meta: &SessionMeta,
        agent: &str,
    ) -> Result<crate::capabilities::permission::api::PermissionSet, String> {
        if meta.agents.len() == 1 && meta.agents[0].name == agent {
            return self.permissions_for(meta);
        }
        let child_name = format!("{}--{}", meta.name, agent);
        if let Ok(child) = self.history.meta(&child_name)
            && child.parent.as_deref() == Some(meta.name.as_str())
        {
            return self.permissions_for(&child);
        }
        self.permissions_for(meta)
    }

    /// 目的：核心自有的**共享区版本化**工具执行者——只在角色面确实发到它们时才装。
    ///   约束：与代理工具同一套路：成员循环只问"谁认领这个名字"，核心自有工具不是循环里的特例。
    pub(crate) fn work_handlers(
        &self,
        sb: &crate::capabilities::workspace::api::Sandbox,
        allowed: &[String],
    ) -> Vec<Arc<dyn crate::kernel::ports::ToolHandler>> {
        use crate::capabilities::conductor::domain::work as dw;
        if !allowed.iter().any(|t| dw::is_work_tool(t)) {
            return Vec::new();
        }
        vec![Arc::new(
            crate::capabilities::conductor::service::work_tools::WorkHandler::new(
                Arc::clone(&self.workspace),
                &sb.work_name,
                &sb.agent,
                sb.permissions.clone(),
            ),
        )]
    }

    /// 目的：一轮内对同一个成员最多提醒几次（用户可设，见 session-model.md 二）。
    pub fn discuss_remind_cap(&self) -> u32 {
        self.registry.app().discuss_remind_cap
    }

    /// 目的：本会话的上下文预算：窗口按模型现算（缺省用核心默认），阈值与保留百分比来自全局设置。
    /// 约束：阈值 0% = 关自动压缩。见 docs/session/session-model.md 六。
    pub(crate) fn context_budget(&self, model: Option<&str>) -> (u64, u8, u8) {
        // 窗口是登记处的事实，不在这里另存一份。
        (
            self.registry.context_of(model),
            self.registry.app().compact_at_percent,
            self.registry.app().compact_keep_percent,
        )
    }

    /// 目的：把当前设置与模型窗口刷到一条活会话（设置改了不用重建会话）。
    /// 参数：`s` = 单 agent 会话；窗口按它自己的模型解析。
    pub(crate) fn refresh_context_budget(
        &self,
        s: &mut crate::capabilities::session::api::AgentSession,
    ) {
        let model = s.model_id().map(str::to_string);
        let (window, percent, keep) = self.context_budget(model.as_deref());
        s.set_context_budget(window, percent, keep);
    }
}
