//! 目的：历史与落盘——历史列出 / 打开 / 删除、事件收编（record_events）与增量落盘手柄。
//! 管：什么时候写（落盘策略与警告形状在 `session` 的 `record_events`，写盘机制在它的 `HistoryStore` 端口后面）。
//! 不管：会话对象怎么建（flow.rs 与 work/create.rs）、视图怎么渲染（work/views.rs）。
//! 联动：会话树与运行态判据在会话本体（turn 能力）里，这里只按需取用。
//!   本方法族只碰 history / log / turn / vm 这四个字段。
use super::*;

use crate::capabilities::lifecycle::api::{FenceRelease, ReleaseByLease, SessionDelete};

impl Conductor {
    /// 目的：历史列表：**树序**（每个父会话紧跟它的子会话）——侧栏据此缩进，
    ///  顺序与缩进同源，不会再出现"子会话排到父会话上面"的错位（见 session-model.md 一）。
    ///  读盘失败如实记日志，返回空表。
    pub fn history_list(&self) -> Vec<HistoryView> {
        crate::capabilities::lifecycle::api::list(&*self.history, &*self.log)
    }

    /// 目的：打开历史会话：返回元信息与事件流（只读回放；是否续跑由用户点「继续」授权）。
    ///  回放同样应用 rewind 截断——流水保留审计，会话内容以截断后为准。
    pub fn history_open(
        &self,
        name: &str,
    ) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        crate::capabilities::lifecycle::api::open(&*self.history, name)
    }

    /// 目的：删除会话（含它的整棵子树）。
    /// 约束：口径见 docs/session/session-model.md 七——**子会话不允许单独删**（它由核心按节点派生，删父会话会一起删掉；
    ///   单独删会让父子关系断掉）；**删父带子**按 meta.parent 收集整棵子树，移出内存 → 收集待删目录；
    ///   目录删除在收尾层先停 guest 再执行，删完目录后在后台按租约撤围栏授权；**子树里任一节点在生成中就整体拒绝**，不能删到一半留下半个状态。
    ///   内部的 session::History::delete（纯存储）不受影响——系统路径还需要它。
    #[cfg(test)]
    pub fn history_delete(&mut self, name: &str) -> Result<bool, String> {
        self.delete_bookkeeping(name)?.run()
    }

    /// 目的：删除的核心动作（毫秒级）：回收常驻服务、移出内存、收集待删目录——**不停 guest、不删目录**。
    /// 返回：调用方线程要执行的收尾（先停 guest，再删目录，最后后台按租约撤权）。
    pub(crate) fn delete_bookkeeping(&mut self, name: &str) -> Result<SessionDelete, String> {
        // 不是落盘会话（或已不存在）：只清内存，目录交给收尾层如实回 false。
        let Ok((meta, _)) = self.history.load(name) else {
            self.turn.take_session(name);
            return Ok(SessionDelete::new(
                None,
                Arc::clone(&self.history),
                vec![name.to_string()],
                name.to_string(),
                None,
            ));
        };
        // 子会话由核心管理，不允许用户单独删。
        if let Some(parent) = meta.parent.clone() {
            return Err(format!(
                "会话 {} 是核心管理的子会话（父会话 {}）：删父会话会一起删掉它，不能单独删",
                name, parent
            ));
        }
        let subtree = self.turn.subtree_of(name);
        for sid in &subtree {
            if self.turn.is_running(sid) {
                return Err(crate::capabilities::turn::api::Turn::running_refusal(sid));
            }
        }
        // 围栏授权按会话租约持有：撤权放到收尾层、删完目录之后在后台做（不占核心线程与用户等待路径）。
        // 常驻服务租约仍在核心线程回收（毫秒级）。
        for sid in &subtree {
            if let Err(e) = self.residents.reap(sid) {
                self.log.warn(
                    "conductor::history_delete",
                    &format!("回收常驻服务租约未完成：{}", e),
                );
            }
        }
        // 虚拟机档的租约交给调用方销毁（guest 关机要等它落盘，别占核心线程）。
        // 共享 guest：按作用域租约去重（协作子会话 / proxy 子树共用一台）。
        let mut guest_leases: Vec<String> = Vec::new();
        for sid in &subtree {
            let Ok(meta) = self.history.meta(sid) else {
                continue;
            };
            if let Ok(Some((lease, _))) = self.turn.guest_scope(&meta)
                && !guest_leases.iter().any(|l| l == &lease)
            {
                guest_leases.push(lease);
            }
        }
        let vm = Arc::clone(&self.vm);
        let stop: Option<Box<dyn FnOnce() + Send>> = Some(Box::new(move || {
            crate::capabilities::work::api::GuestStop::new(vm, guest_leases).run()
        }));
        for sid in &subtree {
            self.turn.take_session(sid);
            // 会话没了，它的裁决队随会话一起消失（等待方与转录都不复存在）。
            self.turn.forget_desk(sid);
        }
        // 目录按后序删：先子后父；父目录被删时，落在它内部的子会话目录一起消失。
        // 真正删除在 `SessionDelete::run`（先停 guest，再删目录，最后后台撤权）。
        let dirs: Vec<String> = subtree.iter().rev().cloned().collect();
        let tools = Arc::clone(&self.tools);
        let release: ReleaseByLease = Arc::new(move |lease: &str| tools.release_by_lease(lease));
        let fences = FenceRelease::new(subtree.clone(), release, Arc::clone(&self.log));
        Ok(SessionDelete::new(
            stop,
            Arc::clone(&self.history),
            dirs,
            name.to_string(),
            Some(fences),
        ))
    }

    /// 目的：把整棵子树（含自己）的运行态落成同一个值：**停止**先整棵冻上，**继续**再整棵解开。
    ///  停止时顺序不能反——先冻态再取消生成，被停会话收尾写的那条"这一轮结束"才会被闸门拒掉。
    pub(crate) fn set_subtree_run(
        &self,
        name: &str,
        run: crate::capabilities::session::api::RunState,
    ) -> Result<Vec<String>, String> {
        let tree = self.turn.subtree_of(name);
        for sid in &tree {
            let mut meta = self.history.meta(sid)?;
            if meta.run != run {
                meta.run = run;
                self.history.save_meta(&meta)?;
            }
        }
        Ok(tree)
    }

    /// 目的：事件落盘；失败如实告知（追加一条警告事件），不静默丢历史。
    /// 约束：留档策略与警告形状归 `session`（`record_events` 只有一处实现），这里只交端口。
    pub(crate) fn record_events(&self, sid: &str, events: &mut Vec<SessionEvent>) {
        crate::capabilities::session::api::record_events(
            self.history.as_ref(),
            self.log.as_ref(),
            sid,
            events,
        );
    }

    /// 目的：增量落盘手柄：交给工作线程，按"一次模型调用"的粒度落盘（见 `Persister`）。
    pub(crate) fn persister(&self, sid: &str) -> crate::capabilities::session::api::Persister {
        crate::capabilities::session::api::Persister::new(
            Arc::clone(&self.history),
            Arc::clone(&self.log),
            sid,
        )
    }
}
