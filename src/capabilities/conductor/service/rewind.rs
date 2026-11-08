//! **跨会话回档编排**：主会话回到某一行（含该行），各 agent 子会话按同一个**回合 id** 同步截断，
//! 然后把新的行与元信息写回；协作/历史会话按转录重建（状态全部派生）。
//!
//! 纯行与事件算术在 session 的 domain/rewind.rs；回档的**副作用告知**（工具执行次数）也在这里。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate) 供兄弟族（会话生命周期、改需求）调用。

use super::*;
use crate::capabilities::conductor::api::RewindOutcome;

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

impl Conductor {
    /// 目的：回档——**留档**（标记 + 折叠，可恢复）、**删除**（真的截断）或**恢复**（在该标记处截断）。
    /// 返回：主会话的活动视图，供前端整体重建；三种模式都同步整棵子树。
    #[cfg(test)]
    pub(crate) fn rewind(
        &mut self,
        sid: &str,
        target: RewindTarget,
    ) -> Result<Vec<serde_json::Value>, String> {
        Ok(self.rewind_outcome(sid, target, false)?.main)
    }

    /// 目的：回档的完整结果——主会话活动视图 + 受影响的非运行子会话活动视图。
    /// 约束：主视图回给调用方；每份视图再由动作层作为一条 `rewound` 推上事件台，供刷新与多端重置。
    pub(crate) fn rewind_outcome(
        &mut self,
        sid: &str,
        target: RewindTarget,
        force: bool,
    ) -> Result<RewindOutcome, String> {
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        let (meta, raw) = self.history.load(sid)?;
        let next_line = crate::capabilities::session::api::next_line_id(&raw);
        // 说明：`align_keep` 对「找不到目标行」返回 u64::MAX = 保留全部；删除模式允许这种写法，
        // 留档模式不能把 MAX 写进标记（下面单独拒绝）。另记这次回档移出的工具执行次数，用于告知"副作用不在回档范围内"。
        let before_tools = crate::capabilities::collab::api::tool_runs(
            &crate::capabilities::session::api::truncate_events(&raw),
        );
        let work = self.work_root(sid)?;
        if force {
            // 用户已确认覆盖：先把受管路径的外部改动采纳成提交，随后的回档就不会再撞上“被外部改动”。
            self.workspace
                .work_sync_tracked(&work, crate::capabilities::conductor::service::now_ts())?;
        }
        let (mut active, children) = match target {
            RewindTarget::Archive(line) => {
                let before = crate::capabilities::session::api::align_keep(&raw, line);
                if before == u64::MAX {
                    return Err(format!(
                        "回档目标行 {} 不在当前转录里；可能已被回档或删除，请刷新后重试",
                        line
                    ));
                }
                let from = next_line;
                let mark = Self::next_mark_id(&raw);
                let turn = crate::capabilities::session::api::turn_of_line(&raw, before);
                let label = crate::capabilities::session::api::archive_label(&raw, before, from);
                // 先算各子会话的折叠点（不落盘），再算共享区点：两者都成功才动文件。
                let (kids, folds) = self.archive_children_plan(sid, turn)?;
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
                    let next = crate::capabilities::session::api::next_line_id(&kraw);
                    let active = self.restore_session(&kid, &kmeta, &kraw, next)?;
                    rewound.push((kid, active));
                }
                let (_, raw_after) = self.history.load(sid)?;
                let next = crate::capabilities::session::api::next_line_id(&raw_after);
                let active = self.restore_session(sid, &meta, &raw_after, next)?;
                (active, rewound)
            }
            RewindTarget::Delete(line) => {
                let before = crate::capabilities::session::api::align_keep(&raw, line);
                // before = u64::MAX = 「保留全部」：删不到任何东西，转录与文件都不动。
                if before == u64::MAX {
                    let (_kids, rewound) = self.cut_children(sid, u64::MAX)?;
                    let kept = crate::capabilities::session::api::cut_before_line(&raw, before);
                    let next = crate::capabilities::session::api::next_line_id(&kept);
                    let active = self.restore_session(sid, &meta, &kept, next)?;
                    return Ok(RewindOutcome {
                        main: active,
                        children: rewound,
                    });
                }
                let kept = crate::capabilities::session::api::cut_before_line(&raw, before);
                // 删除会截掉转录里的留档标记；版本库里的 refs/archive/* 必须跟着作废，
                // 否则留下"文件里有档案、转录里找不到"的孤儿。先算清楚并在确认前报数量。
                let kept_marks: Vec<u64> = crate::capabilities::session::api::rewind_marks(&kept)
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
                let turn = crate::capabilities::session::api::turn_of_line(&raw, before);
                let (kids, rewound) = self.cut_children(sid, turn)?;
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
                let precise = match self.sessions.get(sid) {
                    // 压缩记录行被截在保留点之后才就地回退；截到它本身或更早必须重建。
                    Some(Session::Single(s)) => s.compacted_line == 0 || before > s.compacted_line,
                    _ => false,
                };
                let active = if precise {
                    if let Some(Session::Single(s)) = self.sessions.get_mut(sid) {
                        s.apply_rewind(before);
                        // 截掉的尾部里的 usage 事件也随 kept 过滤；状态栏不能继续显示旧数字。
                        s.set_last_usage(crate::capabilities::session::api::last_usage(&kept));
                    }
                    // 就地回退没重建对象：shell 快照也按截断后的转录重算（被截掉的 shell 不能继续显示）。
                    self.shell_slot(sid)
                        .set(crate::capabilities::session::api::last_shell(&kept));
                    crate::capabilities::session::api::truncate_events(&kept)
                } else {
                    let next = crate::capabilities::session::api::next_line_id(&kept);
                    self.restore_session(sid, &meta, &kept, next)?
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
                let (_kids, rewound) = self.unfold_children(sid, mark)?;
                let next = crate::capabilities::session::api::next_line_id(&kept);
                let active = self.restore_session(sid, &meta, &kept, next)?;
                (active, rewound)
            }
        };
        let after_tools = crate::capabilities::collab::api::tool_runs(&active);
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
        &mut self,
        sid: &str,
        meta: &SessionMeta,
        raw: &[serde_json::Value],
        next: u64,
    ) -> Result<Vec<serde_json::Value>, String> {
        let active = crate::capabilities::session::api::truncate_events(raw);
        let mut rebuilt = self.rebuild_session(meta, &active)?;
        match &mut rebuilt {
            Session::Single(s) => s.next_line = next,
            Session::Collab(c) => c.next_line = next,
        }
        self.sessions.insert(sid.to_string(), rebuilt);
        Ok(active)
    }

    /// 流水里下一个回档标记 id（现存量 + 1）。
    fn next_mark_id(raw: &[serde_json::Value]) -> u64 {
        crate::capabilities::session::api::rewind_marks(raw)
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
    fn archive_children_plan(&mut self, sid: &str, keep_turn: u64) -> Result<ChildFolds, String> {
        let kids: Vec<String> = self.subtree_of(sid).into_iter().skip(1).collect();
        let mut keeps = std::collections::BTreeMap::new();
        let mut folds = Vec::new();
        for kid in kids {
            let (meta, raw) = self.history.load(&kid)?;
            let agent = meta
                .agents
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_default();
            if self.running.contains(&kid) {
                if !agent.is_empty() {
                    keeps.insert(agent, u64::MAX);
                }
                continue;
            }
            let active = crate::capabilities::session::api::truncate_events(&raw);
            let before = crate::capabilities::session::api::last_line_within(&active, keep_turn);
            let from = crate::capabilities::session::api::next_line_id(&raw);
            if !agent.is_empty() {
                keeps.insert(agent, before);
            }
            folds.push((kid, before, from));
        }
        Ok((keeps, folds))
    }

    /// 目的：这次留档要一起回滚的 agent 名单（主/子会话名单的并集）。
    fn archive_agents(&mut self, sid: &str, meta: &SessionMeta) -> Result<Vec<String>, String> {
        let mut out: std::collections::BTreeSet<String> =
            meta.agents.iter().map(|a| a.name.clone()).collect();
        for kid in self.subtree_of(sid).into_iter().skip(1) {
            let (km, _) = self.history.load(&kid)?;
            if let Some(a) = km.agents.first() {
                out.insert(a.name.clone());
            }
        }
        Ok(out.into_iter().collect())
    }

    /// 目的：把一条留档标记追加到某个会话的流水里（主/子用同一个 mark 与同一份指针）。
    fn write_archive_mark(
        &mut self,
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
    fn cut_children(&mut self, sid: &str, keep_turn: u64) -> Result<RewindChildren, String> {
        let kids: Vec<String> = self.subtree_of(sid).into_iter().skip(1).collect();
        let mut keeps = std::collections::BTreeMap::new();
        let mut rewound: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
        for kid in kids {
            let (meta, raw) = self.history.load(&kid)?;
            let agent = meta
                .agents
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_default();
            if self.running.contains(&kid) {
                if !agent.is_empty() {
                    keeps.insert(agent, u64::MAX);
                }
                continue;
            }
            let active = crate::capabilities::session::api::truncate_events(&raw);
            let before = crate::capabilities::session::api::last_line_within(&active, keep_turn);
            let kept = crate::capabilities::session::api::cut_before_line(&raw, before);
            self.history
                .replace(&kid, &kept)
                .map_err(|e| format!("子会话 {} 删除落盘失败：{}", kid, e))?;
            let next = crate::capabilities::session::api::next_line_id(&kept);
            let active = self.restore_session(&kid, &meta, &kept, next)?;
            rewound.push((kid.clone(), active));
            if !agent.is_empty() {
                keeps.insert(agent, before);
            }
        }
        Ok((keeps, rewound))
    }

    /// 目的：在同一个 mark 处展开整棵子树的转录（恢复模式）。
    fn unfold_children(&mut self, sid: &str, mark: u64) -> Result<RewindChildren, String> {
        let kids: Vec<String> = self.subtree_of(sid).into_iter().skip(1).collect();
        let mut rewound: Vec<(String, Vec<serde_json::Value>)> = Vec::new();
        for kid in kids {
            let (meta, raw) = self.history.load(&kid)?;
            if self.running.contains(&kid) {
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
            let next = crate::capabilities::session::api::next_line_id(&kept);
            let active = self.restore_session(&kid, &meta, &kept, next)?;
            rewound.push((kid.clone(), active));
        }
        Ok((std::collections::BTreeMap::new(), rewound))
    }

    /// 按会话元信息 + 转录事件重建会话对象（通道是机制，按记录的选择重新装配）。
    pub(crate) fn rebuild_session(
        &self,
        meta: &SessionMeta,
        events: &[serde_json::Value],
    ) -> Result<Session, String> {
        let roster = self.modules.roster();
        // 打开 / 重建时按 meta 刷新布局投影（会话目录可能被复制或迁移；meta 是权威）。
        let work = self.work_root_of(meta)?;
        let binding = self.workspace_binding_of(meta)?;
        self.workspace.bind(&work, binding.as_deref())?;
        if binding.is_some() {
            // 旧会话首次启用：补一次初始导入（有 head 时无操作）。
            if let Err(e) = self.workspace.work_import_root(&work, now_ts()) {
                self.log.warn(
                    "conductor::rebuild_session",
                    &format!("工作区初始导入失败：{}", e),
                );
            }
        }
        let sandboxes = self.sandboxes(meta, &roster)?;
        // 整队的权威是**转录**：重建之前先把队清空并按已发出的卡号续号（内存里那一份不作数）。
        self.desk_of(&meta.name)
            .rebuild(crate::capabilities::session::api::issued_in(events));
        let built = match meta.mode.as_str() {
            "collab" => Session::Collab(CollabSession::restore(
                Arc::clone(&self.llm),
                Arc::clone(&self.vm),
                Arc::clone(&self.modules),
                self.registry.snapshot(),
                Arc::clone(&self.prompt),
                Arc::clone(&self.systools),
                Arc::clone(&self.tools),
                Arc::clone(&self.log),
                meta,
                events,
                sandboxes,
                self.guest_lease_for(meta)?,
                Arc::clone(&self.workspace),
                // 重建出来的整队进**同一条队**（按转录里已发出的卡号续号，已答过的不重问）。
                self.desk_of(&meta.name),
            )?),
            // 单 agent：按 meta.agents[0] 重建（名单是唯一真相；模块数不限）。
            "single" => {
                let a = meta
                    .agents
                    .first()
                    .ok_or_else(|| format!("会话 {} 缺少 agent 名单", meta.name))?;
                let modules: Vec<Module> = a
                    .modules
                    .iter()
                    .filter_map(|id| {
                        roster
                            .modules
                            .iter()
                            .find(|m| &m.manifest.id == id)
                            .cloned()
                    })
                    .collect();
                if modules.len() != a.modules.len() {
                    return Err(format!("agent {} 的模块已不在清单", a.name));
                }
                let sb = sandboxes.for_agent(&a.name).cloned().ok_or_else(|| {
                    format!("会话 {} 缺少 agent {} 的沙箱信息", meta.name, a.name)
                })?;
                let channel = self
                    .registry
                    .channel(a.model.as_deref())
                    .map_err(|e| format!("agent {} 没有可用模型通道：{}", a.name, e))?;
                // 重建时同样按登记处派生形态：系统提示与实际协议必须一致（回放才与实时一致）
                let mode = self.registry.tool_mode(a.model.as_deref());
                // **会话参数**：与建立时同一个口径（身份块每回合现渲染，不进消息列表）。
                let params = crate::capabilities::session::api::SessionParams::from_workspace(
                    &a.name,
                    &sb,
                    &modules,
                    &self.session_kind_of(meta)?,
                    &self.role_of(meta)?,
                )
                .with_chara(a.chara.clone())
                .with_guest(self.guest_os_of(&meta.exec));
                let chat = self.llm.member_channel(&channel, &a.name);
                let (history, marks, line_reply, line_ids, compacted_upto, compacted_line) =
                    replay_dialogue(events, mode);
                let unavailable = self.unavailable_modules(&meta.exec, &modules);
                // 身份与角色**同一把尺子**（重建与实时不能各写一份，见 Conductor::role_of）：
                // 协作派生的成员 / 节点 = executor（拿得到回报工具）；其余单 agent 工作 = solo。
                let role = self.role_of(meta)?;
                let host_plan = if meta.exec.tier == crate::kernel::api::Tier::Host {
                    self.host_plan_for(
                        &meta.exec,
                        &modules,
                        crate::capabilities::packages::api::PinPolicy::Require,
                    )
                } else {
                    crate::capabilities::packages::api::PackagePlan::default()
                };
                let mut tools = self.tools_env(
                    &modules,
                    &sb,
                    unavailable,
                    self.network_allowed(),
                    mode,
                    &role,
                    meta.exec.tier,
                    self.guest_lease_for(meta)?,
                    &host_plan,
                );
                // 回复 id 跨重启单调：从转录里的最大值续号，否则新回复会与旧回复并成一组。
                tools.reply_seq = crate::capabilities::session::api::max_reply(events);
                let mut s = crate::capabilities::session::api::AgentSession::restore(
                    &a.name,
                    params,
                    history,
                    marks,
                    line_reply,
                    line_ids,
                    compacted_upto,
                    compacted_line,
                    chat,
                    Some(tools),
                    self.prompt
                        .text(crate::capabilities::prompt::api::Segment::CompactPrompt)
                        .to_string(),
                );
                // 重建也要恢复上下文预算与最近一次用量：否则重启后自动压缩与状态栏都失忆。
                let (window, percent, keep_percent) = self.context_budget(a.model.as_deref());
                s.set_context_budget(window, percent, keep_percent);
                s.set_model(a.model.clone().or_else(|| self.registry.core_model()));
                s.set_compact_factory(self.compact_factory());
                s.set_context_freshness(meta.context_freshness);
                for (body, fact) in replay_reads(events) {
                    s.note_read_at(body, fact);
                }
                s.set_last_usage(crate::capabilities::session::api::last_usage(events));
                Session::Single(s)
            }
            // 代理会话：没有 agent（核心自己说话）；外壳由 build_proxy 装配，回放与单 agent 共用同一件。
            "proxy" => {
                let mut s = self.build_proxy(meta)?;
                let (history, marks, line_reply, line_ids, compacted_upto, compacted_line) =
                    replay_dialogue(events, s.tool_mode());
                s.dialogue = history;
                s.marks = marks;
                s.line_reply = line_reply;
                s.line_ids = line_ids;
                s.compacted_upto = compacted_upto;
                s.compacted_line = compacted_line;
                s.next_line = s.marks.len() as u64;
                if let Some(t) = s.tools.as_mut() {
                    t.reply_seq = crate::capabilities::session::api::max_reply(events);
                }
                s.set_context_freshness(meta.context_freshness);
                for (body, fact) in replay_reads(events) {
                    s.note_read_at(body, fact);
                }
                s.set_last_usage(crate::capabilities::session::api::last_usage(events));
                Session::Single(s)
            }
            other => {
                return Err(format!(
                    "未知会话形态：{}（只认 single / collab / proxy）",
                    other
                ))
            }
        };
        // 状态栏的 shell 快照同样按转录派生：重启 / 启用 / 回档重建后不再"失忆"。
        self.shell_slot(&meta.name)
            .set(crate::capabilities::session::api::last_shell(events));
        Ok(built)
    }
}
/// 目的：把转录里的读事实重新装回读台账（重建后的发送视图与实时一致）。
///
/// 约束：正文按 `tool_result_body` 现算，必须与 `reply_msgs` 同源——发送视图按正文找消息。
///   被压缩吃掉的行不会命中任何消息，留着也无害。
fn replay_reads(
    events: &[serde_json::Value],
) -> Vec<(String, crate::capabilities::workspace::api::ReadFact)> {
    let mut out = Vec::new();
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for l in lines {
            let Some(t) = l.get("tool") else {
                continue;
            };
            let Ok(c) = serde_json::from_value::<crate::capabilities::session::api::ToolCallView>(
                t.clone(),
            ) else {
                continue;
            };
            if let Some(fact) = c.read.clone() {
                out.push((
                    crate::capabilities::session::api::tool_result_body(&c),
                    fact,
                ));
            }
        }
    }
    out
}

/// 目的：把转录事件还原成（对话, 每行历史长度, 每行回复号, 每行 id, 压缩点, 压缩行 id）。
/// 约束：重建必须与实时逐条一致（见 `reply_msgs` 与 session-model.md）。
pub(crate) fn replay_dialogue(
    events: &[serde_json::Value],
    mode: crate::capabilities::llm::api::ToolMode,
) -> (
    Vec<crate::capabilities::llm::api::Msg>,
    Vec<usize>,
    Vec<u64>,
    Vec<u64>,
    u64,
    u64,
) {
    // 先把转录行按顺序摊平：分组判断要看「下一行是不是 tool 行」。
    let mut rows: Vec<&serde_json::Value> = Vec::new();
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
            rows.extend(lines.iter());
        }
    }
    // **压缩过**的会话：按最后一条压缩行重建发送视图（见 session-model.md 六）——
    // `up_to` 之前的行不再进对话，由一份摘要代替（转录本身完整保留，用户照样能查）。
    // 回档到压缩行之前时它已随转录被截掉，所以「没有它」就是「回到压缩前」。
    let compacted = crate::capabilities::session::api::last_compaction(events);
    let compacted_upto = compacted.as_ref().map(|(up_to, _, _)| *up_to).unwrap_or(0);
    let compacted_line = compacted.as_ref().map(|(_, _, id)| *id).unwrap_or(0);
    // 对话里**只有**真正发生过的事；身份与环境由 params 现渲染。
    let mut history: Vec<Msg> = compacted
        .as_ref()
        .map(|(_, summary, _)| crate::capabilities::session::api::summary_message(summary))
        .into_iter()
        .collect();
    let mut marks: Vec<usize> = Vec::new();
    let mut line_reply: Vec<u64> = Vec::new();
    let mut line_ids: Vec<u64> = Vec::new();
    let reply_of = |v: &serde_json::Value| v.get("reply").and_then(|x| x.as_u64()).unwrap_or(0);
    let id_of = |v: &serde_json::Value| v.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
    // 回复级原文可能挂在该回复的正文行上（正文行随后被工具组跳过）：先接住再交给工具组。
    let mut pending_raw = String::new();
    let mut i = 0usize;
    while i < rows.len() {
        let l = rows[i];
        let id = id_of(l);
        // 被总结掉的行：不进对话，但仍占一行（marks / line_reply / line_ids 与转录行一一对应）。
        if compacted_upto > 0 && id < compacted_upto {
            if l.get("tool").is_some() {
                // 同一次回复的 tool 行连续同号：整组一起跳，别从中间切开。
                let reply = reply_of(l.get("tool").expect("已判存在"));
                while i < rows.len()
                    && reply_of(rows[i].get("tool").unwrap_or(&serde_json::Value::Null)) == reply
                {
                    line_reply.push(reply);
                    line_ids.push(id_of(rows[i]));
                    marks.push(history.len().max(1));
                    i += 1;
                }
            } else {
                line_reply.push(reply_of(l));
                line_ids.push(id);
                marks.push(history.len().max(1));
                i += 1;
            }
            continue;
        }
        let line = l.get("line").and_then(|x| x.as_str()).unwrap_or("");
        // **读结构化字段**（种类 / 系统标记 / 正文），不从正文里抠 [标签]。
        let kind = l.get("kind").and_then(|x| x.as_str()).unwrap_or("");
        // **压缩记录**只给人看：它不产生对话消息；摘要由最后一条记录行在顶部合成。
        if kind == "compacted" {
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        // **内容类系统提示**只给人看：不进对话，但仍占一行。
        } else if kind == "notice" {
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        // 系统注入的行按它该有的角色还原：提醒/边界是 system，
        // **派发行**（`task`）是 user——否则重建出来的请求又变成一条 user 都没有，供应商照样拒收。
        } else if kind == "system" || l.get("system").and_then(|x| x.as_bool()).unwrap_or(false) {
            let is_task = l.get("task").and_then(|x| x.as_bool()).unwrap_or(false);
            history.push(if is_task {
                Msg::user(line.to_string())
            } else {
                Msg::system(line.to_string())
            });
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        } else if kind == "user" {
            // 用户说的行：正文就是用户那句话（[用户] / [用户:需求] 这类标签在字段里）。
            history.push(Msg::user(line.to_string()));
            // 用户行不属于任何回复：给它自己的行号，回档时才不会与相邻行误并成一组。
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        } else if l.get("tool").is_some() {
            // 【回复分组 · 改动前务必读完】**同一次回复的 tool 行连续同号**（reply 由引擎给、
            // 落行时写入）；整组一起翻译成消息，靠的正是这个号——不靠"相邻行猜分组"。
            let reply = reply_of(l.get("tool").expect("已判存在"));
            let mut group: Vec<&serde_json::Value> = Vec::new();
            while i < rows.len()
                && reply_of(rows[i].get("tool").unwrap_or(&serde_json::Value::Null)) == reply
            {
                group.push(rows[i]);
                i += 1;
            }
            // 这一回复的助手消息原文（每条回复只落一份：可能在正文行上，也可能在组内首行）。
            let raw = if pending_raw.is_empty() {
                group
                    .iter()
                    .find_map(|t| {
                        t.get("raw")
                            .and_then(|x| x.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or_default()
                    .to_string()
            } else {
                std::mem::take(&mut pending_raw)
            };
            let views: Vec<crate::capabilities::session::api::ToolCallView> = group
                .iter()
                .filter_map(|t| t.get("tool").cloned())
                .filter_map(|t| serde_json::from_value(t).ok())
                .collect();
            for m in crate::capabilities::session::api::reply_msgs(mode, &raw, &views) {
                history.push(m);
            }
            for row in &group {
                line_reply.push(reply);
                line_ids.push(id_of(row));
                marks.push(history.len());
            }
        } else {
            // 文本行：它紧跟 tool 行时属于同一次回复（历史由那组 tool 行统一推进，这里不推）；
            // 否则这一行自己就是一条回复，推 assistant(该行文本)。
            let next_is_tool = rows
                .get(i + 1)
                .map(|n| n.get("tool").is_some())
                .unwrap_or(false);
            // 同一次回复的正文行可能带着回复级原文；工具组在下一轮消费它。
            if next_is_tool {
                pending_raw = l
                    .get("raw")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
            } else {
                pending_raw.clear();
            }
            if !next_is_tool {
                // 正文本身就是内容（说话人/动词在字段里），直接进助手消息。
                history.push(Msg::assistant(line.to_string()));
            }
            line_reply.push(reply_of(l));
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        }
    }
    (
        history,
        marks,
        line_reply,
        line_ids,
        compacted_upto,
        compacted_line,
    )
}
