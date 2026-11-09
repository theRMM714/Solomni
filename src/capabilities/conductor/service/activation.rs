//! 目的：会话的启用与激活两段——历史会话按转录重建为活动会话、构建请求、准备与提交激活。
//! 管：跨重启续跑与回档的前提（状态全部派生）、激活的三步（构建请求 / 准备 / 提交）。
//! 不管：回合怎么跑（turn.rs / flow.rs）、协作动作（flow.rs）。
//! 联动：turn / flow 在推进前经 ensure_session 取活动会话。
//!   本方法族只碰 history / sessions / registry / vm 这几个字段。

use super::*;

impl Conductor {
    /// 目的：历史会话转为活动会话（跨重启续跑/回档的前提）：按转录重建，状态全部派生。
    pub(crate) fn ensure_session(&mut self, sid: &str) -> Result<(), String> {
        if self.sessions.contains_key(sid) {
            return Ok(());
        }
        // 会话对象在工作线程上（生成中）：**不能**从盘上再建一份（会变成两个实例）。
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        let (meta, events) = self.history_open(sid)?;
        let rebuilt = self.rebuild_session(&meta, &events)?;
        self.sessions.insert(sid.to_string(), rebuilt);
        Ok(())
    }

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
        if self.running.contains(sid) {
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
                if let Err(e) = self.ensure_session(&sid) {
                    if let Some(l) = &lease {
                        if !self.running.contains(&sid) {
                            self.vm.guest_stop(l);
                        }
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
