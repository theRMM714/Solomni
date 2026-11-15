//! 目的：会话的启用与激活两段——历史会话按转录重建为活动会话、构建请求、准备与提交激活。
//! 管：跨重启续跑与回档的前提（状态全部派生）、激活的三步（构建请求 / 准备 / 提交）。
//! 不管：回合怎么跑（turn.rs / flow.rs）、协作动作（flow.rs）。
//! 联动：turn 能力在推进前用会话本体的 `ensure_session` 取活动会话；本文件只剩激活三段（构建请求 / 准备 / 提交）。
//!   本方法族只碰 history / turn / registry / vm 这四个字段。

use super::*;

impl Conductor {
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
        sid: &str,
    ) -> Result<super::PreparedActivation, String> {
        let meta = self.history.meta(sid)?;
        if meta.run == RunState::Closed {
            return Err(format!("会话 {} 已关闭：终态，不能再启用", sid));
        }
        if self.turn.is_running(sid) {
            return Ok(super::PreparedActivation::Running(Box::new(
                self.session_view(sid)?,
            )));
        }
        let boot = self.prepare_guest_boot(sid)?;
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
        prepared: super::PreparedActivation,
    ) -> Result<SessionView, String> {
        match prepared {
            super::PreparedActivation::Running(view) => Ok(*view),
            super::PreparedActivation::Ready(ready) => {
                let super::ReadyActivation { sid, boot, lease } = *ready;
                // guest 启动已在调用方线程完成；这里只把它的句柄丢掉。
                drop(boot);
                let rebuilt = {
                    // 装配要 `&mut` 会话本体，同时读登记处与宿主探测：拆开借。
                    let (turn, registry, probe) = self.turn_parts();
                    turn.ensure_session(&sid, registry, probe, &|w, sb, allowed| {
                        work_tools::handlers(w, sb, allowed)
                    })
                };
                if let Err(e) = rebuilt {
                    if let Some(l) = &lease
                        && !self.turn.is_running(&sid)
                    {
                        self.vm.guest_stop(l);
                    }
                    return Err(e);
                }
                if let Err(e) = self.resume_subtree(&sid) {
                    if let Some(l) = &lease {
                        self.vm.guest_stop(l);
                    }
                    return Err(e);
                }
                self.session_view(&sid)
            }
        }
    }
}
