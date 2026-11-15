//! 目的：从落盘事实**重建**会话对象——打开、启用与回档都走这一条（重建必须与实时逐条一致）。
//! 管：按 meta 刷新工作区布局投影、裁决队按转录续号、按形态（collab / single / proxy）装回会话对象、
//!   把读台账 / 回复号 / 用量 / shell 快照按转录恢复。
//! 不管：回档计划与转录裁剪（仍在 `conductor` 的 `rewind`）；落盘。
//! 联动：`protocol.rs` 的 `ensure_session` 调这里；`assemble.rs` / `proxy.rs` 提供装配构件。

use std::sync::Arc;

use crate::capabilities::collab::api::CollabSession;
use crate::capabilities::modules::api::Module;
use crate::capabilities::prompt::api::Segment;
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{SessionMeta, SessionParams};
use crate::capabilities::turn::api::{MemberSpec, Session, member_tools};
use crate::kernel::ports::HostProbe;

use crate::capabilities::turn::service::session::Turn;
use crate::kernel::api::now_ts;

impl Turn {
    /// 目的：按会话元信息 + 转录事件重建会话对象（通道是机制，按记录的选择重新装配）。
    /// 参数：`meta` = 会话元信息；`events` = 该会话的转录事件；`registry` = 登记处；
    ///   `probe` = 宿主能力探测。
    pub(crate) fn rebuild_session(
        &self,
        meta: &SessionMeta,
        events: &[serde_json::Value],
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<Session, String> {
        let roster = self.modules.roster();
        // 打开 / 重建时按 meta 刷新布局投影（会话目录可能被复制或迁移；meta 是权威）。
        let work = self.work_root_of(meta)?;
        let binding = self.workspace_binding_of(meta, probe)?;
        self.workspace.bind(&work, binding.as_deref())?;
        if binding.is_some() {
            // 旧会话首次启用：补一次初始导入（有 head 时无操作）。
            if let Err(e) = self.workspace.work_import_root(&work, now_ts()) {
                self.log.warn(
                    "turn::rebuild_session",
                    &format!("工作区初始导入失败：{}", e),
                );
            }
        }
        let sandboxes = self.sandboxes(meta, &roster, registry, probe)?;
        // 整队的权威是**转录**：重建之前先把队清空并按已发出的卡号续号（内存里那一份不作数）。
        self.desk_of(&meta.name)
            .rebuild(crate::capabilities::session::api::issued_in(events));
        let built = match meta.mode.as_str() {
            "collab" => Session::Collab(CollabSession::restore(
                Arc::clone(&self.llm),
                Arc::clone(&self.vm),
                Arc::clone(&self.modules),
                registry.snapshot(),
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
                let channel = registry
                    .channel(a.model.as_deref())
                    .map_err(|e| format!("agent {} 没有可用模型通道：{}", a.name, e))?;
                // 重建时同样按登记处派生形态：系统提示与实际协议必须一致（回放才与实时一致）
                let mode = registry.tool_mode(a.model.as_deref());
                // **会话参数**：与建立时同一个口径（身份块每回合现渲染，不进消息列表）。
                let params = SessionParams::from_workspace(
                    &a.name,
                    &sb,
                    &modules,
                    &self.session_kind_of(meta)?,
                    &self.role_of(meta)?,
                )
                .with_chara(a.chara.clone())
                .with_guest(self.guest_os_of(&meta.exec, probe));
                let chat = self.llm.member_channel(&channel, &a.name);
                let (history, marks, line_reply, line_ids, compacted_upto, compacted_line) =
                    replay_dialogue(events, mode);
                let unavailable = self.unavailable_modules(&meta.exec, &modules, probe);
                // 身份与角色**同一把尺子**（重建与实时不能各写一份，见 Turn::role_of）：
                // 协作派生的成员 / 节点 = executor（拿得到回报工具）；其余单 agent 工作 = solo。
                let role = self.role_of(meta)?;
                let host_plan = if meta.exec.tier == crate::kernel::api::Tier::Host {
                    self.host_plan_for(
                        &meta.exec,
                        &modules,
                        crate::capabilities::packages::api::PinPolicy::Require,
                        probe,
                    )
                } else {
                    crate::capabilities::packages::api::PackagePlan::default()
                };
                let mut tools = member_tools(
                    &self.session_deps(),
                    &MemberSpec {
                        modules: &modules,
                        sandbox: &sb,
                        unavailable,
                        net: self.network_allowed(registry),
                        mode,
                        role: &role,
                        tier: meta.exec.tier,
                        guest_lease: self.guest_lease_for(meta)?,
                        plan: &host_plan,
                        base_roots: &self.base_roots,
                        host_read_roots: self.host_read_roots(registry),
                        shell: self.shell_slot(&sb.session),
                    },
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
                    self.prompt.text(Segment::CompactPrompt).to_string(),
                );
                // 重建也要恢复上下文预算与最近一次用量：否则重启后自动压缩与状态栏都失忆。
                let (window, percent, keep_percent) =
                    self.context_budget(a.model.as_deref(), registry);
                s.set_context_budget(window, percent, keep_percent);
                s.set_model(a.model.clone().or_else(|| registry.core_model()));
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
                let mut s = self.build_proxy(meta, registry)?;
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
                ));
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
pub fn replay_dialogue(
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
    // **压缩过**的会话按最后一条压缩行重建发送视图（见 session-model.md 六）：`up_to` 之前的行由摘要代替，
    // 转录本身完整保留；回档到压缩行之前时它已随转录被截掉，所以「没有它」就是「回到压缩前」。
    let compacted = crate::capabilities::session::api::last_compaction(events);
    let compacted_upto = compacted.as_ref().map(|(up_to, _, _)| *up_to).unwrap_or(0);
    let compacted_line = compacted.as_ref().map(|(_, _, id)| *id).unwrap_or(0);
    // 对话里**只有**真正发生过的事；身份与环境由 params 现渲染。
    let mut history: Vec<crate::capabilities::llm::api::Msg> = compacted
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
                crate::capabilities::llm::api::Msg::user(line.to_string())
            } else {
                crate::capabilities::llm::api::Msg::system(line.to_string())
            });
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        } else if kind == "user" {
            // 用户说的行：正文就是用户那句话（[用户] / [用户:需求] 这类标签在字段里）。
            history.push(crate::capabilities::llm::api::Msg::user(line.to_string()));
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
                history.push(crate::capabilities::llm::api::Msg::assistant(
                    line.to_string(),
                ));
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
