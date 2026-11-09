//! 目的：会话中心与工作文件的视图——在世会话 × 历史的并集、继续判据、文件视图 / 上传 / 日志 / 提交与恢复。
//! 管：会话视图与存在性、继续判据、文件视图、上传、日志、提交视图与回滚恢复。
//! 不管：会话怎么建（`create.rs`）、配置怎么改（`config.rs`）、提交本身（workspace）。
//! 联动：api 层的会话与文件接口直接转发到这里。
//!   本方法族只碰 workspace / sessions / history / running 这四个字段。

use super::*;

impl Conductor {
    /// 目的：上下文「继续」是否该出现——判据只在后端（前端不按末条/模式猜）。
    /// 约束：正在跑、已关闭、代理、或没有真实断点时不出现；协作有挂起卡时由卡承担，不给继续。
    fn continue_action(
        &self,
        sid: &str,
        running: bool,
        mode: &str,
        run: &str,
        has_pending: bool,
    ) -> Option<ContinueView> {
        if running || run == "closed" || mode == "proxy" {
            return None;
        }
        match mode {
            "single" => {
                let user_last =
                    matches!(self.sessions.get(sid), Some(Session::Single(s)) if s.last_is_user());
                if user_last {
                    Some(ContinueView {
                        kind: "turn".to_string(),
                        label: "继续上一轮".to_string(),
                        reason: "上一轮还没有回答".to_string(),
                    })
                } else {
                    None
                }
            }
            "collab" if !has_pending => Some(ContinueView {
                kind: "collab".to_string(),
                label: "恢复协作".to_string(),
                reason: "协作泵没有在跑".to_string(),
            }),
            _ => None,
        }
    }

    /// 目的：一条会话的权威视图（启用回包 / 正在生成时附着用）：状态、运行态与上下文继续。
    pub(crate) fn session_view(&self, sid: &str) -> Result<SessionView, String> {
        self.session_views(&self.history_list())
            .into_iter()
            .find(|v| v.sid == sid)
            .ok_or_else(|| format!("无此会话：{}", sid))
    }

    /// 目的：会话列表视图（进行中的工作）。形态取落盘 meta（单一真相，不在内存里留影子状态）。
    pub fn session_views(&self, history: &[HistoryView]) -> Vec<SessionView> {
        // 在表里的会话 + **正在生成的会话**（后者对象在工作线程上，但它确实存在、也确实在跑）。
        // 漏掉它们会让界面以为会话不见了。
        let running: Vec<(String, bool)> = self
            .running
            .iter()
            .map(|sid| (sid.clone(), false))
            .collect();
        let running_now: std::collections::BTreeSet<String> =
            self.running.iter().cloned().collect();
        let listed: Vec<(String, bool)> = self
            .sessions
            .iter()
            .map(|(sid, s)| {
                let done = match s {
                    Session::Collab(c) => c.is_done(),
                    Session::Single(_) => false,
                };
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
                let mode = entry.map(|h| h.mode.clone()).unwrap_or_default();
                // 运行态取落盘事实（不在内存里留影子状态）；拿不到（生成中 / 未落盘）按正常运行。
                let run = entry
                    .map(|h| h.run.as_str())
                    .unwrap_or("active")
                    .to_string();
                // 记的档位来自落盘 meta（权威）：环境变了也要如实提示——**不拦打开**（记录是用户的）。
                let exec = entry.map(|h| h.exec.clone()).unwrap_or_default();
                let qemu = self.qemu_path();
                let share_tool = self.share_tool_path();
                let readiness = crate::capabilities::vm::api::tier_readiness(
                    &self.root,
                    &exec,
                    qemu.as_deref(),
                    share_tool.as_deref(),
                    self.probe.as_ref(),
                );
                // 「改需求」能力位：有本次需求行才给。在表里看会话种类；不在表里（生成中/未打开）
                // 看落盘 meta 的形态（那是名单与形态的单一真相）。
                let can_update_task = match self.sessions.get(&sid) {
                    Some(Session::Collab(_)) => true,
                    Some(Session::Single(_)) => false,
                    None => mode == "collab",
                };
                // 待裁决：**判据只有队本身**（核心各关卡与工具级确认排在同一条队上）——
                // 正在生成的会话（对象在工作线程手里）照样取得到；推的卡片事件与它是同一份。
                let pending = self.desk.peek(&sid).and_then(|d| d.json());
                let running = running_now.contains(&sid);
                let continue_action =
                    self.continue_action(&sid, running, &mode, &run, pending.is_some());
                // 只有持续对话的会话才有单一上下文；协作主会话由核心按次组上下文，不伪造数字。
                let context = match self.sessions.get(&sid) {
                    Some(Session::Single(s)) => s.context_usage(),
                    _ => None,
                };
                let (permission_profile, permission_label, permission_narrowed, permission_parent) =
                    match self.history.meta(&sid) {
                        Ok(m) => {
                            let parent = m.parent.as_ref().and_then(|p| {
                                self.history.meta(p).ok().and_then(|pm| {
                                    self.permissions_for(&pm)
                                        .ok()
                                        .map(|ep| ep.profile.as_str().to_string())
                                })
                            });
                            match self.permissions_for(&m) {
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
                let shell = self.shell_view(&sid);
                // 过期内容替换是**工作根**的会话级事实：子会话与工作根共享同一个开关。
                let context_freshness = match self.work_root(&sid) {
                    Ok(root) => self
                        .history
                        .meta(&root)
                        .map(|m| m.context_freshness)
                        .unwrap_or(false),
                    Err(_) => false,
                };
                let thinking = match self.work_root(&sid) {
                    Ok(root) => thinking_cache
                        .entry(root.clone())
                        .or_insert_with(|| self.thinking_view_for_root(&root))
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
                    mode,
                    done,
                    tier: exec.tier.as_str().to_string(),
                    tier_ready: readiness.ready(),
                    tier_missing: readiness.missing().iter().map(|s| s.to_string()).collect(),
                    permission_profile,
                    permission_label,
                    thinking,
                    context_freshness,
                    permission_narrowed,
                    permission_parent,
                    can_update_task,
                    run,
                    pending,
                    continue_action,
                    context,
                    shell,
                    workspace: entry.and_then(|h| h.workspace.clone()),
                    workspace_missing: entry.map(|h| h.workspace_missing).unwrap_or(false),
                }
            })
            .collect()
    }

    pub fn session_exists(&self, name: &str) -> bool {
        self.sessions.contains_key(name)
    }
}

impl Conductor {
    /// 目的：界面投喂：把文件写进本次工作的 work/。
    ///  返回 Ok(false) = 同名文件已存在且未选择覆盖（交前端让用户决定：覆盖/改名/取消）。
    pub fn work_upload(
        &mut self,
        sid: &str,
        name: &str,
        bytes: &[u8],
        overwrite: bool,
    ) -> Result<bool, String> {
        // 策略在 conductor：先净化文件名，再交给工作区端口（机制只看已净化的名字）。
        let name = crate::capabilities::workspace::api::safe_file_name(name)?;
        if !self.sessions.contains_key(sid) && self.history.load(sid).is_err() {
            return Err(format!("无此会话：{}", sid));
        }
        // 先恢复未完成的版本库事务，再做同名文件判定：否则 pending 里的文件会被当成不存在。
        let work = self.work_root(sid).unwrap_or_else(|_| sid.to_string());
        self.workspace.work_recover(&work)?;
        if self.workspace.work_has(&work, &name) && !overwrite {
            return Ok(false);
        }
        // 用户投喂 = 一次**权威提交**（作者 user）：共享区只有提交这一条写路径。
        // 锚到主会话当时的下一条行号，回档才能把这次投喂算进某个转录点。
        let line = match self.sessions.get(sid) {
            Some(Session::Single(s)) => s.next_line,
            Some(Session::Collab(c)) => c.next_line,
            None => 0,
        };
        self.workspace
            .work_commit_user(&work, &name, bytes, now_ts(), line)?;
        Ok(true)
    }

    /// 目的：本工作可引用的文件清单 + 真实根（前端 @ 菜单与「长路径缩写」用）。
    ///  名单取自会话 meta（活动会话与历史会话都以 meta.agents 为权威）；列目录的机制在 Workspace 端口，
    ///  根取自沙箱（与文件清单同源）：agents 与 roots.agents 同序同名。
    pub fn files_view(&self, sid: &str) -> Result<FilesView, String> {
        let (meta, _) = self
            .history
            .load(sid)
            .map_err(|_| format!("无此会话：{}", sid))?;
        let names: Vec<String> = meta.agents.iter().map(|a| a.name.clone()).collect();
        let files = self.workspace.files(sid, &names)?;
        let usage = self.workspace.usage(sid, &names)?;
        let roster = self.scan();
        let sandboxes = self.sandboxes(&meta, &roster)?;
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
                    .map(|s| crate::kernel::api::slash(&s.private))
                    .unwrap_or_default(),
            });
        }
        Ok(FilesView {
            work: files.work,
            agents,
            roots: FilesRootsView {
                work: crate::kernel::api::slash(&sandboxes.shared),
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
        let work = self.work_root(sid)?;
        self.workspace.work_log(&work, limit, before)
    }

    /// 目的：一条提交的详情（只读）。
    pub fn work_commit_view(
        &self,
        sid: &str,
        commit: u64,
    ) -> Result<crate::capabilities::workspace::api::CommitDetail, String> {
        let work = self.work_root(sid)?;
        self.workspace.work_commit_view(&work, commit)
    }

    /// 目的：一条提交里某个文件的内容预览（只读）。
    pub fn work_commit_file(
        &self,
        sid: &str,
        commit: u64,
        path: &str,
    ) -> Result<crate::capabilities::workspace::api::CommitFileView, String> {
        let work = self.work_root(sid)?;
        self.workspace.work_commit_file(&work, commit, path)
    }

    /// 目的：用户把共享区回滚到某个提交点（非破坏性）：只改共享区与 head，目标之后的提交仍保留。
    /// 约束：**同一工作子树里任一生成中会话都拒绝**；expect_head 与当前 head 不符也拒绝。
    /// 返回：回滚后的 head + 要外送的 notice 事件（已落盘）。
    pub(crate) fn restore_work_commit(
        &mut self,
        sid: &str,
        commit: u64,
        expect_head: u64,
        force: bool,
    ) -> Result<(Option<u64>, Vec<SessionEvent>), String> {
        let work = self.work_root(sid)?;
        let tree = self.subtree_of(&work);
        if let Some(running) = self.running.iter().find(|s| tree.iter().any(|x| x == *s)) {
            return Err(Self::running_refusal(running));
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
        let mut events = self.notice_row(sid, &text);
        self.record_events(sid, &mut events);
        Ok((after, events))
    }
}
