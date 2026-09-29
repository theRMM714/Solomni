//! **跨会话回档编排**：主会话回到某一行（含该行），各 agent 子会话按同一个**回合 id** 同步截断，
//! 然后把新的行与元信息写回；协作/历史会话按转录重建（状态全部派生）。
//!
//! 纯行与事件算术在 session 的 domain/rewind.rs；回档的**副作用告知**（工具执行次数）也在这里。
//!
//! 它是 Conductor 的一个方法族：与 mod.rs 同在 service 模块下（子模块看得见父模块的私有字段），
//! 方法取 pub(crate) 供兄弟族（会话生命周期、改需求）调用。

use super::*;
impl Conductor {
    /// 回档：保留到转录行 id 为止（含该行），其后记录一并删除；流水只追加 rewind 记录。
    /// 返回重放后的完整事件流，供前端整体重建（不用前端自己推算截断）。
    /// 单 agent 的活动会话按历史精确回退；协作与历史会话按转录重建（状态全部派生）。
    pub(crate) fn rewind(
        &mut self,
        sid: &str,
        keep_id: u64,
    ) -> Result<Vec<serde_json::Value>, String> {
        if self.running.contains(sid) {
            return Err(Self::running_refusal(sid));
        }
        // 单 agent 活动会话可按历史精确回退；但**压缩点之前**的对话已被摘要取代、内存里补不回来，
        // 那时只能按转录重建（重建出来的是压缩前的内容，正是回档该有的语义）。
        let precise = match self.sessions.get(sid) {
            Some(Session::Single(s)) => s.compacted_upto() == 0 || keep_id >= s.compacted_upto(),
            _ => false,
        };
        if !precise {
            self.ensure_session(sid)?;
        }
        let (meta, raw_before) = self.history.load(sid)?;
        let before = crate::capabilities::session::api::truncate_events(&raw_before);
        self.history
            .append(
                sid,
                &[serde_json::json!({ "type": "rewind", "keep": keep_id })],
            )
            .map_err(|e| format!("回档落盘失败：{}", e))?;
        let (_, raw_after) = self.history.load(sid)?;
        let after = crate::capabilities::session::api::truncate_events(&raw_after);
        if precise {
            if let Some(Session::Single(s)) = self.sessions.get_mut(sid) {
                s.rewind(keep_id);
            }
        } else {
            let rebuilt = self.rebuild_session(&meta, &after)?;
            self.sessions.insert(sid.to_string(), rebuilt);
        }
        // **回档同步**：主会话回到第 keep_id 行，各 agent 会话按同一个**回合 id** 同步截断
        // （见 docs/session/session-model.md 五）——子会话不在主会话的流水里，得各自回档。
        if !precise {
            let keep_turn = crate::capabilities::session::api::turn_of_line(&before, keep_id);
            self.rewind_children(sid, keep_turn)?;
        }
        let dropped = crate::capabilities::collab::api::tool_runs(&before)
            .saturating_sub(crate::capabilities::collab::api::tool_runs(&after));
        let mut out = after;
        if dropped > 0 {
            out.push(serde_json::json!({
                "type": "notice",
                "text": format!("[提示] 回档删掉了其后 {} 次工具执行——那些副作用不会回滚，继续可能会重新执行。", dropped),
            }));
        }
        Ok(out)
    }

    /// 主会话回档后，把各 agent 会话**同步**截断到"最后一个回合 ≤ T 的行"。
    /// 正在跑的会话跳过（它自己的收尾会落盘，硬截会留下半截）。
    pub(crate) fn rewind_children(&mut self, sid: &str, keep_turn: u64) -> Result<(), String> {
        let kids: Vec<String> = self
            .history_list()
            .into_iter()
            .filter(|h| h.parent.as_deref() == Some(sid))
            .map(|h| h.name)
            .collect();
        for kid in kids {
            if self.running.contains(&kid) {
                continue;
            }
            let (_, events) = self.history.load(&kid)?;
            let keep = crate::capabilities::session::api::last_line_within(
                &crate::capabilities::session::api::truncate_events(&events),
                keep_turn,
            );
            self.history
                .append(
                    &kid,
                    &[serde_json::json!({ "type": "rewind", "keep": keep })],
                )
                .map_err(|e| format!("子会话 {} 回档落盘失败：{}", kid, e))?;
            if let Some(Session::Single(s)) = self.sessions.get_mut(&kid) {
                s.rewind(keep);
            }
        }
        Ok(())
    }

    /// 按会话元信息 + 转录事件重建会话对象（通道是机制，按记录的选择重新装配）。
    pub(crate) fn rebuild_session(
        &self,
        meta: &SessionMeta,
        events: &[serde_json::Value],
    ) -> Result<Session, String> {
        let roster = self.workspace.roster();
        let sandboxes = self.sandboxes(meta, &roster)?;
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
                    &a.name, &sb, &modules,
                );
                let (chat, note) = self.llm.member_channel(channel.as_ref(), &a.name);
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
                let texts = self.prompt.tools();
                let reply_of =
                    |v: &serde_json::Value| v.get("reply").and_then(|x| x.as_u64()).unwrap_or(0);
                let mut i = 0usize;
                while i < rows.len() {
                    let l = rows[i];
                    // 被总结掉的行：不进对话，但仍占一行（marks / line_reply 与转录行一一对应）。
                    if compacted_upto > 0
                        && l.get("id").and_then(|x| x.as_u64()).unwrap_or(0) < compacted_upto
                    {
                        if l.get("tool").is_some() {
                            // 同一次回复的 tool 行连续同号：整组一起跳，别从中间切开。
                            let reply = reply_of(l.get("tool").expect("已判存在"));
                            while i < rows.len()
                                && reply_of(rows[i].get("tool").unwrap_or(&serde_json::Value::Null))
                                    == reply
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
                    if kind == "system"
                        || l.get("system").and_then(|x| x.as_bool()).unwrap_or(false)
                    {
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
                            && reply_of(rows[i].get("tool").unwrap_or(&serde_json::Value::Null))
                                == reply
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
                        for m in
                            crate::capabilities::session::api::reply_msgs(mode, raw, &views, &texts)
                        {
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
                let unavailable = self.unavailable_modules(&meta.exec, &modules);
                let mut tools = self.tools_env(&modules, &sb, unavailable, meta.exec.net, mode);
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
            other => Err(format!("未知会话形态：{}（只认 single / collab）", other)),
        }
    }
}
