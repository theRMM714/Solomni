//! 目的：回档的编排：先把整棵子树的回合停稳，再让下层做回档算术，最后把结果推上事件台。
//! 管：rewind_with_reset、settle_tree、stop_tree、void_gates、publish_rewound。
//! 不管：回档怎么算、转录怎么切（在 lifecycle / session）；本文件只保证停下、回档、发布的次序。
//! 联动：lifecycle::api 的 rewind_outcome 是回档用例；事件台在 handle；裁决队作废走 turn。

use super::*;

impl Handle {
    /// 目的：回档并推重置快照（`RuntimeOps` 与动作表共用这一条收尾）。
    /// 约束：回档会改写盘上转录，事件台必须同时收到 `rewound` 才能让刷新/多端重置；两个入口各写一份，迟早一处漏推。
    pub(crate) fn rewind_with_reset(
        &self,
        sid: &str,
        target: RewindTarget,
        force: bool,
    ) -> Result<RewindOutcome, String> {
        // 三种回档都改写转录，删除还一起回退工作区文件：在跑的回合先停稳，
        // 否则它收尾时的写会落到已被改写的旧线或刚铺好的树上。
        if matches!(
            target,
            RewindTarget::Archive(_) | RewindTarget::Restore(_) | RewindTarget::Delete(_)
        ) {
            self.settle_tree(sid)?;
        }
        let outcome = self.call({
            let sid = sid.to_string();
            move |core| {
                let deps = RebuildDeps {
                    registry: core.registry.as_ref(),
                    probe: core.probe.as_ref(),
                };
                core.lifecycle
                    .rewind_outcome(&mut core.turn, deps, &target, &sid, force)
            }
        })?;
        self.publish_rewound(sid, &outcome);
        Ok(outcome)
    }

    /// 目的：留档 / 删除 / 恢复前把整棵子树在跑的回合停下来，并等它们把会话交回核心。
    /// 返回：实际停下的会话 id。
    /// 约束：等待期间**不占核心队列**——收尾要靠核心交回会话；30 秒仍未收尾 = 拒绝，文件与转录都还没动。
    pub(super) fn settle_tree(&self, root: &str) -> Result<Vec<String>, String> {
        let root_owned = root.to_string();
        let tree: Vec<String> = self.call(move |core| {
            let history = core.history_handle();
            Ok(
                crate::units::business::conversation::session::api::subtree_of(
                    history.as_ref(),
                    &root_owned,
                ),
            )
        })?;
        if !tree.iter().any(|s| self.jobs.is_running(s)) {
            return Ok(Vec::new());
        }
        // 等用户裁决的卡先作废（停止 = 拒绝），生成线程才不会卡在确认上。
        self.void_gates(root);
        let stopped = self.stop_tree(root)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let jobs_busy = tree.iter().any(|s| self.jobs.is_running(s));
            let ids = tree.clone();
            let core_busy = self.call(move |core| Ok(core.turn.running_any(&ids)))?;
            if !jobs_busy && !core_busy {
                return Ok(stopped);
            }
            if std::time::Instant::now() >= deadline {
                return Err(
                    "整棵子树里有回合 30 秒内没停下来：这次操作没有动文件与转录，请稍后重试"
                        .to_string(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// 目的：把一次回档结果发布成事件台上的重置快照（受影响的每个会话各一条 `rewound`）。
    /// 约束：主/子会话都要发——子会话也有自己的历史视图与刷新路径。
    pub(crate) fn publish_rewound(&self, sid: &str, outcome: &RewindOutcome) {
        for (child, events) in &outcome.children {
            self.bus.push_rewound(child, events.clone());
        }
        self.bus.push_rewound(sid, outcome.main.clone());
    }

    /// 目的：**级联停止**：把 root 及其整棵子树（按 `meta.parent`）里正在生成的会话都停下来。 返回实际停下的会话 id——不假装“停止了一个本来就没在跑的会话”。 代理核心的 `stop` 走这条：主会话停 = 所有相关工作一起停（用户看得见的那一下）。
    pub(crate) fn stop_tree(&self, root: &str) -> Result<Vec<String>, String> {
        let root_owned = root.to_string();
        let tree = self.call(move |core| {
            let history = core.history_handle();
            Ok(
                crate::units::business::conversation::session::api::subtree_of(
                    history.as_ref(),
                    &root_owned,
                ),
            )
        })?;
        let mut stopped = Vec::new();
        for sid in tree {
            // 在等的工具确认由「停止 = 整队作废」解开（见 RuntimeOps::stop），这里只管取消生成。
            if self.jobs.stop(&sid) {
                stopped.push(sid);
            }
        }
        Ok(stopped)
    }

    /// 目的：把 root 及其整棵子树里等用户裁决的队**一律作废**（停止 = 拒绝），并落盘 + 外送。
    /// 约束：作废是会话的事实——与回答同一条路（落进转录），重启后按它重建，作废过的卡不再挂起。
    pub(crate) fn void_gates(&self, root: &str) {
        let root_owned = root.to_string();
        let Ok(voids) = self.call(move |core| Ok(core.turn.void_gates_in_subtree(&root_owned)))
        else {
            return;
        };
        for (sid, events) in voids {
            let warn = self
                .call({
                    let sid = sid.clone();
                    let events = events.clone();
                    move |core| Ok(core.turn.persister(&sid).persist(&events))
                })
                .ok()
                .flatten();
            self.bus.push(&sid, &events);
            if let Some(w) = warn {
                self.bus
                    .push(&sid, std::slice::from_ref(&SessionEvent::Notice(w)));
            }
        }
    }
}
