//! **装配材料**：成员通道、沙箱清单、工具环境、预算与角色工具面、单 agent 会话对象（build_single）。
//!
//! 它是"把各能力的事实拼成一个能跑的会话"的那一层：纯派生，不落盘。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate)（或 pub）供兄弟族与 conductor/api.rs 调用。

use super::*;
use crate::capabilities::packages::api as pkgs;

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
        // proxy：父会话没有 guest；整棵代理树的子工作共用顶层那一台（租约 = 代理根名）。
        if top.mode == "proxy" {
            if meta.parent.is_none() {
                return Ok(None);
            }
            return Ok(Some((top.name.clone(), top)));
        }
        // 单 agent / 协作：整棵树共用顶层工作那台 guest。
        Ok(Some((top.name.clone(), top)))
    }

    /// 目的：这台会话的工具执行要用的 guest 租约（空 = 本机档 / 没有 guest）。
    pub(crate) fn guest_lease_for(&self, meta: &SessionMeta) -> Result<String, String> {
        Ok(self
            .guest_scope(meta)?
            .map(|(lease, _)| lease)
            .unwrap_or_default())
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

    /// 组装本次工作的沙箱清单：工作根来自 Workspace 端口，模块目录来自清单。
    /// 权限策略在此收口：模块目录只对使用它的 agent 可达；同一模块可被多个 agent 共用（各自独立沙箱与工具面）。
    /// 每个 agent 的沙箱按 meta.agents 建；共享区根任何时候都有（代拟还没名单时也有，@ 改写要用）。
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
        let path_codec = self.path_codec_of(&meta.exec);
        let mut list: Vec<crate::capabilities::workspace::api::Sandbox> = Vec::new();
        for a in &meta.agents {
            let private = roots
                .agents
                .get(&a.name)
                .cloned()
                .ok_or_else(|| format!("工作区没有给出 agent {} 的沙箱路径", a.name))?;
            let mut modules = BTreeMap::new();
            let mut modules_with_userdata = std::collections::BTreeSet::new();
            for id in &a.modules {
                if let Some(m) = roster.modules.iter().find(|m| &m.manifest.id == id) {
                    modules.insert(id.clone(), m.root.clone());
                    if m.has_userdata {
                        modules_with_userdata.insert(id.clone());
                    }
                }
            }
            list.push(crate::capabilities::workspace::api::Sandbox {
                work_name: work.clone(),
                session: meta.name.clone(),
                agent: a.name.clone(),
                shared: roots.shared.clone(),
                private,
                modules,
                modules_with_userdata,
                permissions: self.permission_for_agent(meta, &a.name)?,
                path_codec: path_codec.clone(),
            });
        }
        Ok(crate::capabilities::workspace::api::Sandboxes {
            shared: roots.shared,
            list,
            codec: path_codec,
        })
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
        let root = chain.first().ok_or_else(|| "会话权限链为空".to_string())?;
        let mut out = self.registry.app().sandbox.defaults.to_permission_set();
        if let Some(layer) = root.sandbox.root_layer()? {
            out = out.apply_root(layer)?;
        }
        for child in chain.iter().skip(1) {
            if let Some(layer) = child.sandbox.child_layer()? {
                out = out.apply_narrowing(layer)?;
            }
        }
        Ok(out.with_hard_deny(&self.registry.app().sandbox.policy.deny))
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
        if let Ok(child) = self.history.meta(&child_name) {
            if child.parent.as_deref() == Some(meta.name.as_str()) {
                return self.permissions_for(&child);
            }
        }
        self.permissions_for(meta)
    }

    /// 工具环境：内置文件工具永远可用；外部工具按模块分组放行（模块 id → 目录 + 工具表）。
    /// unavailable：本档位下缺运行包、不能执行工具的模块（机制侧据此拒绝执行，并如实报缺哪个能力）。
    /// net：会话是否放行出站网络（exec 段；默认否），随围栏交给机制层。
    // 会话装配把"这一席的工具面"一次说全（模块 / 沙箱 / 缺包 / 网络 / 形态 / 角色 / 档位）。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tools_env(
        &self,
        modules: &[Module],
        sb: &crate::capabilities::workspace::api::Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        mode: crate::capabilities::llm::api::ToolMode,
        // 这一席的身份：用户建的单 agent 会话 = solo，协作子会话 = executor（见 systools/roles.yaml）。
        role: &str,
        tier: crate::kernel::api::Tier,
        guest_lease: String,
        host_plan: &pkgs::PackagePlan,
    ) -> crate::capabilities::session::api::MemberTools {
        // 角色表发放的系统工具 id 清单（工具面的名字部分），再按权限档位过滤：
        // deny 的工具不发放（不是发放后再拒），ask 的照常发放。
        let book = self.systools.book();
        let allowed: Vec<String> = self
            .role_tools(role)
            .into_iter()
            .filter(|id| {
                book.get(id)
                    .map(|s| {
                        sb.permissions
                            .offers(id, s.effect(), &s.max_reach(), s.scope)
                    })
                    .unwrap_or(true)
            })
            .collect();
        let handlers = self.work_handlers(sb, &allowed);
        // 工具面：静态模块工具 + **已就绪**常驻服务的操作（操作是服务跑起来才发现的，按模块现填）。
        let with_modules = self.systools.allows_module_tools(role)
            && !sb.permissions.decisions.exec_sandbox.is_deny();
        let mut table = crate::capabilities::session::api::tool_table(modules);
        // 服务清单是全局的：只把**这个 agent 工具面里**的模块服务写进提示词。
        let started = if with_modules {
            crate::capabilities::session::api::service_ops_for_modules(
                self.started_service_ops(),
                modules,
            )
        } else {
            BTreeMap::new()
        };
        let mut module_tool_lines: Vec<String> = Vec::new();
        // 这些操作**自带参数契约**（服务给的 JSON Schema 原样）：信封通道也要把参数说清，
        // 否则模型只知道名字、只能瞎试参数（真实模型上真发生过）。
        let mut service_param_lines: Vec<String> = Vec::new();
        for (id, ops) in &started {
            if let Some(mt) = table.get_mut(id) {
                for op in ops {
                    mt.services.insert(op.tool.clone(), op.clone());
                }
            }
            let names = ops
                .iter()
                .map(|o| o.tool.clone())
                .collect::<Vec<_>>()
                .join("、");
            module_tool_lines.push(format!("- {}：{}", id, names));
            for op in ops {
                let Some(schema) = &op.parameters else {
                    continue;
                };
                let signature =
                    crate::capabilities::tools::api::json_schema_lines(schema).join("\n");
                if signature.is_empty() {
                    continue;
                }
                service_param_lines.push(format!("- {}.{}\n{}", id, op.tool, signature));
            }
        }
        let mut notes = crate::capabilities::tools::api::tool_notes(
            &*self.prompt,
            sb,
            if with_modules { modules } else { &[] },
            &service_param_lines,
        );
        if !module_tool_lines.is_empty() {
            notes.module_tools =
                format!("{}\n{}", notes.module_tools, module_tool_lines.join("\n"));
        }
        crate::capabilities::session::api::MemberTools {
            mode,
            modules: table,
            observations: crate::capabilities::tools::api::Observations::default(),
            llm: Arc::clone(&self.llm),
            log: Arc::clone(&self.log),
            tools: Arc::clone(&self.tools),
            tier,
            guest_lease,
            vm: Arc::clone(&self.vm),
            sandbox: sb.clone(),
            tools_this_turn: self
                .prompt
                .text(crate::capabilities::prompt::api::Segment::ToolsThisTurn)
                .to_string(),
            builtin_tools: self.systools.book(),

            unavailable,
            // 围栏：可达范围 + 断网 + 环境白名单的落点，全部由该 agent 的沙箱派生（机制在 adapters）；
            // 只读根来自用户显式授权（`sandbox.policy.host_read_roots`）与本机运行包的包根。
            fence: {
                let mut fence =
                    crate::kernel::api::FenceSpec::for_session(sb, &self.host_read_roots(), net);
                fence.ro_tree.extend(host_plan.roots.iter().cloned());
                fence.ro_tree.sort();
                fence.ro_tree.dedup();
                fence
            },
            // 各模块隐私字段的注入项：装配期按模块解析一次，起工具进程时只走环境（解析失败 = 该模块没有可用值）。
            module_env: modules
                .iter()
                .map(|m| {
                    (
                        m.manifest.id.clone(),
                        self.secrets.resolve(&m.manifest.id).unwrap_or_default(),
                    )
                })
                .collect(),
            runtime_path: host_plan.path.clone(),
            shell_live: Some(self.shell_slot(&sb.session)),
            runtime_roots: {
                // 基础工具链根只在宿主档放行：VM 的工具链由 guest 的只读挂载提供，宿主路径在 guest 里不存在。
                let mut roots = if tier == crate::kernel::api::Tier::Host {
                    self.base_roots.clone()
                } else {
                    Vec::new()
                };
                roots.extend(host_plan.roots.iter().cloned());
                roots.sort();
                roots.dedup();
                roots
            },
            // 从零开始；按落盘转录重建时由调用方按转录里的最大值续号（见 rebuild_session）。
            reply_seq: 0,
            line: Default::default(),
            // 这一席的系统工具面**由角色表发放**（越权校验的唯一判据）：给什么写什么，代码里不留第二份名单。
            allowed,
            role: role.to_string(),
            // 能不能用自己模块的工具由角色表发放；说明块素材在装配期算好（服务操作已在上面并进来）。
            with_modules,
            notes,
            handlers,
            // 常驻服务的统一管理面：模块的 services 操作经它调用。
            residents: Arc::clone(&self.residents),
        }
    }

    /// 目的：**已就绪**常驻服务的操作，按模块分组（模块 → 该模块的服务操作）。
    /// 约束：只收已就绪的服务——没启动的没有操作可发现，不该出现在模型工具面里。
    fn started_service_ops(
        &self,
    ) -> BTreeMap<String, Vec<crate::capabilities::session::api::ServiceOp>> {
        let mut out: BTreeMap<String, Vec<crate::capabilities::session::api::ServiceOp>> =
            BTreeMap::new();
        for view in self.residents.services().unwrap_or_default() {
            if view.state != crate::capabilities::residents::api::ServiceState::Ready {
                continue;
            }
            for op in &view.operations {
                out.entry(view.module.clone()).or_default().push(
                    crate::capabilities::session::api::ServiceOp {
                        tool: format!("{}.{}", view.name, op.name),
                        service: view.name.clone(),
                        op: op.name.clone(),
                        description: op.description.clone(),
                        parameters: op.parameters.clone(),
                    },
                );
            }
        }
        out
    }

    /// 核心自有的**共享区版本化**工具执行者：只在角色面确实发到它们时才装。
    /// 与代理工具同一套路：成员循环只问"谁认领这个名字"，核心自有工具不是循环里的特例。
    fn work_handlers(
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

    /// 一轮内对同一个成员最多提醒几次（用户可设，见 session-model.md 二）。
    pub fn discuss_remind_cap(&self) -> u32 {
        self.registry.app().discuss_remind_cap
    }

    /// 目的：本会话的上下文预算：窗口按模型现算（缺省用核心默认），百分比来自全局设置。
    /// 约束：0% = 关自动压缩。见 docs/session/session-model.md 六。
    pub(crate) fn context_budget(&self, model: Option<&str>) -> (u64, u8) {
        // 窗口是登记处的事实，不在这里另存一份。
        (
            self.registry.context_of(model),
            self.registry.app().compact_at_percent,
        )
    }

    /// 角色表发放的系统工具 id 清单（工具面的名字部分）。
    /// 为什么不留第二份名单：代码里出现"哪个角色能调哪个工具"必然与表漂。
    pub(crate) fn role_tools(&self, role: &str) -> Vec<String> {
        self.systools
            .tool_face(role)
            .map(|f| f.into_iter().map(|(id, _)| id.to_string()).collect())
            .unwrap_or_default()
    }

    /// 装配单 agent 会话（不插入会话中心；插入与落盘由 create_work 统一做）。
    // 会话装配把这一场单 agent 会话一次说全；收口成参数对象只是把参数挪个地方。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_single(
        &self,
        a: &AgentMeta,
        modules: &[Module],
        channel: Channel,
        sb: &crate::capabilities::workspace::api::Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        tier: crate::kernel::api::Tier,
        guest: Option<crate::capabilities::vm::api::GuestOs>,
        guest_lease: String,
        host_plan: &pkgs::PackagePlan,
    ) -> (
        crate::capabilities::session::api::AgentSession,
        Vec<SessionEvent>,
    ) {
        let chat = self.llm.member_channel(&channel, &a.name);
        // 形态按登记处解析；通道已在前置判据里解析成功，这里不再有"没有通道"的分支。
        let mode = self.registry.tool_mode(a.model.as_deref());
        self.log.info(
            "conductor::build_single",
            &format!(
                "单 agent 工作：agent {}，模块 {}，模型 {}",
                a.name,
                a.modules.join("+"),
                channel.model
            ),
        );
        // **会话参数**：身份块每回合由它现渲染（不存进消息列表）。
        let params = crate::capabilities::session::api::SessionParams::from_workspace(
            &a.name, sb, modules, "single", "solo",
        )
        .with_chara(a.chara.clone())
        .with_guest(guest);
        // 单 agent 工作：用户自己开的那场对话（不是任务链的节点）——身份是 solo。
        let tools = self.tools_env(
            modules,
            sb,
            unavailable,
            net,
            mode,
            "solo",
            tier,
            guest_lease,
            host_plan,
        );
        let mut s = crate::capabilities::session::api::AgentSession::new(
            &a.name,
            params,
            chat,
            Some(tools),
            self.prompt
                .text(crate::capabilities::prompt::api::Segment::CompactPrompt)
                .to_string(),
        );
        // 自动压缩的预算按**这个 agent 的模型**窗口算（见 session-model.md 六）。
        let (window, percent) = self.context_budget(a.model.as_deref());
        s.set_context_budget(window, percent);
        (s, Vec::new())
    }
}
