//! 目的：guest 的一段活与创建失败的撤销——准备起 guest 的参数、打包停止、撤销准备段占下的东西。
//! 管：`prepare_guest_boot` / `guest_boot`（起）、`guest_stop`（停）、`abort_work`（撤销）。
//! 不管：两段建工作本身（`create.rs`）、guest 进程（vm 能力）、配置改配（`config.rs`）。
//! 联动：`conductor` 的句柄层在调用方线程上 `run()` 这些活；命令行里的外部工具路径读 `host.rs`。

use super::*;

impl Work {
    /// 目的：创建失败（guest 起不来 / 落盘失败）时撤销准备段占下的东西：名字占位 + 这次建出来的空工作区目录。
    pub(crate) fn abort_work(&mut self, turn: &mut Turn, prepared: PreparedWork) {
        if let PreparedWork::Pending(p) = prepared {
            turn.unmark_starting(&p.name);
            if let Some((session, agents)) = p.discard {
                let _ = self.workspace.discard(&session, &agents);
            }
        }
    }

    /// 目的：把"停 guest"这一段打包成交给调用方线程的活（销毁要等它落盘，秒级）。
    pub(crate) fn guest_stop(&self, lease: &str) -> GuestStop {
        GuestStop::new(Arc::clone(&self.vm), vec![lease.to_string()])
    }

    /// 目的：按一条会话的现状准备"确保 guest"那一段（启用、发言、成员回合共用）。
    /// 返回：None = 本机档 / proxy 父会话 / 已有活 guest / 代拟名单还没确认；有 = 要在调用方线程跑。
    pub(crate) fn prepare_guest_boot(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        sid: &str,
    ) -> Result<Option<crate::capabilities::work::service::guest_job::GuestBoot>, String> {
        let meta = self.history.meta(sid)?;
        if meta.run == RunState::Closed {
            return Err(format!("会话 {} 已关闭：终态，不能再启用", sid));
        }
        let Some((lease, scope_meta)) = turn.guest_scope(&meta)? else {
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
        let tree = turn.tree_meta(&scope_meta, None)?;
        let plan = turn.plan_for_meta_with(&tree, &roster, self.probe(), registry)?;
        let sandboxes = turn.sandboxes(&tree, &roster, registry, self.probe())?;
        Ok(Some(self.guest_boot(
            turn, registry, &tree, &plan, &sandboxes, false, &lease,
        )))
    }

    /// 目的：把"起 guest"这一段打包成交给调用方线程的活——核心只准备参数，不在这里拉起。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn guest_boot(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        _meta: &SessionMeta,
        plan: &crate::capabilities::vm::api::ExecPlan,
        sandboxes: &crate::capabilities::workspace::api::Sandboxes,
        restart: bool,
        lease: &str,
    ) -> GuestBoot {
        let share_tool = self.share_tool_path(registry);
        // 开发/应急开关打开时，本进程的 VM 会话强制走 copy（不追加成回退链）；
        // 普通进程没有这个开关，share_chain 只给用户档。
        let shares = if crate::capabilities::vm::api::copy_share_enabled() {
            vec!["copy"]
        } else {
            crate::capabilities::vm::api::share_chain(
                std::env::consts::OS,
                share_tool.as_deref(),
                self.probe.as_ref(),
            )
        };
        GuestBoot::new(
            Arc::clone(&self.vm),
            self.root.clone(),
            Arc::clone(&self.probe),
            Arc::clone(&self.log),
            registry.app().max_guests,
            turn.host_read_roots(registry),
            self.qemu_path(registry),
            share_tool,
            shares,
            lease.to_string(),
            turn.network_allowed(registry),
            plan.clone(),
            sandboxes.clone(),
            restart,
        )
    }
}
