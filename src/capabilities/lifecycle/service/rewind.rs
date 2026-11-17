//! 目的：跨会话回档编排——主会话回到某一行（含该行），各 agent 子会话按同一个回合 id 同步截断，然后把新行与元信息写回。
//! 管：留档 / 删除 / 还原三模式的顺序（先停稳整棵子树、再动文件、最后写标记）、子会话折叠、副作用告知（工具执行次数）。
//! 不管：纯行与事件算术（session 的 domain/rewind.rs）、落盘机制（session 的 HistoryStore 端口）。
//! 联动：runtime 的回档入口与改需求经这里；`Turn` / `RebuildDeps` 都是方法形参、不是字段。

use super::*;
use crate::capabilities::lifecycle::api::{RebuildDeps, RewindOutcome, RewindTarget};
use crate::capabilities::conversation::session::api::{subtree_of, work_root};

/// 目的：子会话回档结果——`agent 名 → 该会话的保留行` + `子会话 → 回档后的活动视图`。
pub(crate) type RewindChildren = (
    std::collections::BTreeMap<String, u64>,
    Vec<(String, Vec<serde_json::Value>)>,
);

/// 目的：子会话折叠计划——各 agent 的保留行 + 每个子会话的 `(子会话, 保留行, 新行起点)`。
pub(crate) type ChildFolds = (
    std::collections::BTreeMap<String, u64>,
    Vec<(String, u64, u64)>,
);

/// 目的：一次留档的公共标记内容（主会话与各子会话共用同一个 mark、指针与标签）。
struct ArchiveStamp {
    mark: u64,
    shared: Option<u64>,
    heads: std::collections::BTreeMap<String, Option<u64>>,
    label: String,
}

impl Lifecycle {
    /// 目的：回档——**留档**（标记 + 折叠，可恢复）、**删除**（真的截断）或**恢复**（在该标记处截断）。
    /// 返回：主会话的活动视图，供前端整体重建；三种模式都同步整棵子树。
    #[cfg(test)]
    pub(crate) fn rewind(
        &self,
        turn: &mut Turn,
        deps: RebuildDeps,
        target: &RewindTarget,
        sid: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        Ok(self.rewind_outcome(turn, deps, target, sid, false)?.main)
    }

    /// 目的：回档的完整结果——主会话活动视图 + 受影响的非运行子会话活动视图。
    /// 约束：主视图回给调用方；每份视图再由动作层作为一条 `rewound` 推上事件台，供刷新与多端重置。
    pub(crate) fn rewind_outcome(
        &self,
        turn: &mut Turn,
        deps: RebuildDeps,
        target: &RewindTarget,
        sid: &str,
        force: bool,
    ) -> Result<RewindOutcome, String> {
        if turn.is_running(sid) {
            return Err(Turn::running_refusal(sid));
        }
        let (meta, raw) = self.history.load(sid)?;
        let next_line = crate::capabilities::conversation::session::api::next_line_id(&raw);
        // 说明：`align_keep` 对「找不到目标行」返回 u64::MAX = 保留全部；删除模式允许这种写法，
        // 留档模式不能把 MAX 写进标记（下面单独拒绝）。另记这次回档移出的工具执行次数，用于告知"副作用不在回档范围内"。
        let before_tools = crate::capabilities::conversation::collab::api::tool_runs(
            &crate::capabilities::conversation::session::api::truncate_events(&raw),
        );
        let work = work_root(self.history.as_ref(), sid)?;
        if force {
            // 用户已确认覆盖：先把受管路径的外部改动采纳成提交，随后的回档就不会再撞上“被外部改动”。
            self.workspace
                .work_sync_tracked(&work, crate::kernel::api::unix_seconds())?;
        }
        let (mut active, children) = match *target {
            RewindTarget::Archive(line) => {
                let before = crate::capabilities::conversation::session::api::align_keep(&raw, line);
                if before == u64::MAX {
                    return Err(format!(
                        "回档目标行 {} 不在当前转录里；可能已被回档或删除，请刷新后重试",
                        line
                    ));
                }
                let from = next_line;
                let mark = Self::next_mark_id(&raw);
                let keep_turn = crate::capabilities::conversation::session::api::turn_of_line(&raw, before);
                let label = crate::capabilities::conversation::session::api::archive_label(&raw, before, from);
                // 先算各子会话的折叠点（不落盘），再算共享区点：两者都成功才动文件。
                let (kids, folds) = self.archive_children_plan(turn, sid, keep_turn)?;
                let keep = Self::keep_by_agent(&meta, before, &kids);
                let agents = self.archive_agents(sid, &meta)?;
                let report = self.workspace.work_archive(&work, &keep, &agents, mark)?;
                let heads: std::collections::BTreeMap<String, Option<u64>> =
                    agents.iter().map(|a| (a.clone(), report.shared)).collect();
                let stamp = ArchiveStamp {
                    mark,
                    shared: report.shared,
                    heads,
                    label,
                };
                // 文件已就位，最后写转录标记（顺序反了会出现"标记说已留档、文件还在新枝"）。
                self.write_archive_mark(sid, &stamp, before, from)?;
                let mut rewound = Vec::new();
                for (kid, kid_before, kid_from) in folds {
                    self.write_archive_mark(&kid, &stamp, kid_before, kid_from)?;
                    let (kmeta, kraw) = self.history.load(&kid)?;
                    let next = crate::capabilities::conversation::session::api::next_line_id(&kraw);
                    let active = self.restore_session(turn, deps, &kid, &kmeta, &kraw, next)?;
                    rewound.push((kid, active));
                }
                let (_, raw_after) = self.history.load(sid)?;
                let next = crate::capabilities::conversation::session::api::next_line_id(&raw_after);
                let active = self.restore_session(turn, deps, sid, &meta, &raw_after, next)?;
                (active, rewound)
            }
            RewindTarget::Delete(line) => {
                let before = crate::capabilities::conversation::session::api::align_keep(&raw, line);
                // before = u64::MAX = 「保留全部」：删不到任何东西，转录与文件都不动。
                if before == u64::MAX {
                    let (_kids, rewound) = self.cut_children(turn, deps, sid, u64::MAX)?;
                    let kept = crate::capabilities::conversation::session::api::cut_before_line(&raw, before);
                    let next = crate::capabilities::conversation::session::api::next_line_id(&kept);
                    let active = self.restore_session(turn, deps, sid, &meta, &kept, next)?;
                    return Ok(RewindOutcome {
                        main: active,
                        children: rewound,
                    });
                }
                let kept = crate::capabilities::conversation::session::api::cut_before_line(&raw, before);
                // 删除会截掉转录里的留档标记；版本库里的 refs/archive/* 必须跟着作废，
                // 否则留下"文件里有档案、转录里找不到"的孤儿。先算清楚并在确认前报数量。
                let kept_marks: Vec<u64> = crate::capabilities::conversation::session::api::rewind_marks(&kept)
                    .iter()
                    .map(|m| m.mark)
                    .collect();
                let all_marks = self.workspace.work_archives(&work)?;
                let doomed: Vec<u64> = all_marks
                    .iter()
                    .copied()
                    .filter(|m| !kept_marks.contains(m))
                    .collect();
                if !doomed.is_empty() && !force {
                    let list = doomed
                        .iter()
                        .map(|m| format!("#{}", m))
                        .collect::<Vec<_>>()
                        .join("、");
                    return Err(format!(
                        "删除会连同工作区文件一起回退，并作废 {} 个留档（{}）：它们的标记在这条删除线之后，作废后不可恢复。确认后继续。",
                        doomed.len(),
                        list
                    ));
                }
                let keep_turn = crate::capabilities::conversation::session::api::turn_of_line(&raw, before);
                let (kids, rewound) = self.cut_children(turn, deps, sid, keep_turn)?;
                let keep = Self::keep_by_agent(&meta, before, &kids);
                let agents = self.archive_agents(sid, &meta)?;
                // 共享区与每个受影响沙箱一起落到删除点；该点之后的提交真的丢掉（删除就是删除）。
                self.workspace
                    .work_delete(&work, &keep, &agents, &kept_marks)?;
                self.history
                    .replace(sid, &kept)
                    .map_err(|e| format!("删除落盘失败：{}", e))?;
                // 删除的算子是"截掉尾部"：单 agent 且压缩点没被切到时**就地精确回退**（不重建，
                // 保住通道与运行态）；否则按转录重建（协作与跨压缩点都得重建）。
                let precise = match turn.session(sid) {
                    // 压缩记录行被截在保留点之后才就地回退；截到它本身或更早必须重建。
                    Some(Session::Single(s)) => s.compacted_line == 0 || before > s.compacted_line,
                    _ => false,
                };
                let active = if precise {
                    if let Some(Session::Single(s)) = turn.session_mut(sid) {
                        s.apply_rewind(before);
                        // 截掉的尾部里的 usage 事件也随 kept 过滤；状态栏不能继续显示旧数字。
                        s.set_last_usage(crate::capabilities::conversation::session::api::last_usage(&kept));
                    }
                    // 就地回退没重建对象：shell 快照也按截断后的转录重算（被截掉的 shell 不能继续显示）。
                    turn.shell_slot(sid)
                        .set(crate::capabilities::conversation::session::api::last_shell(&kept));
                    crate::capabilities::conversation::session::api::truncate_events(&kept)
                } else {
                    let next = crate::capabilities::conversation::session::api::next_line_id(&kept);
                    self.restore_session(turn, deps, sid, &meta, &kept, next)?
                };
                (active, rewound)
            }
            RewindTarget::Restore(mark) => {
                let idx = raw
                    .iter()
                    .position(|ev| {
                        ev.get("type").and_then(|t| t.as_str()) == Some("rewind")
                            && ev.get("mark").and_then(|m| m.as_u64()) == Some(mark)
                    })
                    .ok_or_else(|| format!("没有回档标记 {}（可能已被恢复）", mark))?;
                // 先恢复文件（失败则转录原封不动），再展开转录；提交一条不删。
                self.workspace.work_restore_archive(&work, mark)?;
                let kept: Vec<serde_json::Value> = raw[..idx].to_vec();
                self.history
                    .replace(sid, &kept)
                    .map_err(|e| format!("恢复落盘失败：{}", e))?;
                let (_kids, rewound) = self.unfold_children(turn, deps, sid, mark)?;
                let next = crate::capabilities::conversation::session::api::next_line_id(&kept);
                let active = self.restore_session(turn, deps, sid, &meta, &kept, next)?;
                (active, rewound)
            }
        };
        let after_tools = crate::capabilities::conversation::collab::api::tool_runs(&active);
        let dropped = before_tools.saturating_sub(after_tools);
        if dropped > 0 {
            active.push(serde_json::json!({
                "type": "notice",
                "text": format!(
                    "[提示] 这次回档移出了 {} 次工具执行——它们的副作用不在回档范围内（只有共享区的提交会一起回退），继续可能会重新执行。",
                    dropped
                ),
            }));
        }
        Ok(RewindOutcome {
            main: active,
            children,
        })
    }

    /// 目的：重建并放回会话（回档后状态全部派生）。
    /// 参数：next = 新行续号（全量最大 id + 1，永不回退）。
    /// 返回：这次重建采用的活动视图（回档重置事件的载荷）。
    pub(crate) fn restore_session(
        &self,
        turn: &mut Turn,
        deps: RebuildDeps,
        sid: &str,
        meta: &SessionMeta,
        raw: &[serde_json::Value],
        next: u64,
    ) -> Result<Vec<serde_json::Value>, String> {
        let active = crate::capabilities::conversation::session::api::truncate_events(raw);
        let mut rebuilt = turn.rebuild_session(meta, &active, deps.registry, deps.probe)?;
        match &mut rebuilt {
            Session::Single(s) => s.next_line = next,
            Session::Collab(c) => c.next_line = next,
        }
        turn.put(sid.to_string(), rebuilt);
        Ok(active)
    }

    /// 目的：流水里下一个回档标记 id（现存量 + 1）。
    /// 约束：只认留档标记；删除与恢复不写标记，因此不占号。
    pub(crate) fn next_mark_id(raw: &[serde_json::Value]) -> u64 {
        crate::capabilities::conversation::session::api::rewind_marks(raw)
            .iter()
            .map(|m| m.mark)
            .max()
            .unwrap_or(0)
            + 1
    }

    /// 各会话保留行 → 共享区提交锚。空串 = 用户投喂（主会话）；单 agent 会话自己就跑工作工具，
    /// 它的 agent 名也锚到同一条保留行。
    fn keep_by_agent(
        meta: &SessionMeta,
        before: u64,
        kids: &std::collections::BTreeMap<String, u64>,
    ) -> std::collections::BTreeMap<String, u64> {
        let mut keep = std::collections::BTreeMap::new();
        keep.insert(String::new(), before);
        if meta.mode == "single" {
            for a in &meta.agents {
                keep.insert(a.name.clone(), before);
            }
        }
        for (agent, line) in kids {
            keep.insert(agent.clone(), *line);
        }
        keep
    }

    /// 目的：留档前先算各子会话的折叠点，不落盘、不动文件。
    /// 返回：`agent 名 → 保留行`（供共享区对齐）+ `(子会话, 保留行, 新行起点)`。
    /// 约束：在跑的会话给 `u64::MAX`（共享区保住它的提交）；它的折叠由收尾路径自己落盘。
    fn archive_children_plan(
        &self,
        turn: &Turn,
        sid: &str,
        keep_turn: u64,
    ) -> Result<ChildFolds, String> {
        let kids: Vec<String> = subtree_of(self.history.as_ref(), sid)
            .into_iter()
            .skip(1)
            .collect();
        let mut keeps = std::collections::BTreeMap::new();
        let mut folds = Vec::new();
        for kid in kids {
            let (meta, raw) = self.history.load(&kid)?;
            let agent = meta
                .agents
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_default();
            if turn.is_running(&kid) {
                if !agent.is_empty() {
                    keeps.insert(agent, u64::MAX);
                }
                continue;
            }
            let active = crate::capabilities::conversation::session::api::truncate_events(&raw);
            let before = crate::capabilities::conversation::session::api::last_line_within(&active, keep_turn);
            let from = crate::capabilities::conversation::session::api::next_line_id(&raw);
            if !agent.is_empty() {
                keeps.insert(agent, before);
            }
            folds.push((kid, before, from));
        }
        Ok((keeps, folds))
    }

    /// 目的：这次留档要一起回滚的 agent 名单（主/子会话名单的并集）。
    fn archive_agents(&self, sid: &str, meta: &SessionMeta) -> Result<Vec<String>, String> {
        let mut out: std::collections::BTreeSet<String> =
            meta.agents.iter().map(|a| a.name.clone()).collect();
        for kid in subtree_of(self.history.as_ref(), sid).into_iter().skip(1) {
            let (km, _) = self.history.load(&kid)?;
            if let Some(a) = km.agents.first() {
                out.insert(a.name.clone());
            }
        }
        Ok(out.into_iter().collect())
    }

    /// 目的：把一条留档标记追加到某个会话的流水里（主/子用同一个 mark 与同一份指针）。
    fn write_archive_mark(
        &self,
        sid: &str,
        stamp: &ArchiveStamp,
        before: u64,
        from: u64,
    ) -> Result<(), String> {
        self.history
            .append(
                sid,
                &[serde_json::json!({
                    "type": "rewind",
                    "mark": stamp.mark,
                    "mode": "archive",
                    "before": before,
                    "from": from,
                    "shared": stamp.shared,
                    "heads": stamp.heads,
                    "label": stamp.label
                })],
            )
            .map_err(|e| format!("会话 {} 留档落盘失败：{}", sid, e))
    }

    /// 目的：按回合边界截断整棵子树的会话（删除模式），返回共享区对齐用的保留行。
    fn cut_children(
        &self,
        turn: &mut Turn,
        deps: RebuildDeps,
        sid: &str,
        keep_turn: u64,
    ) -> Result<RewindChildren, String> {
        let kids: Vec<String> = subtree_of(self.history.as_ref(), sid)
            .into_iter()
            .skip(1)
            .collect();
        let mut keeps = std::collections::BTreeMap::new();
        let mut rewound: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
        for kid in kids {
            let (meta, raw) = self.history.load(&kid)?;
            let agent = meta
                .agents
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_default();
            if turn.is_running(&kid) {
                if !agent.is_empty() {
                    keeps.insert(agent, u64::MAX);
                }
                continue;
            }
            let active = crate::capabilities::conversation::session::api::truncate_events(&raw);
            let before = crate::capabilities::conversation::session::api::last_line_within(&active, keep_turn);
            let kept = crate::capabilities::conversation::session::api::cut_before_line(&raw, before);
            self.history
                .replace(&kid, &kept)
                .map_err(|e| format!("子会话 {} 删除落盘失败：{}", kid, e))?;
            let next = crate::capabilities::conversation::session::api::next_line_id(&kept);
            let active = self.restore_session(turn, deps, &kid, &meta, &kept, next)?;
            rewound.push((kid.clone(), active));
            if !agent.is_empty() {
                keeps.insert(agent, before);
            }
        }
        Ok((keeps, rewound))
    }

    /// 目的：在同一个 mark 处展开整棵子树的转录（恢复模式）。
    fn unfold_children(
        &self,
        turn: &mut Turn,
        deps: RebuildDeps,
        sid: &str,
        mark: u64,
    ) -> Result<RewindChildren, String> {
        let kids: Vec<String> = subtree_of(self.history.as_ref(), sid)
            .into_iter()
            .skip(1)
            .collect();
        let mut rewound: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
        for kid in kids {
            let (meta, raw) = self.history.load(&kid)?;
            if turn.is_running(&kid) {
                continue;
            }
            let Some(idx) = raw.iter().position(|ev| {
                ev.get("type").and_then(|t| t.as_str()) == Some("rewind")
                    && ev.get("mark").and_then(|m| m.as_u64()) == Some(mark)
            }) else {
                continue;
            };
            let kept: Vec<serde_json::Value> = raw[..idx].to_vec();
            self.history
                .replace(&kid, &kept)
                .map_err(|e| format!("子会话 {} 恢复落盘失败：{}", kid, e))?;
            let next = crate::capabilities::conversation::session::api::next_line_id(&kept);
            let active = self.restore_session(turn, deps, &kid, &meta, &kept, next)?;
            rewound.push((kid.clone(), active));
        }
        Ok((std::collections::BTreeMap::new(), rewound))
    }
}
