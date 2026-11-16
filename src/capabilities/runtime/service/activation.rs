//! 目的：启用的长步骤编排：补编模块产物、拉 guest、把激活请求跑完。
//! 管：ensure_built（调下层 builds 的用例）、run_guest_boot（起 guest 租约）、ensure_guest、activate_session。
//! 不管：构建与 guest 的具体做法（在下层 builds / vm / work）；本文件只排先后、转错误与提示。
//! 联动：work::api 的用例面提供四段式（构建请求 / 准备 / 提交），Handle::call 把状态改动交给核心线程。

use super::*;

impl Handle {
    /// 目的：创建会话前的补编——本次名单里的模块若在目标环境没有入口、且声明了 `build:`，先在这里编好。
    /// 参数：`ids` = 本次会话的模块 id（已去重保序）；`tier` / `guest` = 目标环境。
    /// 错误：构建失败或环境不齐时如实拒绝（错误里带目标与构建输出）。
    /// 约束：跑在**调用方线程**（不占核心队列），核心线程不陪着等编译器；入口已在场的模块不碰。
    pub(crate) fn ensure_built(
        &self,
        ids: &[String],
        tier: crate::kernel::api::Tier,
        guest: Option<&str>,
    ) -> Result<(), String> {
        use crate::capabilities::builds::api::Target;
        if ids.is_empty() {
            return Ok(());
        }
        let target = match tier {
            crate::kernel::api::Tier::Host => Target::Host,
            crate::kernel::api::Tier::Vm => match guest.map(str::trim).filter(|g| !g.is_empty()) {
                Some(g) => Target::Guest(g.to_string()),
                // 虚拟根没给：准备段会如实拒绝（这里不猜默认值）。
                None => return Ok(()),
            },
        };
        let roster = self.modules.roster();
        let net = self.call(|core| {
            Ok(crate::capabilities::env::api::network_allowed(
                core.registry().app(),
            ))
        })?;
        // 一批一起交给构建能力：虚拟机档只起一台 guest、一块缓存卷，不按模块反复起停。
        let modules: Vec<&crate::capabilities::modules::api::Module> = ids
            .iter()
            .filter_map(|id| roster.modules.iter().find(|m| &m.manifest.id == id))
            .collect();
        if modules.is_empty() {
            return Ok(());
        }
        let built = self
            .builds
            .ensure_many(&modules, &target, net)
            // 单模块时指名模块；多个模块时内层错误已经点出是哪一个。
            .map_err(|e| match modules.as_slice() {
                [one] => format!(
                    "模块 {} 构建失败（目标 {}）：{}",
                    one.manifest.id,
                    target.label(),
                    e
                ),
                _ => format!("构建失败（目标 {}）：{}", target.label(), e),
            })?;
        for b in built.iter().filter(|b| !b.skipped) {
            self.log.info(
                "runtime::create_session",
                &format!("已补编模块 {}（目标 {}）", b.module, b.target),
            );
        }
        Ok(())
    }

    /// 目的：串行同一台 guest 的启动；离开作用域自动摘掉标记（即便启动失败/ panic）。
    pub(super) fn run_guest_boot(
        &self,
        boot: crate::capabilities::work::api::GuestBoot,
    ) -> Result<(), String> {
        struct Slot<'a> {
            set: &'a std::sync::Mutex<std::collections::BTreeSet<String>>,
            lease: String,
        }
        impl Drop for Slot<'_> {
            fn drop(&mut self) {
                if let Ok(mut set) = self.set.lock() {
                    set.remove(&self.lease);
                }
            }
        }
        let lease = boot.lease().to_string();
        {
            let mut set = self
                .booting
                .lock()
                .map_err(|_| "guest 启动锁坏了".to_string())?;
            if !set.insert(lease.clone()) {
                return Err(format!("guest 正在启动（租约 {}）：稍后再试", lease));
            }
        }
        let _slot = Slot {
            set: &self.booting,
            lease,
        };
        boot.run()
    }

    /// 目的：确保这条会话的 guest 已经起来（启用 / 发言 / 成员回合 / 节点共用的兜底）。
    /// 约束：只准备与启动，不改会话运行态；已有活 guest 或本机档 = 什么都不做。
    pub(crate) fn ensure_guest(&self, sid: &str) -> Result<(), String> {
        let sid = sid.to_string();
        let boot = self.call(move |core| core.prepare_guest_boot(&sid))?;
        if let Some(boot) = boot {
            self.run_guest_boot(boot)?;
        }
        Ok(())
    }

    /// 目的：启用一条历史会话（只恢复，不生成）：重建、确保 guest、解冻整棵子树。
    pub(crate) fn activate_session(&self, sid: &str) -> Result<SessionView, String> {
        let sid = sid.to_string();
        // 启用历史会话也要先确保目标环境的模块产物：老会话可能是在宿主上建的，
        // 切到/重开 VM 档时不能把宿主 native 产物直接带进 guest。
        let (ids, tier, guest) = self.call({
            let sid = sid.clone();
            move |core| core.activation_build_request(&sid)
        })?;
        self.ensure_built(&ids, tier, guest.as_deref())?;
        let prepared = self.call(move |core| core.prepare_activation(&sid))?;
        let prepared = match prepared {
            crate::capabilities::work::api::PreparedActivation::Ready(ready) => {
                let crate::capabilities::work::api::ReadyActivation { sid, boot, lease } = *ready;
                if let Some(boot) = boot {
                    self.run_guest_boot(boot)?;
                }
                crate::capabilities::work::api::PreparedActivation::Ready(Box::new(
                    crate::capabilities::work::api::ReadyActivation {
                        sid,
                        boot: None,
                        lease,
                    },
                ))
            }
            other => other,
        };
        self.call(move |core| core.commit_activation(prepared))
    }
}
