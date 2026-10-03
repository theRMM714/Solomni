//! **装配材料**：成员通道、沙箱清单、工具环境、预算与角色工具面、单 agent 会话对象（build_single）。
//!
//! 它是"把各能力的事实拼成一个能跑的会话"的那一层：纯派生，不落盘。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate)（或 pub）供兄弟族与 conductor/api.rs 调用。

use super::*;

impl Conductor {
    /// 核心按用户选择解析模型通道；缺省用核心默认；都缺 = None（网关回落演示并告知）。
    pub(crate) fn channel_of(&self, model: Option<&str>) -> Option<Channel> {
        self.registry.channel(model)
    }

    /// 组装本次工作的沙箱清单：工作根来自 Workspace 端口，模块目录来自清单。
    /// 权限策略在此收口：模块目录只对其所属 agent 可达（同一模块不会同属两个 agent，创建时已校验）。
    /// 每个 agent 的沙箱按 meta.agents 建；共享区根任何时候都有（代拟还没名单时也有，@ 改写要用）。
    pub(crate) fn sandboxes(
        &self,
        meta: &SessionMeta,
        roster: &crate::capabilities::workspace::api::Roster,
    ) -> Result<crate::capabilities::workspace::api::Sandboxes, String> {
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        // 沙箱锚在**工作**上：子会话与父会话共用一套工作区（见 SessionMeta::work）。
        let roots = self.workspace.roots(meta.work(), &names)?;
        let mut list: Vec<crate::capabilities::workspace::api::Sandbox> = Vec::new();
        for a in &meta.agents {
            let private = roots
                .agents
                .get(&a.name)
                .cloned()
                .ok_or_else(|| format!("工作区没有给出 agent {} 的沙箱路径", a.name))?;
            let mut modules = BTreeMap::new();
            for id in &a.modules {
                if let Some(m) = roster.modules.iter().find(|m| &m.manifest.id == id) {
                    modules.insert(id.clone(), m.root.clone());
                }
            }
            list.push(crate::capabilities::workspace::api::Sandbox {
                work_name: meta.work().to_string(),
                agent: a.name.clone(),
                shared: roots.shared.clone(),
                private,
                modules,
                texts: self.prompt.tools(),
            });
        }
        Ok(crate::capabilities::workspace::api::Sandboxes {
            shared: roots.shared,
            list,
        })
    }

    /// 工具环境：内置文件工具永远可用；外部工具按模块分组放行（模块 id → 目录 + 工具表）。
    /// unavailable：本档位下缺运行包、不能执行工具的模块（机制侧据此拒绝执行，并如实报缺哪个能力）。
    /// net：会话是否放行出站网络（exec 段；默认否），随围栏交给机制层。
    pub(crate) fn tools_env(
        &self,
        modules: &[Module],
        sb: &crate::capabilities::workspace::api::Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
        mode: crate::capabilities::llm::api::ToolMode,
        // 这一席的身份：用户建的单 agent 会话 = solo，协作子会话 = executor（见 systools/roles.yaml）。
        role: &str,
    ) -> crate::capabilities::session::api::MemberTools {
        crate::capabilities::session::api::MemberTools {
            mode,
            modules: crate::capabilities::session::api::tool_table(modules),
            observations: crate::capabilities::tools::api::Observations::default(),
            llm: Arc::clone(&self.llm),
            log: Arc::clone(&self.log),
            tools: Arc::clone(&self.tools),
            sandbox: sb.clone(),
            builtin_tools: self.systools.book(),

            unavailable,
            // 围栏：可达范围 + 断网 + 环境白名单的落点，全部由该 agent 的沙箱派生（机制在 adapters）；
            // 只读根来自用户显式授权（`fence_read`），默认空。
            fence: crate::capabilities::tools::api::FenceSpec::from_sandbox(sb, net)
                .with_read_only(self.fence_read_roots()),
            // 从零开始；按落盘转录重建时由调用方按转录里的最大值续号（见 rebuild_session）。
            reply_seq: 0,
            // 这一席的系统工具面**由角色表发放**（越权校验的唯一判据）：给什么写什么，代码里不留第二份名单。
            allowed: self.role_tools(role),
            // 能不能用自己模块的工具、以及工具说明块的素材：都按角色表与这个 agent 的模块装配期算好。
            with_modules: self.systools.allows_module_tools(role),
            notes: crate::capabilities::tools::api::tool_notes(&*self.prompt, sb, modules),
            handlers: Vec::new(),
        }
    }

    /// 一轮内对同一个成员最多提醒几次（用户可设，见 session-model.md 二）。
    pub fn discuss_remind_cap(&self) -> u32 {
        self.registry.app().discuss_remind_cap
    }

    /// 自动压缩的**字符预算** = 该模型的上下文窗口 × 设置百分比 × 4（≈ 字符/token 的粗估）。
    /// 百分比为 0 = 关。见 docs/session/session-model.md 六。
    pub(crate) fn compact_budget(&self, model: Option<&str>) -> usize {
        let pct = self.registry.app().compact_at_percent as u64;
        if pct == 0 {
            return 0;
        }
        // 窗口按模型现算（缺省用核心默认；查不到就是保守缺省）——这是登记处的事实，不在这里另存一份。
        let ctx = self.registry.context_of(model);
        (ctx * pct / 100 * 4) as usize
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
    pub(crate) fn build_single(
        &self,
        a: &AgentMeta,
        modules: &[Module],
        channel: Option<Channel>,
        sb: &crate::capabilities::workspace::api::Sandbox,
        unavailable: BTreeMap<String, Vec<String>>,
        net: bool,
    ) -> (
        crate::capabilities::session::api::AgentSession,
        Vec<SessionEvent>,
    ) {
        let (chat, note) = self.llm.member_channel(channel.as_ref(), &a.name);
        // 形态按登记处解析；没有真实通道（演示回落）只能是手写信封——演示通道不会原生调用。
        let mode = if channel.is_some() {
            self.registry.tool_mode(a.model.as_deref())
        } else {
            crate::capabilities::llm::api::ToolMode::Envelope
        };
        self.log.info(
            "conductor::build_single",
            &format!(
                "单 agent 工作：agent {}，模块 {}，模型 {}",
                a.name,
                a.modules.join("+"),
                channel
                    .as_ref()
                    .map(|c| c.model.as_str())
                    .unwrap_or("无（演示）")
            ),
        );
        // **会话参数**：身份块每回合由它现渲染（不存进消息列表）。
        let params =
            crate::capabilities::session::api::SessionParams::from_workspace(&a.name, sb, modules);
        // 单 agent 工作：用户自己开的那场对话（不是任务链的节点）——身份是 solo。
        let tools = self.tools_env(modules, sb, unavailable, net, mode, "solo");
        let mut s = crate::capabilities::session::api::AgentSession::new(
            &a.name,
            params,
            chat,
            note,
            Some(tools),
            self.prompt.refs(),
            self.prompt.tools(),
        );
        // 自动压缩的预算按**这个 agent 的模型**窗口算（见 session-model.md 六）。
        s.set_compact_budget(self.compact_budget(a.model.as_deref()));
        let opened = s.open();
        (s, opened)
    }
}
