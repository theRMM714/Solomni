//! **跨会话回档编排**：主会话回到某一行（含该行），各 agent 子会话按同一个**回合 id** 同步截断，
//! 然后把新的行与元信息写回；协作/历史会话按转录重建（状态全部派生）。
//!
//! 纯行与事件算术在 session 的 domain/rewind.rs；回档的**副作用告知**（工具执行次数）也在这里。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate) 供兄弟族（会话生命周期、改需求）调用。

use super::*;
use crate::capabilities::session::api::RewindMode;

impl Conductor {
    /// 回档：**留档**（标记 + 折叠，可恢复）、**删除**（真的截断）或**恢复**（在该标记处截断）。
    /// 返回重放后的活动视图，供前端整体重建；三种模式都同步整棵子树。
    pub(crate) fn rewind(
        &mut self,
        sid: &str,
        target: RewindTarget,
    ) -> Result<Vec<serde_json::Value>, String> {
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        let (meta, raw) = self.history.load(sid)?;
        // 这次回档移出活动视图的工具执行次数：用于如实告知"副作用不在回档范围内"。
        let before_tools = crate::capabilities::collab::api::tool_runs(
            &crate::capabilities::session::api::truncate_events(&raw),
        );
        let work = self.work_root(sid)?;
        let mut active = match target {
            RewindTarget::Archive(line) => {
                let before = crate::capabilities::session::api::align_keep(&raw, line);
                let from = crate::capabilities::session::api::next_line_id(&raw);
                let mark = Self::next_mark_id(&raw);
                let turn = crate::capabilities::session::api::turn_of_line(&raw, before);
                // 先记下留档前的共享区头，再算各会话保留行（子会话的收窗会落盘）。
                let after = self.workspace.work_head(&work)?;
                let kids = self.rewind_children(sid, turn, mark, RewindMode::Archive, after)?;
                let keep = Self::keep_by_agent(&meta, before, &kids);
                let commit = self.workspace.work_rewind_to(&work, &keep)?;
                self.history
                    .append(
                        sid,
                        &[serde_json::json!({
                            "type": "rewind",
                            "mark": mark,
                            "mode": "archive",
                            "before": before,
                            "from": from,
                            "commit": commit,
                            "after": after
                        })],
                    )
                    .map_err(|e| format!("留档落盘失败：{}", e))?;
                let (_, raw_after) = self.history.load(sid)?;
                let next = crate::capabilities::session::api::next_line_id(&raw_after);
                self.restore_session(sid, &meta, &raw_after, next)?;
                crate::capabilities::session::api::truncate_events(&raw_after)
            }
            RewindTarget::Delete(line) => {
                let before = crate::capabilities::session::api::align_keep(&raw, line);
                let turn = crate::capabilities::session::api::turn_of_line(&raw, before);
                let kids = self.rewind_children(sid, turn, 0, RewindMode::Delete, None)?;
                let keep = Self::keep_by_agent(&meta, before, &kids);
                // 共享区：物化到删除点，并**真的丢弃**该点之后的提交（删除就是删除）。
                let point = self.workspace.work_rewind_to(&work, &keep)?;
                self.workspace.work_discard_after(&work, point)?;
                let kept = crate::capabilities::session::api::cut_before_line(&raw, before);
                self.history
                    .replace(sid, &kept)
                    .map_err(|e| format!("删除落盘失败：{}", e))?;
                // 删除的算子是"截掉尾部"：单 agent 且压缩点没被切到时**就地精确回退**（不重建，
                // 保住通道与运行态）；否则按转录重建（协作与跨压缩点都得重建）。
                let precise = match self.sessions.get(sid) {
                    Some(Session::Single(s)) => s.compacted_upto == 0 || before >= s.compacted_upto,
                    _ => false,
                };
                if precise {
                    if let Some(Session::Single(s)) = self.sessions.get_mut(sid) {
                        s.apply_rewind(before);
                    }
                } else {
                    let next = crate::capabilities::session::api::next_line_id(&kept);
                    self.restore_session(sid, &meta, &kept, next)?;
                }
                crate::capabilities::session::api::truncate_events(&kept)
            }
            RewindTarget::Restore(mark) => {
                let idx = raw
                    .iter()
                    .position(|ev| {
                        ev.get("type").and_then(|t| t.as_str()) == Some("rewind")
                            && ev.get("mark").and_then(|m| m.as_u64()) == Some(mark)
                    })
                    .ok_or_else(|| format!("没有回档标记 {}（可能已被恢复）", mark))?;
                let recorded = crate::capabilities::session::api::RewindMark::from_value(&raw[idx]);
                let after = recorded.as_ref().and_then(|m| m.after);
                let kept: Vec<serde_json::Value> = raw[..idx].to_vec();
                self.history
                    .replace(sid, &kept)
                    .map_err(|e| format!("恢复落盘失败：{}", e))?;
                self.rewind_children(sid, 0, mark, RewindMode::Restore, None)?;
                // 共享区回到留档前那一刻，并丢弃留档之后的所有提交。
                self.workspace.work_restore_point(&work, after)?;
                self.workspace.work_discard_after(&work, after)?;
                let next = crate::capabilities::session::api::next_line_id(&kept);
                self.restore_session(sid, &meta, &kept, next)?;
                crate::capabilities::session::api::truncate_events(&kept)
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
        Ok(active)
    }

    /// 重建并放回会话（回档后状态全部派生）；next = 新行续号（全量最大 id + 1，永不回退）。
    fn restore_session(
        &mut self,
        sid: &str,
        meta: &SessionMeta,
        raw: &[serde_json::Value],
        next: u64,
    ) -> Result<(), String> {
        let active = crate::capabilities::session::api::truncate_events(raw);
        let mut rebuilt = self.rebuild_session(meta, &active)?;
        match &mut rebuilt {
            Session::Single(s) => s.next_line = next,
            Session::Collab(c) => c.next_line = next,
        }
        self.sessions.insert(sid.to_string(), rebuilt);
        Ok(())
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

    /// 把整棵子树的会话同步到同一个回档点，并返回「agent 名 → 该会话的保留行」（供共享区对齐）。
    /// 留档：每个子会话按自己的回合边界收窗并写同一个 mark id（恢复按 id 找它）；
    /// 删除：每个子会话在自己的回合边界处真的截断；恢复：删该 mark 记录及其后。
    pub(crate) fn rewind_children(
        &mut self,
        sid: &str,
        keep_turn: u64,
        mark: u64,
        mode: RewindMode,
        after: Option<u64>,
    ) -> Result<std::collections::BTreeMap<String, u64>, String> {
        let kids: Vec<String> = self.subtree_of(sid).into_iter().skip(1).collect();
        let mut keeps = std::collections::BTreeMap::new();
        for kid in kids {
            let (meta, raw) = self.history.load(&kid)?;
            let agent = meta
                .agents
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_default();
            if self.running.contains(&kid) {
                // 正在跑的会话跳过（它自己的收尾会落盘）；共享区要保住它的提交。
                if !agent.is_empty() {
                    keeps.insert(agent, u64::MAX);
                }
                continue;
            }
            match mode {
                RewindMode::Archive => {
                    let active = crate::capabilities::session::api::truncate_events(&raw);
                    let before =
                        crate::capabilities::session::api::last_line_within(&active, keep_turn);
                    let from = crate::capabilities::session::api::next_line_id(&raw);
                    self.history
                        .append(
                            &kid,
                            &[serde_json::json!({
                                "type": "rewind",
                                "mark": mark,
                                "mode": "archive",
                                "before": before,
                                "from": from,
                                "after": after
                            })],
                        )
                        .map_err(|e| format!("子会话 {} 留档落盘失败：{}", kid, e))?;
                    let (_, raw_after) = self.history.load(&kid)?;
                    let next = crate::capabilities::session::api::next_line_id(&raw_after);
                    self.restore_session(&kid, &meta, &raw_after, next)?;
                    if !agent.is_empty() {
                        keeps.insert(agent, before);
                    }
                }
                RewindMode::Delete => {
                    let active = crate::capabilities::session::api::truncate_events(&raw);
                    let before =
                        crate::capabilities::session::api::last_line_within(&active, keep_turn);
                    let kept = crate::capabilities::session::api::cut_before_line(&raw, before);
                    self.history
                        .replace(&kid, &kept)
                        .map_err(|e| format!("子会话 {} 删除落盘失败：{}", kid, e))?;
                    let next = crate::capabilities::session::api::next_line_id(&kept);
                    self.restore_session(&kid, &meta, &kept, next)?;
                    if !agent.is_empty() {
                        keeps.insert(agent, before);
                    }
                }
                RewindMode::Restore => {
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
                    self.restore_session(&kid, &meta, &kept, next)?;
                }
            }
        }
        Ok(keeps)
    }

    /// 按会话元信息 + 转录事件重建会话对象（通道是机制，按记录的选择重新装配）。
    pub(crate) fn rebuild_session(
        &self,
        meta: &SessionMeta,
        events: &[serde_json::Value],
    ) -> Result<Session, String> {
        let roster = self.workspace.roster();
        let sandboxes = self.sandboxes(meta, &roster)?;
        // 整队的权威是**转录**：重建之前先把队清空并按已发出的卡号续号（内存里那一份不作数）。
        self.desk_of(&meta.name)
            .rebuild(crate::capabilities::session::api::issued_in(events));
        match meta.mode.as_str() {
            "collab" => Ok(Session::Collab(CollabSession::restore(
                Arc::clone(&self.llm),
                Arc::clone(&self.workspace),
                self.registry.snapshot(),
                Arc::clone(&self.prompt),
                Arc::clone(&self.systools),
                Arc::clone(&self.tools),
                Arc::clone(&self.log),
                meta,
                events,
                sandboxes,
                // 重建出来的整队进**同一条队**（按转录里已发出的卡号续号，已答过的不重问）。
                self.desk_of(&meta.name),
            )?)),
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
                let channel = self.registry.channel(a.model.as_deref());
                // 重建时同样按登记处派生形态：系统提示与实际协议必须一致（回放才与实时一致）
                let mode = if channel.is_some() {
                    self.registry.tool_mode(a.model.as_deref())
                } else {
                    crate::capabilities::llm::api::ToolMode::Envelope
                };
                // **会话参数**：与建立时同一个口径（身份块每回合现渲染，不进消息列表）。
                let params = crate::capabilities::session::api::SessionParams::from_workspace(
                    &a.name,
                    &sb,
                    &modules,
                    &self.session_kind_of(meta)?,
                    &self.role_of(meta)?,
                );
                let (chat, note) = self.llm.member_channel(channel.as_ref(), &a.name);
                let texts = self.prompt.tools();
                let (history, marks, line_reply, compacted_upto) =
                    replay_dialogue(events, mode, &texts);
                let unavailable = self.unavailable_modules(&meta.exec, &modules);
                // 身份与角色**同一把尺子**（重建与实时不能各写一份，见 Conductor::role_of）：
                // 协作派生的成员 / 节点 = executor（拿得到回报工具）；其余单 agent 工作 = solo。
                let role = self.role_of(meta)?;
                let mut tools =
                    self.tools_env(&modules, &sb, unavailable, meta.exec.net, mode, &role);
                // 回复 id 跨重启单调：从转录里的最大值续号，否则新回复会与旧回复并成一组。
                tools.reply_seq = crate::capabilities::session::api::max_reply(events);
                Ok(Session::Single(
                    crate::capabilities::session::api::AgentSession::restore(
                        &a.name,
                        params,
                        history,
                        marks,
                        line_reply,
                        compacted_upto,
                        chat,
                        note,
                        Some(tools),
                        self.prompt.refs(),
                        self.prompt.tools(),
                    ),
                ))
            }
            // 代理会话：没有 agent（核心自己说话）；外壳由 build_proxy 装配，回放与单 agent 共用同一件。
            "proxy" => {
                let mut s = self.build_proxy(meta)?;
                let texts = self.prompt.tools();
                let (history, marks, line_reply, compacted_upto) =
                    replay_dialogue(events, s.tool_mode(), &texts);
                s.dialogue = history;
                s.marks = marks;
                s.line_reply = line_reply;
                s.compacted_upto = compacted_upto;
                s.next_line = s.marks.len() as u64;
                if let Some(t) = s.tools.as_mut() {
                    t.reply_seq = crate::capabilities::session::api::max_reply(events);
                }
                Ok(Session::Single(s))
            }
            other => Err(format!(
                "未知会话形态：{}（只认 single / collab / proxy）",
                other
            )),
        }
    }
}
/// 把转录事件还原成（对话, 每行历史长度, 每行回复号, 压缩点）——单 agent 与代理会话**共用同一件**。
/// 两处不各拼一遍：重建必须与实时逐条一致（见 `reply_msgs` 与 session-model.md）。
fn replay_dialogue(
    events: &[serde_json::Value],
    mode: crate::capabilities::llm::api::ToolMode,
    texts: &crate::capabilities::prompt::api::ToolTexts,
) -> (
    Vec<crate::capabilities::llm::api::Msg>,
    Vec<usize>,
    Vec<u64>,
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
    // **压缩过**的会话：按 `compacted` 事件重建发送视图（见 session-model.md 六）——
    // `up_to` 之前的行不再进对话，由一份摘要代替（转录本身完整保留，用户照样能查）。
    // 回档到压缩点之前时这条事件已随转录被截掉，所以「没有它」就是「回到压缩前」。
    let compacted = crate::capabilities::session::api::last_compaction(events);
    let compacted_upto = compacted.as_ref().map(|(up_to, _)| *up_to).unwrap_or(0);
    // 对话里**只有**真正发生过的事；身份与环境由 params 现渲染。
    let mut history: Vec<Msg> = compacted
        .as_ref()
        .map(|(_, summary)| crate::capabilities::session::api::summary_message(summary))
        .into_iter()
        .collect();
    let mut marks: Vec<usize> = Vec::new();
    let mut line_reply: Vec<u64> = Vec::new();
    let reply_of = |v: &serde_json::Value| v.get("reply").and_then(|x| x.as_u64()).unwrap_or(0);
    let mut i = 0usize;
    while i < rows.len() {
        let l = rows[i];
        // 被总结掉的行：不进对话，但仍占一行（marks / line_reply 与转录行一一对应）。
        if compacted_upto > 0 && l.get("id").and_then(|x| x.as_u64()).unwrap_or(0) < compacted_upto
        {
            if l.get("tool").is_some() {
                // 同一次回复的 tool 行连续同号：整组一起跳，别从中间切开。
                let reply = reply_of(l.get("tool").expect("已判存在"));
                while i < rows.len()
                    && reply_of(rows[i].get("tool").unwrap_or(&serde_json::Value::Null)) == reply
                {
                    line_reply.push(reply);
                    marks.push(history.len().max(1));
                    i += 1;
                }
            } else {
                line_reply.push(reply_of(l));
                marks.push(history.len().max(1));
                i += 1;
            }
            continue;
        }
        let line = l.get("line").and_then(|x| x.as_str()).unwrap_or("");
        // **读结构化字段**（种类 / 系统标记 / 正文），不从正文里抠 [标签]。
        let kind = l.get("kind").and_then(|x| x.as_str()).unwrap_or("");
        // 系统注入的行按它该有的角色还原：提醒/边界是 system，
        // **派发行**（`task`）是 user——否则重建出来的请求又变成一条 user 都没有，供应商照样拒收。
        if kind == "system" || l.get("system").and_then(|x| x.as_bool()).unwrap_or(false) {
            let is_task = l.get("task").and_then(|x| x.as_bool()).unwrap_or(false);
            history.push(if is_task {
                Msg::user(line.to_string())
            } else {
                Msg::system(line.to_string())
            });
            line_reply.push(l.get("id").and_then(|x| x.as_u64()).unwrap_or(0));
            marks.push(history.len());
            i += 1;
        } else if kind == "user" {
            // 用户说的行：正文就是用户那句话（[用户] / [用户:需求] 这类标签在字段里）。
            history.push(Msg::user(line.to_string()));
            // 用户行不属于任何回复：给它自己的行号，回档时才不会与相邻行误并成一组。
            line_reply.push(l.get("id").and_then(|x| x.as_u64()).unwrap_or(0));
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
            // 这一回复的助手消息正文（空正文的回复不带 raw；组内取一份即可）。
            let raw = group
                .iter()
                .find_map(|t| {
                    t.get("tool")
                        .and_then(|x| x.get("raw"))
                        .and_then(|x| x.as_str())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_default();
            let views: Vec<crate::capabilities::session::api::ToolCallView> = group
                .iter()
                .filter_map(|t| t.get("tool").cloned())
                .filter_map(|t| serde_json::from_value(t).ok())
                .collect();
            for m in crate::capabilities::session::api::reply_msgs(mode, raw, &views, texts) {
                history.push(m);
            }
            for _ in 0..group.len() {
                line_reply.push(reply);
                marks.push(history.len());
            }
        } else {
            // 文本行：它紧跟 tool 行时属于同一次回复（历史由那组 tool 行统一推进，这里不推）；
            // 否则这一行自己就是一条回复，推 assistant(该行文本)。
            let next_is_tool = rows
                .get(i + 1)
                .map(|n| n.get("tool").is_some())
                .unwrap_or(false);
            if !next_is_tool {
                // 正文本身就是内容（说话人/动词在字段里），直接进助手消息。
                history.push(Msg::assistant(line.to_string()));
            }
            line_reply.push(reply_of(l));
            marks.push(history.len());
            i += 1;
        }
    }
    (history, marks, line_reply, compacted_upto)
}
