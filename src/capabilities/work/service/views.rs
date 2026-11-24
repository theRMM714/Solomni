//! 目的：会话中心与工作文件的视图——在世会话 × 历史的并集、继续判据、文件视图 / 上传 / 日志 / 提交与恢复。
//! 管：会话视图与存在性、继续判据、文件视图、上传、日志、提交视图与回滚恢复。
//! 不管：会话怎么建（`create.rs`）、配置怎么改（`config.rs`）、提交本身（workspace）。
//! 联动：api 层的会话与文件接口直接转发到这里。
//!   本方法族只碰 workspace / sessions / history / running 这四个字段。

use super::*;
use crate::capabilities::conversation::session::api::{subtree_of, work_root};

impl Work {
    /// 目的：会话类型表翻成界面要的样子（名字、一句说明、要不要名单与需求）。
    /// 约束：顺序照声明；要不要需求按声明里的流程（有流程才必填本次需求）；代理式（没有流程、坐核心代理席）不要名单。
    pub(crate) fn session_types(&self) -> Vec<SessionTypeView> {
        self.flows
            .book()
            .profiles
            .iter()
            .map(|p| {
                let auto = p.auto;
                SessionTypeView {
                    kind: p.id.clone(),
                    name: if p.name.trim().is_empty() {
                        p.id.clone()
                    } else {
                        p.name.clone()
                    },
                    hint: p.hint.clone(),
                    flows: !p.flows.is_empty(),
                    roster: !auto,
                    needs_task: !p.flows.is_empty(),
                    min_agents: self.flows.book().min_agents(&p.flows),
                }
            })
            .collect()
    }

    /// 目的：临时拼装的白名单：只认已注册的流程 id，重复给同一个 id 如实拒绝。
    /// 约束：名字来自机制的流程登记（`session::api` 的 `flow_ids`），不另立一份清单。
    pub(crate) fn flow_ids(&self, ids: &[String]) -> Result<Vec<String>, String> {
        let mut out: Vec<String> = Vec::new();
        for id in ids {
            if out.contains(id) {
                return Err(format!("临时拼装里同一个流程出现了两次：{}", id));
            }
            if crate::capabilities::conversation::session::api::mechanism_of(id).is_none() {
                return Err(format!(
                    "会话类型表里没有这个流程：{}（已注册的流程：{}）",
                    id,
                    crate::capabilities::conversation::session::api::flow_ids().join(" / ")
                ));
            }
            out.push(id.clone());
        }
        Ok(out)
    }

    /// 目的：上下文「继续」是否该出现——判据只在后端（前端不按末条/类型猜）。
    /// 约束：正在跑、已归档、已停止、代理、或没有真实断点时不出现；有流程的会话有挂起卡时由卡承担。
    ///   已停止 / 已归档都是历史态，恢复入口是「启用会话」/「恢复归档」；这时的「继续」会被派发闸拒掉。
    fn continue_action(
        &self,
        turn: &Turn,
        sid: &str,
        running: bool,
        entry: Option<&HistoryView>,
        has_pending: bool,
    ) -> Option<ContinueView> {
        let flows = entry.is_some_and(|h| h.flows);
        let auto = entry.is_some_and(|h| h.auto);
        let run = entry.map(|h| h.run.as_str()).unwrap_or("active");
        if running || run == "archived" || run == RunState::Stopped.as_str() {
            return None;
        }
        if flows {
            // 有流程的会话没有「断点重答」这回事：泵没在跑就从断点推进（挂起卡优先由卡承担）。
            return (!has_pending).then(|| ContinueView {
                kind: "resume".to_string(),
                label: "继续".to_string(),
                reason: "流程没有在跑".to_string(),
            });
        }
        if auto {
            return None;
        }
        // 只有「上一轮还没答完」才算断点：末条是 AI 发言时继续会撞上「请先发言再继续」。
        let user_last = matches!(turn.session(sid), Some(Session::Single(s)) if s.last_is_user());
        user_last.then(|| ContinueView {
            kind: "turn".to_string(),
            label: "继续".to_string(),
            reason: "上一轮被打断：继续会把这轮重答一遍".to_string(),
        })
    }

    /// 目的：一条会话的权威视图（启用回包 / 正在生成时附着用）：状态、运行态与上下文继续。
    pub(crate) fn session_view(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        sid: &str,
    ) -> Result<SessionView, String> {
        self.session_views(turn, registry, probe, &self.history_list())
            .into_iter()
            .find(|v| v.sid == sid)
            .ok_or_else(|| format!("无此会话：{}", sid))
    }

    /// 目的：会话列表视图（进行中的工作）。形态取落盘 meta（单一真相，不在内存里留影子状态）。
    pub fn session_views(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        history: &[HistoryView],
    ) -> Vec<SessionView> {
        // 在表里的会话 + **正在生成的会话**（后者对象在工作线程上，但它确实存在、也确实在跑）。
        // 漏掉它们会让界面以为会话不见了。
        let running: Vec<(String, bool)> = turn
            .running_iter()
            .map(|sid| (sid.clone(), false))
            .collect();
        let running_now: std::collections::BTreeSet<String> =
            turn.running_iter().cloned().collect();
        let listed: Vec<(String, bool)> = turn
            .sessions_iter()
            .map(|(sid, s)| {
                let done = s.is_done();
                (sid.clone(), done)
            })
            .chain(running)
            .collect();
        // 思考视图按**工作根**算一次、整棵子树复用（同一条工作里所有会话共享一个档位）。
        let mut thinking_cache: BTreeMap<String, ThinkingView> = BTreeMap::new();
        listed
            .into_iter()
            .map(|(sid, done)| {
                let entry = history.iter().find(|h| h.name == sid);
                let kind = entry.map(|h| h.kind.clone()).unwrap_or_default();
                let flows = entry.is_some_and(|h| h.flows);
                let auto = entry.is_some_and(|h| h.auto);
                // 运行态取落盘事实（不在内存里留影子状态）；拿不到（生成中 / 未落盘）按正常运行。
                let run = entry
                    .map(|h| h.run.as_str())
                    .unwrap_or("active")
                    .to_string();
                // 记的档位来自落盘 meta（权威）：环境变了也要如实提示——**不拦打开**（记录是用户的）。
                let exec = entry.map(|h| h.exec.clone()).unwrap_or_default();
                let qemu = self.qemu_path(registry);
                let share_tool = self.share_tool_path(registry);
                let readiness = crate::capabilities::vm::api::tier_readiness(
                    &self.root,
                    &exec,
                    qemu.as_deref(),
                    share_tool.as_deref(),
                    probe,
                );
                // 「改需求」能力位：有本次需求行才给。在表里看会话种类；不在表里（生成中/未打开）
                // 看落盘 meta 的形态（那是名单与形态的单一真相）。
                let can_update_task = match turn.session(&sid) {
                    Some(Session::Flow(_)) => true,
                    Some(Session::Single(_)) => false,
                    None => flows,
                };
                // 待裁决：**判据只有队本身**（核心各关卡与工具级确认排在同一条队上）——
                // 正在生成的会话（对象在工作线程手里）照样取得到；推的卡片事件与它是同一份。
                let pending = turn.desk().peek_gate(&sid).and_then(|d| d.json());
                let running = running_now.contains(&sid);
                let continue_action =
                    self.continue_action(turn, &sid, running, entry, pending.is_some());
                // 只有持续对话的会话才有单一上下文；协作主会话由核心按次组上下文，不伪造数字。
                let context = match turn.session(&sid) {
                    Some(Session::Single(s)) => s.context_usage(),
                    _ => None,
                };
                let (permission_profile, permission_label, permission_narrowed, permission_parent) =
                    match self.history.meta(&sid) {
                        Ok(m) => {
                            let parent = m.parent.as_ref().and_then(|p| {
                                self.history.meta(p).ok().and_then(|pm| {
                                    turn.permissions_for(registry, &pm)
                                        .ok()
                                        .map(|ep| ep.profile.as_str().to_string())
                                })
                            });
                            match turn.permissions_for(registry, &m) {
                                Ok(p) => (
                                    p.profile.as_str().to_string(),
                                    p.profile.label().to_string(),
                                    m.parent.is_some() && m.sandbox.narrowing.is_some(),
                                    parent,
                                ),
                                Err(_) => (String::new(), "权限不可用".to_string(), false, None),
                            }
                        }
                        Err(_) => (String::new(), String::new(), false, None),
                    };
                let shell = turn.shell_view(&sid);
                // 过期内容替换是**工作根**的会话级事实：子会话与工作根共享同一个开关与保底。
                let (context_freshness, freshness_keep_percent) =
                    match work_root(self.history.as_ref(), &sid) {
                        Ok(root) => self
                            .history
                            .meta(&root)
                            .map(|m| (m.context_freshness, m.freshness_keep_percent))
                            .unwrap_or((false, 80)),
                        Err(_) => (false, 80),
                    };
                // 最近一次发送视图的替换统计只活在活会话里：重建后为 None，面板就不显示这一行。
                let freshness_stats = match turn.session(&sid) {
                    Some(Session::Single(s)) => s.freshness_stats(),
                    _ => None,
                };
                // 累计用量是**这条会话自己**的账面（子会话各记各的，不汇总到工作根）：与"当前上下文用量"分开。
                let usage_totals = self
                    .history
                    .meta(&sid)
                    .map(|m| m.usage_totals)
                    .unwrap_or_default();
                let thinking = match work_root(self.history.as_ref(), &sid) {
                    Ok(root) => thinking_cache
                        .entry(root.clone())
                        .or_insert_with(|| self.thinking_view_for_root(registry, &root))
                        .clone(),
                    Err(_) => ThinkingView {
                        level: None,
                        default_level: None,
                        label: "未设置".to_string(),
                        options: Vec::new(),
                        note: String::new(),
                    },
                };
                SessionView {
                    running,
                    sid,
                    kind,
                    flows,
                    auto,
                    done,
                    tier: exec.tier.as_str().to_string(),
                    tier_ready: readiness.ready(),
                    tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
                    permission_profile,
                    permission_label,
                    thinking,
                    context_freshness,
                    freshness_keep_percent,
                    freshness_stats,
                    permission_narrowed,
                    permission_parent,
                    can_update_task,
                    run,
                    pending,
                    continue_action,
                    context,
                    usage_totals,
                    shell,
                    workspace: entry.and_then(|h| h.workspace.clone()),
                    workspace_missing: entry.map(|h| h.workspace_missing).unwrap_or(false),
                }
            })
            .collect()
    }

    pub fn session_exists(&self, turn: &Turn, name: &str) -> bool {
        turn.holds(name)
    }
}

impl Work {
    /// 目的：界面投喂：把文件写进本次工作的 work/。
    ///  返回 Ok(false) = 同名文件已存在且未选择覆盖（交前端让用户决定：覆盖/改名/取消）。
    pub fn work_upload(
        &mut self,
        turn: &Turn,
        sid: &str,
        name: &str,
        bytes: &[u8],
        overwrite: bool,
    ) -> Result<bool, String> {
        // 策略在 runtime：先净化文件名，再交给工作区端口（流程只看已净化的名字）。
        let name = crate::capabilities::workspace::api::safe_file_name(name)?;
        if !turn.holds(sid) && self.history.load(sid).is_err() {
            return Err(format!("无此会话：{}", sid));
        }
        // 先恢复未完成的版本库事务，再做同名文件判定：否则 pending 里的文件会被当成不存在。
        let work = work_root(self.history.as_ref(), sid).unwrap_or_else(|_| sid.to_string());
        self.workspace.work_recover(&work)?;
        if self.workspace.work_has(&work, &name) && !overwrite {
            return Ok(false);
        }
        // 用户投喂 = 一次**权威提交**（作者 user）：共享区只有提交这一条写路径。
        // 锚到主会话当时的下一条行号，回档才能把这次投喂算进某个转录点。
        let line = match turn.session(sid) {
            Some(Session::Single(s)) => s.next_line,
            Some(Session::Flow(m)) => m.next_line(),
            None => 0,
        };
        self.workspace.work_commit_user(
            &work,
            &name,
            bytes,
            crate::kernel::api::procs::unix_seconds(),
            line,
        )?;
        Ok(true)
    }

    /// 目的：本工作可引用的文件清单 + 真实根（前端 @ 菜单与「长路径缩写」用）。
    ///  名单取自会话 meta（活动会话与历史会话都以 meta.agents 为权威）；列目录的流程在 Workspace 端口，
    ///  根取自沙箱（与文件清单同源）：agents 与 roots.agents 同序同名。
    pub fn files_view(
        &self,
        turn: &Turn,
        registry: &dyn Registry,
        probe: &dyn HostProbe,
        sid: &str,
    ) -> Result<FilesView, String> {
        let (meta, _) = self
            .history
            .load(sid)
            .map_err(|_| format!("无此会话：{}", sid))?;
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        let files = self.workspace.files(sid, &names)?;
        let usage = self.workspace.usage(sid, &names)?;
        let roster = self.modules.roster();
        let sandboxes = turn.sandboxes(registry, probe, &meta, &roster)?;
        let mut agents: Vec<FilesAgentView> = Vec::new();
        let mut agent_roots: Vec<FilesAgentRootView> = Vec::new();
        for a in &meta.agents {
            agents.push(FilesAgentView {
                name: a.name.clone(),
                files: files.agents.get(&a.name).cloned().unwrap_or_default(),
            });
            agent_roots.push(FilesAgentRootView {
                name: a.name.clone(),
                root: sandboxes
                    .for_agent(&a.name)
                    .map(|s| crate::kernel::api::path::slash(&s.private))
                    .unwrap_or_default(),
            });
        }
        Ok(FilesView {
            work: files.work,
            agents,
            roots: FilesRootsView {
                work: crate::kernel::api::path::slash(&sandboxes.shared),
                agents: agent_roots,
            },
            usage,
        })
    }

    /// 目的：提交历史（只读）：工作根从会话的 meta 上溯；列表与当前 head 一起给。
    pub fn work_log(
        &self,
        sid: &str,
        limit: usize,
        before: Option<u64>,
    ) -> Result<crate::capabilities::workspace::api::WorkLog, String> {
        let work = work_root(self.history.as_ref(), sid)?;
        self.workspace.work_log(&work, limit, before)
    }

    /// 目的：一条提交的详情（只读）。
    pub fn work_commit_view(
        &self,
        sid: &str,
        commit: u64,
    ) -> Result<crate::capabilities::workspace::api::CommitDetail, String> {
        let work = work_root(self.history.as_ref(), sid)?;
        self.workspace.work_commit_view(&work, commit)
    }

    /// 目的：一条提交里某个文件的内容预览（只读）。
    pub fn work_commit_file(
        &self,
        sid: &str,
        commit: u64,
        path: &str,
    ) -> Result<crate::capabilities::workspace::api::CommitFileView, String> {
        let work = work_root(self.history.as_ref(), sid)?;
        self.workspace.work_commit_file(&work, commit, path)
    }

    /// 目的：用户把共享区回滚到某个提交点（非破坏性）：只改共享区与 head，目标之后的提交仍保留。
    /// 约束：**同一工作子树里任一生成中会话都拒绝**；expect_head 与当前 head 不符也拒绝。
    /// 返回：回滚后的 head + 要外送的 notice 事件（已落盘）。
    pub(crate) fn restore_work_commit(
        &mut self,
        turn: &mut Turn,
        sid: &str,
        commit: u64,
        expect_head: u64,
        force: bool,
    ) -> Result<(Option<u64>, Vec<SessionEvent>), String> {
        let work = work_root(self.history.as_ref(), sid)?;
        let tree = subtree_of(self.history.as_ref(), &work);
        if let Some(running) = turn.running_iter().find(|s| tree.iter().any(|x| x == *s)) {
            return Err(Turn::running_refusal(running));
        }
        let current = self.workspace.work_head(&work)?.unwrap_or(0);
        if current != expect_head {
            return Err(format!(
                "共享区已经变化：你看到的是 head #{}，现在是 #{}；先刷新提交历史再回滚",
                expect_head, current
            ));
        }
        let target = self.workspace.work_commit_view(&work, commit)?;
        if force {
            self.workspace.work_restore_force(&work, commit)?;
        } else {
            self.workspace.work_restore(&work, commit)?;
        }
        let after = self.workspace.work_head(&work)?;
        let before = if current == 0 {
            "空共享区".to_string()
        } else {
            format!("提交 #{}", current)
        };
        let text = format!(
            "[工作区回滚] 共享区已从{}回滚到提交 #{}（{}：{}）。目标之后的提交仍保留在历史里，可再次回滚；各 agent 沙箱未改动，后续拉取可能遇到冲突。",
            before, commit, target.author, target.message
        );
        let mut events = self.notice_row(turn, sid, &text);
        self.record_events(sid, &mut events);
        Ok((after, events))
    }
}

impl Work {
    /// 目的：历史列表：**树序**（每个父会话紧跟它的子会话）——侧栏据此缩进，
    ///  顺序与缩进同源，不会再出现"子会话排到父会话上面"的错位（见 session-model.md 一）。
    ///  读盘失败如实记日志，返回空表。
    pub(super) fn history_list(&self) -> Vec<HistoryView> {
        match self.history.list() {
            Ok(v) => crate::capabilities::conversation::session::api::tree_order(v),
            Err(e) => {
                self.log
                    .error("work::history_list", &format!("会话列表失败：{}", e));
                Vec::new()
            }
        }
    }

    /// 目的：事件落盘；失败如实告知（追加一条警告事件），不静默丢历史。
    pub(super) fn record_events(&self, sid: &str, events: &mut Vec<SessionEvent>) {
        crate::capabilities::conversation::session::api::record_events(
            self.history.as_ref(),
            self.log.as_ref(),
            sid,
            events,
        );
    }

    /// 目的：把一条内容类系统提示记成**转录行**（有 id、可回档/折叠）；返回要外送的事件。
    /// 约束：只在会话对象在核心表里时分配行 id；生成中的会话被取走、没有计数器可分配，
    ///   这时如实退回瞬时提示（界面照样看得见，只是不可回档）。
    pub(super) fn notice_row(
        &mut self,
        turn: &mut Turn,
        sid: &str,
        text: &str,
    ) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        match turn.session_mut(sid) {
            Some(Session::Single(s)) => out.extend(s.note_notice(text)),
            Some(Session::Flow(m)) => m.notice(text, &mut |e| out.push(e)),
            None => out.push(SessionEvent::Notice(text.to_string())),
        }
        out
    }

    /// 目的：打开历史会话：返回元信息与事件流（只读回放；是否续跑由用户点「继续」授权）。
    ///  回放同样应用 rewind 截断——流水保留审计，会话内容以截断后为准。
    pub(super) fn history_open(
        &self,
        sid: &str,
    ) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let (meta, events) = self.history.load(sid)?;
        Ok((
            meta,
            crate::capabilities::conversation::session::api::truncate_events(&events),
        ))
    }

    /// 目的：取这个会话的**裁决队**（核心各关卡与工具级确认共用同一条）。
    /// 约束：新队按转录里已发出的卡号续号——卡号会话内唯一、跨重启稳定（已答过的不重问）。
    pub(super) fn desk_of(&self, turn: &Turn, sid: &str) -> Arc<dyn Gates> {
        if let Some(d) = turn.desk().peek_gate(sid) {
            return d;
        }
        let issued = self
            .history
            .load(sid)
            .map(|(_, ev)| crate::capabilities::conversation::session::api::issued_in(&ev))
            .unwrap_or(0);
        turn.desk_for(sid, issued)
    }
}
