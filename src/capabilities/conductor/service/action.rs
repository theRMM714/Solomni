//! 目的：动作的核心实现——建会话（人经呈现层与核心代理共用的唯一一条）。
//! 管：把规整后的建会话载荷变成一个真正的会话（名字 / 档位 / 名单的规整）。
//! 不管：会话装配本身（`work.rs` 的 `create_work_inner`）；生成怎么跑（在句柄的生成里）。
//! 联动：载荷形状见 `conductor/domain/proxy.rs`；声明与 callers 在 `systools/tools.yaml`。

use super::guest_job::GuestBoot;
use super::*;
use crate::capabilities::conductor::domain::proxy as d;
use crate::kernel::api::Tier;

impl Conductor {
    /// 目的：**建会话的唯一实现**——父会话由 `spec.parent` 给出（None = 顶层会话，人经呈现层建的）。
    /// 参数：`spec` 是规整后的载荷（名字 / 档位 / 名单 / 本次需求）。
    /// 返回：建出的会话与名单 + 开场事实（事实由 api 层发布到事件台）。
    /// 错误：形态、名字、档位或名单不成立时如实拒绝，不留半成品。
    #[cfg(test)]
    pub(crate) fn create_session(
        &mut self,
        spec: &d::NewSession,
    ) -> Result<(d::Created, Vec<SessionEvent>), String> {
        let (prepared, boot) = self.prepare_session(spec)?;
        if let Some(boot) = boot {
            if let Err(e) = boot.run() {
                self.abort_session(prepared);
                return Err(e);
            }
        }
        self.commit_session(prepared)
    }

    /// 目的：建会话的第一段（核心，毫秒级）——规整载荷、占名、备好工作区与计划。
    /// 返回：准备结果 + 可选的那段"要等 guest"的活（虚拟机档才有）。
    pub(crate) fn prepare_session(
        &mut self,
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
            Some(n) => self.unique_work_name(n, &derived),
            None => self.unique_work_name(&derived, "work"),
        };
        // 档位：点名就用它；否则子会话沿用父会话，顶层用设置里的默认档。
        let tier = match spec
            .tier
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(t) => Tier::parse(t)?,
            None => match &parent {
                Some(p) => self.history.meta(p)?.exec.tier,
                None => self.registry.app_settings().tier,
            },
        };
        // 虚拟根：点名就用它；否则子会话沿用父会话，顶层为空（建好后在会话配置里选）。
        let vm_guest = match spec
            .guest
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
        {
            Some(b) => Some(b.to_string()),
            None => match &parent {
                Some(p) => self.history.meta(p)?.exec.guest.clone(),
                None => None,
            },
        };
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
            guest: vm_guest,
        };
        // 有父 = 子工作（`create_work_inner` 直接给 parent）；没有父 = 顶层会话，走同一个入口包装。
        let prepared = match parent.as_deref() {
            Some(p) => self.prepare_work(work, Some(p))?,
            None => self.prepare_work(work, None)?,
        };
        Ok((PreparedSession { work: prepared.0 }, prepared.1))
    }

    /// 目的：建会话的第二段（核心）——装配会话对象、落盘身份、开场事实。
    pub(crate) fn commit_session(
        &mut self,
        prepared: PreparedSession,
    ) -> Result<(d::Created, Vec<SessionEvent>), String> {
        let opened = self.commit_work(prepared.work)?;
        Ok((
            d::Created {
                session: opened.sid,
                agents: opened.agents,
            },
            opened.facts,
        ))
    }

    /// 目的：建会话失败时撤销准备段（占名与空工作区目录）。
    pub(crate) fn abort_session(&mut self, prepared: PreparedSession) {
        self.abort_work(prepared.work);
    }
}

/// 目的：一次建会话的准备结果（两段之间的交接物）。
pub(crate) struct PreparedSession {
    work: super::work::PreparedWork,
}
