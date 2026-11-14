//! 目的：历史与落盘——历史列出 / 打开 / 删除、事件收编（record_events）与增量落盘手柄。
//! 管：什么时候写、写什么（落盘机制在 session 的 HistoryStore 端口后面）。
//! 不管：会话对象怎么建（flow.rs 与 work/create.rs）、视图怎么渲染（work/views.rs）。
//! 联动：所有方法族的事件都经 record_events 落盘。
//!   本方法族只碰 history / log / sessions / vm 这几个字段。
use super::*;

/// 目的：按租约撤围栏授权的回调——由核心把 `ToolExec::release_by_lease` 包成闭包注入，测试可直接换成记录型替身。
pub(crate) type ReleaseByLease = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// 目的：删除收尾里的围栏撤权段：租约清单 + 撤销回调 + 失败日志。
/// 约束：在调用方线程上**只起线程**；真正撤权在后台跑，进程退出时线程被杀留下的无归属条目由启动对账回收。
pub(crate) struct FenceRelease {
    leases: Vec<String>,
    release: ReleaseByLease,
    log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
}

impl FenceRelease {
    /// 目的：起后台线程按租约逐个撤权；线程起不来就同步兜底，不能把撤权丢掉。
    pub(crate) fn spawn(self) {
        if self.leases.is_empty() {
            return;
        }
        let leases = self.leases;
        let release = Arc::clone(&self.release);
        let log = Arc::clone(&self.log);
        let worker = {
            let leases = leases.clone();
            let release = Arc::clone(&release);
            let log = Arc::clone(&log);
            move || {
                for lease in leases {
                    if let Err(e) = release(&lease) {
                        log.warn(
                            "conductor::release_fences",
                            &format!("按会话撤销围栏授权未完成（{}）：{}", lease, e),
                        );
                    }
                }
            }
        };
        match std::thread::Builder::new()
            .name("fence-release".to_string())
            .spawn(worker)
        {
            Ok(_) => {}
            Err(e) => {
                log.warn(
                    "conductor::release_fences",
                    &format!("后台撤权线程起不来，改为同步撤权：{}", e),
                );
                for lease in &leases {
                    if let Err(e) = release(lease) {
                        log.warn(
                            "conductor::release_fences",
                            &format!("按会话撤销围栏授权未完成（{}）：{}", lease, e),
                        );
                    }
                }
            }
        }
    }
}

/// 目的：删除会话的收尾——**先停 guest，再删会话目录，最后把撤权甩到后台**。
/// 约束：在调用方线程上执行；Windows 上 guest / 共享进程可能还占着会话目录里的文件，顺序反过来会 sharing violation。
///   先删目录让产品根内的授权自动走「路径不在 → 只销账」快分支；外部根的递归撤权不占用户等待路径。
pub(crate) struct SessionDelete {
    stop: super::guest_job::GuestStop,
    history: Arc<dyn History + Send + Sync>,
    /// 待删目录（已按**先子后父**排好）。
    dirs: Vec<String>,
    /// 调用方要的“目标会话是否真的删掉了”。
    target: String,
    /// 围栏撤权段；没有租约或测试替身不需要时为 None。
    fences: Option<FenceRelease>,
}

impl SessionDelete {
    #[cfg(test)]
    pub(crate) fn for_test(
        stop: super::guest_job::GuestStop,
        history: Arc<dyn History + Send + Sync>,
        dirs: Vec<String>,
        target: String,
    ) -> SessionDelete {
        SessionDelete {
            stop,
            history,
            dirs,
            target,
            fences: None,
        }
    }

    /// 目的：测试用构造：带一个可观察的撤权回调，钉住「后台执行、目录已删、失败也撤」三条契约。
    #[cfg(test)]
    pub(crate) fn for_test_with_fences(
        stop: super::guest_job::GuestStop,
        history: Arc<dyn History + Send + Sync>,
        dirs: Vec<String>,
        target: String,
        leases: Vec<String>,
        release: ReleaseByLease,
        log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    ) -> SessionDelete {
        SessionDelete {
            stop,
            history,
            dirs,
            target,
            fences: Some(FenceRelease {
                leases,
                release,
                log,
            }),
        }
    }

    pub(crate) fn run(self) -> Result<bool, String> {
        let SessionDelete {
            stop,
            history,
            dirs,
            target,
            fences,
        } = self;
        stop.run();
        let mut result = Ok(false);
        for sid in dirs {
            match history.delete(&sid) {
                Ok(ok) => {
                    if sid == target {
                        result = Ok(ok);
                    }
                }
                Err(e) => {
                    // 保留原有的逐目录早退：删到一半失败就停，但撤权仍要执行。
                    result = Err(e);
                    break;
                }
            }
        }
        if let Some(fences) = fences {
            fences.spawn();
        }
        result
    }
}

impl Conductor {
    /// 目的：历史列表：**树序**（每个父会话紧跟它的子会话）——侧栏据此缩进，
    ///  顺序与缩进同源，不会再出现"子会话排到父会话上面"的错位（见 session-model.md 一）。
    ///  读盘失败如实记日志，返回空表。
    pub fn history_list(&self) -> Vec<HistoryView> {
        match self.history.list() {
            Ok(v) => tree_order(v),
            Err(e) => {
                self.log
                    .error("conductor::history_list", &format!("会话列表失败：{}", e));
                Vec::new()
            }
        }
    }

    /// 目的：打开历史会话：返回元信息与事件流（只读回放；是否续跑由用户点「继续」授权）。
    ///  回放同样应用 rewind 截断——流水保留审计，会话内容以截断后为准。
    pub fn history_open(
        &self,
        name: &str,
    ) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let (meta, events) = self.history.load(name)?;
        Ok((
            meta,
            crate::capabilities::session::api::truncate_events(&events),
        ))
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
            self.sessions.remove(name);
            return Ok(SessionDelete {
                stop: super::guest_job::GuestStop::new(Arc::clone(&self.vm), Vec::new()),
                history: Arc::clone(&self.history),
                dirs: vec![name.to_string()],
                target: name.to_string(),
                fences: None,
            });
        };
        // 子会话由核心管理，不允许用户单独删。
        if let Some(parent) = meta.parent.clone() {
            return Err(format!(
                "会话 {} 是核心管理的子会话（父会话 {}）：删父会话会一起删掉它，不能单独删",
                name, parent
            ));
        }
        let subtree = self.subtree_of(name);
        for sid in &subtree {
            if self.running.contains(sid) {
                return Err(Self::running_refusal(sid));
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
            if let Ok(Some((lease, _))) = self.guest_scope(&meta)
                && !guest_leases.iter().any(|l| l == &lease)
            {
                guest_leases.push(lease);
            }
        }
        let stop = super::guest_job::GuestStop::new(Arc::clone(&self.vm), guest_leases);
        for sid in &subtree {
            self.sessions.remove(sid);
            // 会话没了，它的裁决队随会话一起消失（等待方与转录都不复存在）。
            self.forget_desk(sid);
        }
        // 目录按后序删：先子后父；父目录被删时，落在它内部的子会话目录一起消失。
        // 真正删除在 `SessionDelete::run`（先停 guest，再删目录，最后后台撤权）。
        let dirs: Vec<String> = subtree.iter().rev().cloned().collect();
        let tools = Arc::clone(&self.tools);
        let release: ReleaseByLease = Arc::new(move |lease: &str| tools.release_by_lease(lease));
        let fences = FenceRelease {
            leases: subtree.clone(),
            release,
            log: Arc::clone(&self.log),
        };
        Ok(SessionDelete {
            stop,
            history: Arc::clone(&self.history),
            dirs,
            target: name.to_string(),
            fences: Some(fences),
        })
    }

    /// 目的：一个会话的整棵子树（含它自己，父在前、子在后）：父子关系以 `meta.parent` 为唯一判据。
    pub(crate) fn subtree_of(&self, name: &str) -> Vec<String> {
        let list = self.history_list();
        let mut out = vec![name.to_string()];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i].clone();
            for h in &list {
                if h.parent.as_deref() == Some(cur.as_str()) && !out.iter().any(|x| x == &h.name) {
                    out.push(h.name.clone());
                }
            }
            i += 1;
        }
        out
    }

    /// 目的：这个会话的**使用类型**（机制册的第一把钥匙）：按落盘形态派生，重建后是同一份。
    ///  协作派生的成员 / 节点会话算 **collab**（它属于那场协作），其余单 agent 会话算 single，代理算 proxy。
    pub(crate) fn session_kind_of(
        &self,
        meta: &crate::capabilities::session::api::SessionMeta,
    ) -> Result<String, String> {
        match meta.mode.as_str() {
            "proxy" => Ok("proxy".to_string()),
            "collab" => Ok("collab".to_string()),
            _ => match meta.parent.as_deref() {
                Some(p) => match self.history.meta(p) {
                    Ok(pm) if pm.mode == "collab" => Ok("collab".to_string()),
                    _ => Ok("single".to_string()),
                },
                None => Ok("single".to_string()),
            },
        }
    }

    /// 目的：这一席的**角色**（机制册的第二把钥匙，也是工具面与提示词的角色名）。
    ///  与实时建立同一把尺子：协作的成员 / 节点 = executor；代理 = core_proxy；其余单 agent 工作 = solo。
    pub(crate) fn role_of(
        &self,
        meta: &crate::capabilities::session::api::SessionMeta,
    ) -> Result<String, String> {
        if meta.mode == "proxy" {
            return Ok("core_proxy".to_string());
        }
        match meta.parent.as_deref() {
            Some(p) => match self.history.meta(p) {
                Ok(pm) if pm.mode == "collab" => Ok("executor".to_string()),
                _ => Ok("solo".to_string()),
            },
            None => Ok("solo".to_string()),
        }
    }

    /// 目的：一个会话沿父链继承的工作区根（仅外部绑定；`None` = 产品内布局）。
    /// 约束：与 `work_root_of` 同一走向：自己带绑定就用它，否则看工作根的 meta。
    ///   取到根后按它所在会话的档位复检（见 workspace_root）——meta 是磁盘上的文本，手改也要过同一套策略。
    pub(crate) fn workspace_binding_of(
        &self,
        meta: &SessionMeta,
    ) -> Result<Option<std::path::PathBuf>, String> {
        let (root, tier) = match &meta.workspace {
            Some(binding) => (binding.root.clone(), meta.exec.tier),
            None => match &meta.parent {
                None => return Ok(None),
                Some(p) => {
                    let top = self.work_root(p)?;
                    let top_meta = self.history.meta(&top)?;
                    match top_meta.workspace {
                        Some(binding) => (binding.root, top_meta.exec.tier),
                        None => return Ok(None),
                    }
                }
            },
        };
        let probe = self.probe.as_ref();
        let product = super::workspace_root::product_root(&self.root, probe);
        super::workspace_root::recheck(&root, tier, &product, probe)?;
        Ok(Some(root))
    }

    /// 目的：一个会话的**工作根**，从它自己的 meta 起算：`meta` 可能**还没落盘**
    ///  （建工作区发生在落盘之前），所以有父就上溯、没有父就是它自己。
    pub(crate) fn work_root_of(&self, meta: &SessionMeta) -> Result<String, String> {
        match &meta.parent {
            None => Ok(meta.name.clone()),
            Some(p) => self.work_root(p),
        }
    }

    /// 目的：把整棵子树（含自己）的运行态落成同一个值：**停止**先整棵冻上，**继续**再整棵解开。
    ///  停止时顺序不能反——先冻态再取消生成，被停会话收尾写的那条"这一轮结束"才会被闸门拒掉。
    pub(crate) fn set_subtree_run(
        &self,
        name: &str,
        run: crate::capabilities::session::api::RunState,
    ) -> Result<Vec<String>, String> {
        let tree = self.subtree_of(name);
        for sid in &tree {
            let mut meta = self.history.meta(sid)?;
            if meta.run != run {
                meta.run = run;
                self.history.save_meta(&meta)?;
            }
        }
        Ok(tree)
    }

    /// 目的：解除整棵子树的「已停止」（停止的逆操作）；返回是否确实解除了至少一个。
    ///  子树里只要有**已关闭**的就整条拒绝——关闭是终态，不能被「继续」拉回来。
    pub(crate) fn resume_subtree(&self, name: &str) -> Result<bool, String> {
        use crate::capabilities::session::api::RunState;
        let mut any = false;
        for sid in self.subtree_of(name) {
            let mut meta = self.history.meta(&sid)?;
            match meta.run {
                RunState::Closed => {
                    return Err(format!("会话 {} 已关闭：终态，不能再继续", sid));
                }
                RunState::Stopped => {
                    meta.run = RunState::Active;
                    self.history.save_meta(&meta)?;
                    any = true;
                }
                RunState::Active => {}
            }
        }
        Ok(any)
    }

    /// 目的：一个会话的**工作根**（顶层会话名）：沿 `meta.parent` 一路走到顶。
    ///  整棵树不论嵌套多少层只有一个 `work/`——共享区与 agent 沙箱都锚在它上面。
    pub(crate) fn work_root(&self, name: &str) -> Result<String, String> {
        let mut cur = self.history.meta(name)?;
        let mut guard = 0usize;
        while let Some(p) = cur.parent.clone() {
            guard += 1;
            if guard > 64 {
                return Err(format!("会话父子关系成环：{}", name));
            }
            cur = self.history.meta(&p)?;
        }
        Ok(cur.name)
    }

    /// 目的：**运行态闸门**：派发 / 唤醒前过它，停止与关闭都拒绝。
    ///  拿不到 meta（系统会话等未落盘）就当正常运行——运行态是落盘事实，
    ///  不存在的会话自有别的检查兜底（这里不替它报"无此会话"）。
    pub(crate) fn dispatch_gate(&self, sid: &str) -> Result<(), String> {
        match self.history.meta(sid) {
            Ok(meta) => match meta.dispatch_refusal() {
                Some(e) => Err(e),
                None => Ok(()),
            },
            Err(_) => Ok(()),
        }
    }

    /// 目的：派发目标的形态 + 这一次派发放不放行（放行 = Ok(mode)）。
    ///  代理转达与叫醒共用这一处，免得"看不看运行态"两处口径不一致。
    pub(crate) fn dispatch_target(&self, sid: &str) -> Result<String, String> {
        let meta = self.history.meta(sid)?;
        match meta.dispatch_refusal() {
            Some(e) => Err(e),
            None => Ok(meta.mode),
        }
    }

    /// 目的：事件落盘；失败如实告知（追加一条警告事件），不静默丢历史。
    pub(crate) fn record_events(&self, sid: &str, events: &mut Vec<SessionEvent>) {
        if events.is_empty() {
            return;
        }
        if let Some(warn) = persist_events(self.history.as_ref(), self.log.as_ref(), sid, events) {
            events.push(SessionEvent::Notice(warn));
        }
    }

    /// 目的：把一条内容类系统提示记成**转录行**（有 id、可回档/折叠）；返回要外送的事件。
    /// 约束：只在会话对象在核心表里时分配行 id；生成中的会话被取走、没有计数器可分配，
    ///   这时如实退回瞬时提示（界面照样看得见，只是不可回档）。
    pub(crate) fn notice_row(&mut self, sid: &str, text: &str) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        match self.sessions.get_mut(sid) {
            Some(Session::Single(s)) => out.extend(s.note_notice(text)),
            Some(Session::Collab(c)) => c.notice(text, &mut |e| out.push(e)),
            None => out.push(SessionEvent::Notice(text.to_string())),
        }
        out
    }

    /// 目的：增量落盘手柄：交给工作线程，按"一次模型调用"的粒度落盘（见 `Persister`）。
    pub(crate) fn persister(&self, sid: &str) -> Persister {
        Persister {
            history: Arc::clone(&self.history),
            log: Arc::clone(&self.log),
            sid: sid.to_string(),
        }
    }
}
