//! 目的：会话的启用与激活两段——历史会话按转录重建为活动会话、构建请求、准备与提交激活。
//! 管：跨重启续跑与回档的前提（状态全部派生）、激活三步（构建请求 / 准备 / 提交）、子树解冻。
//! 不管：回合怎么跑（`conductor` 的生成驱动）、协作动作（`conductor` 的 flow）。
//! 联动：`conductor` 的句柄层按两段调用；重建走 `turn` 的 `ensure_session`。

use super::*;

impl Work {
    /// 目的：启用前补编所需的目标与模块清单（只读事实；构建在调用方线程上跑）。
    pub(crate) fn activation_build_request(
        &self,
        sid: &str,
    ) -> Result<(Vec<String>, crate::kernel::api::Tier, Option<String>), String> {
        let meta = self.history.meta(sid)?;
        Ok((
            meta.modules.clone(),
            meta.exec.tier,
            meta.exec.guest.clone(),
        ))
    }

    /// 目的：启用会话的第一段（核心，毫秒级）——closed 拒绝；正在生成则附着；
    ///   否则准备可选的一段 guest 启动，等调用方线程跑完后提交。
    pub(crate) fn prepare_activation(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        sid: &str,
    ) -> Result<super::PreparedActivation, String> {
        let meta = self.history.meta(sid)?;
        if meta.run == RunState::Closed {
            return Err(format!("会话 {} 已关闭：终态，不能再启用", sid));
        }
        if turn.is_running(sid) {
            return Ok(super::PreparedActivation::Running(Box::new(
                self.session_view(turn, registry, sid)?,
            )));
        }
        let boot = self.prepare_guest_boot(turn, registry, sid)?;
        let lease = boot.as_ref().map(|b| b.lease().to_string());
        Ok(super::PreparedActivation::Ready(Box::new(
            super::ReadyActivation {
                sid: sid.to_string(),
                boot,
                lease,
            },
        )))
    }

    /// 目的：启用会话的第二段（核心）——按落盘转录重建、解冻整棵子树，返回启用后的视图。
    /// 约束：guest 已经由调用方线程启动；这里失败要收掉本次刚拉起的 guest，不留半成品。
    pub(crate) fn commit_activation(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        prepared: super::PreparedActivation,
    ) -> Result<SessionView, String> {
        match prepared {
            super::PreparedActivation::Running(view) => Ok(*view),
            super::PreparedActivation::Ready(ready) => {
                let super::ReadyActivation { sid, boot, lease } = *ready;
                // guest 启动已在调用方线程完成；这里只把它的句柄丢掉。
                drop(boot);
                let probe = self.probe.as_ref();
                let rebuilt = turn.ensure_session(&sid, registry, probe);
                if let Err(e) = rebuilt {
                    if let Some(l) = &lease
                        && !turn.is_running(&sid)
                    {
                        self.vm.guest_stop(l);
                    }
                    return Err(e);
                }
                if let Err(e) = self.resume_subtree(turn, &sid) {
                    if let Some(l) = &lease {
                        self.vm.guest_stop(l);
                    }
                    return Err(e);
                }
                self.session_view(turn, registry, &sid)
            }
        }
    }

    /// 目的：解除整棵子树的「已停止」（停止的逆操作）；返回是否确实解除了至少一个。
    /// 约束：子树里只要有**已关闭**的就整条拒绝——关闭是终态，不能被「继续」拉回来。
    pub(crate) fn resume_subtree(&self, turn: &Turn, name: &str) -> Result<bool, String> {
        let mut any = false;
        for sid in turn.subtree_of(name) {
            let mut meta = self.history.meta(&sid)?;
            match meta.run {
                RunState::Closed => {
                    return Err(format!("会话 {} 已关闭：终态，不能再继续", sid));
                }
                RunState::Stopped => {
                    meta.run = RunState::Active;
                    self.history.save_meta(&meta)?;
                    any = true;
                }
                RunState::Active => {}
            }
        }
        Ok(any)
    }
}
