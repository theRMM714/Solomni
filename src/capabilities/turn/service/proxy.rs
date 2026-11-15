//! 目的：代理会话的**装配**——按一份 meta 装出代理会话对象（创建与重建共用同一处）。
//! 管：代理私有沙箱的取法、发言席沙箱与权限面、代理工具面与上下文预算的装配。
//! 不管：代理会话的清单 / 转达 / 观察 / 生命周期 / 消息倒查（仍在 `conductor`）；落盘。
//! 联动：`assemble.rs` 的 `member_tools` 装工具面；`rebuild.rs` 与 `conductor` 的建代理会话都经这里。

use std::collections::BTreeMap;

use crate::capabilities::llm::api::Channel;
use crate::capabilities::proxy::api as d;
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{AgentSession, SessionMeta, SessionParams};
use crate::capabilities::turn::api::{HandlerSource, MemberSpec, member_tools};
use crate::capabilities::workspace::api::Sandbox;

use crate::capabilities::turn::service::session::Turn;

/// 目的：代理会话自己的私有沙箱目录名（与 work/、children/ 同级，写进保留名表）。
pub const PROXY_PRIVATE_DIR: &str = ".proxy";

impl Turn {
    /// 目的：装配一个代理会话对象（创建与重建**共用同一处**，两处不各拼一遍）。
    pub(crate) fn build_proxy(
        &self,
        meta: &SessionMeta,
        registry: &dyn Registry,
        handlers: &HandlerSource<'_>,
    ) -> Result<AgentSession, String> {
        let channel = registry.core_channel()?;
        self.build_proxy_with(meta, &channel, registry, handlers)
    }

    pub(crate) fn build_proxy_with(
        &self,
        meta: &SessionMeta,
        channel: &Channel,
        registry: &dyn Registry,
        handlers: &HandlerSource<'_>,
    ) -> Result<AgentSession, String> {
        self.workspace
            .prepare(&meta.name, &[PROXY_PRIVATE_DIR.to_string()])?;
        let roots = self
            .workspace
            .roots(&meta.name, &[PROXY_PRIVATE_DIR.to_string()])?;
        let private = roots
            .agents
            .get(PROXY_PRIVATE_DIR)
            .cloned()
            .ok_or_else(|| "代理会话没有私有沙箱路径".to_string())?;
        let shared_grant = self.workspace_grant(meta)?;
        let sb = Sandbox {
            work_name: meta.name.clone(),
            session: meta.name.clone(),
            agent: d::SPEAKER.to_string(),
            shared: roots.shared.clone(),
            private,
            modules: BTreeMap::new(),
            modules_with_userdata: std::collections::BTreeSet::new(),
            permissions: self.permissions_for(meta, registry)?,
            // 代理发言席自己住在宿主上（文件工具也跑在宿主），不需要 guest 翻译；
            // 但视图盘打开时仍要同一个视图写法，否则环境块渲染视图路径、寻址却按宿主根判越界。
            path_codec: crate::kernel::api::view_codec(),
            shared_grant,
        };
        let tool_mode = registry.tool_mode(None);
        let chat = self.llm.core_channel(channel);
        // 身份是**角色提示词**（core_proxy），不是某个 agent 的模块能力包：
        // 机制 / 环境 / 调用约定仍与 agent 身份同一份口径（见 workspace 的 render_system）。
        let mut params = SessionParams::from_workspace(d::SPEAKER, &sb, &[], "proxy", "core_proxy");
        params.role_system = Some(crate::capabilities::prompt::api::Segment::ProxySystem);
        let tools = member_tools(
            &self.session_deps(),
            &MemberSpec {
                modules: &[],
                sandbox: &sb,
                unavailable: BTreeMap::new(),
                net: self.network_allowed(registry),
                mode: tool_mode,
                role: "core_proxy",
                tier: crate::kernel::api::Tier::Host,
                guest_lease: String::new(),
                plan: &crate::capabilities::packages::api::PackagePlan::default(),
                base_roots: &self.base_roots,
                host_read_roots: self.host_read_roots(registry),
                shell: self.shell_slot(&sb.session),
                handlers: &|allowed| handlers(&self.workspace, &sb, allowed),
            },
        );
        let mut s = AgentSession::new(
            d::SPEAKER,
            params,
            chat,
            Some(tools),
            self.prompt
                .text(crate::capabilities::prompt::api::Segment::CompactPrompt)
                .to_string(),
        );
        let (window, percent, keep_percent) = self.context_budget(None, registry);
        s.set_context_budget(window, percent, keep_percent);
        s.set_context_freshness(meta.context_freshness);
        s.set_model(registry.core_model());
        s.set_compact_factory(self.compact_factory());
        Ok(s)
    }
}
