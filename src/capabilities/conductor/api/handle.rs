//! **核心手柄**：把 `Conductor` 搬到它自己的执行线程，命令与事件都经这里。
//!
//! 队列只占「取 / 交」两步：**长步骤生成在核心线程上跑**（见 docs/presentation/contracts.md）；
//! 停止 / 取消由 `kernel::api::JobRegistry` 承担、不进队列，所以生成期间照样立刻生效。

use super::*;
use crate::capabilities::conductor::service::{Conductor, Prepared};
use crate::capabilities::session::api::Live;
pub use crate::capabilities::session::api::SessionEvent;
use crate::kernel::api::JobRegistry;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self};
use std::sync::Arc;
impl ConductorHandle {
    /// 把核心搬到它自己的执行线程：此后所有核心状态只被这一个线程碰。
    /// 全部手柄丢弃后线程自然结束（通道断开即退出）。
    pub fn spawn(core: Conductor) -> Result<ConductorHandle, String> {
        let worker_log = core.log_handle();
        let jobs = JobRegistry::new();
        let approvals = crate::kernel::api::ApprovalRegistry::new();
        let approval_enabled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let bus = EventBus::new();
        let (tx, rx) = mpsc::channel::<Job>();
        let book = core.systools_book();
        let texts = core.prompt_texts();
        let handle = ConductorHandle {
            tx,
            jobs,
            approvals,
            approval_enabled,
            bus,
            log: Arc::clone(&worker_log),
            book,
            texts,
        };
        // 注意：工作线程**绝不能**捕获取手柄（那会持有一个 Sender，通道永不闭合、线程永不退出）。
        std::thread::Builder::new()
            .name("solomni-core".to_string())
            .spawn(move || {
                let mut core = core;
                while let Ok(job) = rx.recv() {
                    // 一条命令 panic 不该带走整个核心：接住并继续（回包通道随闭包销毁，调用方会看到「无回应」）。
                    if catch_unwind(AssertUnwindSafe(|| job(&mut core))).is_err() {
                        worker_log.error(
                            "conductor::api",
                            "核心命令 panic：已接住，核心继续服务（请查上面的 panic 现场）",
                        );
                    }
                }
                worker_log.info("conductor::api", "核心线程退出：全部手柄已释放");
            })
            .map_err(|e| format!("启动核心线程失败：{}", e))?;
        Ok(handle)
    }

    /// 发一条命令并等回复：呈现层因此仍是「调用即拿结果」，不需要改成异步。
    pub(crate) fn call<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Conductor) -> Result<T, String> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move |core: &mut Conductor| {
            let _ = tx.send(f(core));
        });
        self.tx
            .send(job)
            .map_err(|_| "核心已停止：无法发送命令".to_string())?;
        rx.recv()
            .map_err(|_| "核心无回应：命令执行中发生 panic，或核心线程已停止".to_string())?
    }

    /// 打开工具级确认：有交互前端（Web）在服务时才调，纯终端不调（生成线程不能空等一个没人回答的问题）。
    pub fn allow_tool_approval(&self) {
        self.approval_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// 本核心的事件台（多端订阅）。
    pub fn events(&self) -> Arc<EventBus> {
        Arc::clone(&self.bus)
    }

    /// **级联停止**：把 root 及其整棵子树（按 `meta.parent`）里正在生成的会话都停下来。
    /// 返回实际停下的会话 id——不假装“停止了一个本来就没在跑的会话”。
    /// 代理核心的 `stop` 走这条：主会话停 = 所有相关工作一起停（用户看得见的那一下）。
    pub(crate) fn stop_tree(&self, root: &str) -> Result<Vec<String>, String> {
        let root_owned = root.to_string();
        let tree = self.call(move |core| Ok(core.subtree_of(&root_owned)))?;
        let mut stopped = Vec::new();
        for sid in tree {
            // 停止也要解掉在等的工具确认，否则生成线程会一直等下去（wait 返回 None = 拒绝）。
            self.approvals.cancel(&sid);
            if self.jobs.stop(&sid) {
                stopped.push(sid);
            }
        }
        Ok(stopped)
    }

    /// 代理会话：把代理工具的成员侧执行面（`ProxyHandler`）装进这一回合的工具环境。
    /// 执行经队列桥回到核心线程，核心状态的所有权不变。非代理会话、或已装过，什么都不做。
    fn inject_proxy_handler(
        &self,
        sid: &str,
        session: &mut crate::capabilities::session::api::AgentSession,
    ) -> Result<(), String> {
        use crate::capabilities::conductor::domain::proxy as dp;
        let Some(tools) = session.tools.as_mut() else {
            return Ok(());
        };
        if !tools.handlers.is_empty() || !tools.allowed.iter().any(|t| dp::is_proxy_tool(t)) {
            return Ok(());
        }
        let face = tools.allowed.clone();
        let granted = {
            let sid = sid.to_string();
            self.call(move |core| Ok(core.history_open(&sid)?.0.delegation.map(|d| d.granted_at)))?
        };
        let grant = granted.map(|_| dp::Grant {
            tools: face,
            ..Default::default()
        });
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let host: std::sync::Arc<
            dyn crate::capabilities::conductor::ports::ProxyHost + Send + Sync,
        > = std::sync::Arc::new(
            crate::capabilities::conductor::service::proxy::ProxyBridge::new(self.clone()),
        );
        let handler = crate::capabilities::conductor::service::proxy::ProxyHandler::new(
            host,
            self.book.clone(),
            std::sync::Arc::clone(&self.texts),
            dp::ProxyCall {
                grant,
                parent: Some(sid.to_string()),
                now,
            },
        );
        tools.handlers = vec![std::sync::Arc::new(handler)];
        Ok(())
    }
    /// 测试专用：注入一条必定 panic 的命令，验证「一条命令 panic 不带垮整个核心」。
    #[cfg(test)]
    pub(crate) fn panic_probe(&self) -> Result<(), String> {
        self.call(|_core| -> Result<(), String> { panic!("入站契约测试注入的 panic") })
    }

    /// 单 agent 生成：**核心队列只占两步短命令**（取出会话 / 交回会话），生成本身在工作线程上跑。
    /// 为什么必须这样：生成要跑几十秒到几分钟，它若占着唯一的命令队列，
    /// 读接口（历史列表 / 状态）与其它会话的命令全排在它后面——界面因此"假死"。
    /// 不变量：会话状态只被一个线程碰——生成期间由工作线程独占，核心表里只留"生成中"这一态。
    pub(crate) fn single_generation(
        &self,
        sid: &str,
        text: Option<String>,
        out: Output,
    ) -> Result<Advance, String> {
        // ① 短命令：检查 + 把会话取出来 + 定下本次调用参数（都在核心线程上，毫秒级）。
        let prepared = self.call({
            let sid = sid.to_string();
            let t = text.clone();
            move |core| core.prepare_single(&sid, t.as_deref(), out == Output::Stream)
        })?;
        self.run_prepared(sid, prepared, text)
    }

    /// 一个节点的执行回合：**核心把任务提示词作为系统消息注入**（不是用户发言），再生成。
    /// 与 CLI 的 `Conductor::drive_node` 同一条语义——各前端只做各自的界面，管道只有这一条。
    pub(crate) fn node_generation(&self, child: &str, objective: &str) -> Result<Advance, String> {
        let prepared = self.call({
            let (child, objective) = (child.to_string(), objective.to_string());
            move |core| core.prepare_node(&child, &objective)
        })?;
        self.run_prepared(child, prepared, None)
    }

    /// 生成的工作线程与收尾：用户发言、**核心注入的任务**、继续都走这一条。
    pub(crate) fn run_prepared(
        &self,
        sid: &str,
        prepared: Prepared,
        text: Option<String>,
    ) -> Result<Advance, String> {
        let bus = Arc::clone(&self.bus);
        let jobs = Arc::clone(&self.jobs);
        let (session, identity, prefix, llm, persister) = match prepared {
            // 不用跑模型（例如"末条是 AI 发言"）：提示本身也是事实，进事件台；回包只给头部。
            Prepared::Immediate(events) => {
                let head = bus.push(sid, &events);
                return Ok(Advance { head });
            }
            // 协作会话的"继续"仍在核心线程上推进（B-1 只搬单 agent 生成）。
            Prepared::NotSingle => {
                if text.is_some() {
                    return Err("该会话不是单 agent 模式".to_string());
                }
                // 协作会话的"继续"：走同一条 own-and-return（泵在工作线程上）。
                return self.collab_generation(sid, CollabWork::Resume, "");
            }
            Prepared::Run {
                session,
                identity,
                prefix,
                llm,
                persister,
            } => (*session, identity, prefix, llm, persister),
        };
        let mut session = session;
        // 代理会话：把代理工具的成员侧执行面装进这一回合（经队列桥回核心线程执行）。
        self.inject_proxy_handler(sid, &mut session)?;
        let session = session;
        // 取消标志在**派发时**就登记：生成一开始「停止」就能生效（它不进命令队列）。
        let cancel = jobs.register(sid);
        // **运行态**：这条会话开始干活，推给它自己的事件台——节点执行、单 agent 发言、继续都走这里，
        // 打开它的标签页要立刻看到占位与「停止」按钮，而不是等 3 秒的状态轮询。
        // 事实只进事件台；命令回包只给头部序号（谁要看谁自己订阅）。
        let start_working = SessionEvent::Working {
            agent: Some(session.params().agent.clone()),
        };
        bus.push(sid, std::slice::from_ref(&start_working));
        // ② 工作线程：跑生成。短暂事件（流式增量 / 工具行）直送事件台——它是独立锁，不进核心队列。
        let approvals = Arc::clone(&self.approvals);
        let approval_enabled = self
            .approval_enabled
            .load(std::sync::atomic::Ordering::Relaxed);
        let worker = {
            let sid = sid.to_string();
            let bus = Arc::clone(&bus);
            std::thread::Builder::new()
                .name("solomni-gen".to_string())
                .spawn(move || {
                    let mut session = session;
                    let mut emit = {
                        let bus = Arc::clone(&bus);
                        let sid = sid.clone();
                        move |ev: SessionEvent| {
                            bus.push(&sid, std::slice::from_ref(&ev));
                        }
                    };
                    let mut live = Live {
                        llm,
                        cancel,
                        emit: &mut emit,
                        approval: approval_enabled.then(|| {
                            crate::kernel::api::ApprovalCtx::new(Arc::clone(&approvals), &sid)
                        }),
                    };
                    // 逐轮外送 + 边落盘：一轮跑完就上屏并落盘（中途刷新页面因此看得到已产生的部分）。
                    // seq 取**最后一次**入台的序号：命令回包按它给订阅起点。
                    let mut seq = 0u64;
                    let mut sink = |ev: SessionEvent| {
                        seq = bus.push(&sid, std::slice::from_ref(&ev));
                        if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                            bus.push(&sid, std::slice::from_ref(&SessionEvent::Notice(warn)));
                        }
                    };
                    // 生成前的提示（例如工具形态变更）先出，再跑。
                    for ev in prefix {
                        sink(ev);
                    }
                    match &text {
                        Some(t) => crate::capabilities::collab::api::say(
                            &mut session,
                            t,
                            &identity,
                            &mut live,
                            &mut sink,
                        ),
                        None => crate::capabilities::collab::api::continue_reply(
                            &mut session,
                            &identity,
                            &mut live,
                            &mut sink,
                        ),
                    }
                    (session, seq)
                })
                .map_err(|e| format!("起生成线程失败：{}", e))?
        };
        // ③ 收尾：拿回会话 → 入台 → 交回核心（重新插入 + 转录落盘）。
        //    **所有状态变更仍只发生在核心线程上**：工作线程只跑生成，不碰核心状态。
        let joined = worker.join();
        jobs.unregister(sid);
        // 运行态收尾：不管这一回合是跑完、崩了还是被用户停掉，都不再"在跑"。
        // seq 取**最后一批**（收尾这条）的序号：客户端按它去重（与逐轮外送同源）。
        let end_working = SessionEvent::Working { agent: None };
        let seq = bus.push(sid, std::slice::from_ref(&end_working));
        let (session, _worker_seq) = match joined {
            Ok(x) => x,
            Err(_) => {
                // 线程崩了：会话对象随线程没了，但**转录在盘上**——解除"生成中"，
                // 下次访问按落盘转录重建。绝不把会话卡在"生成中"。
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.abort_running(&sid);
                        Ok(())
                    }
                })?;
                // 崩了也要**叫醒父会话**：代理在"派完就等"之后，没有这条通知就会一直睡着。
                if let Some(proxy) = self.call({
                    let sid = sid.to_string();
                    move |core| Ok(core.notify_proxy_of_child(&sid))
                })? {
                    self.spawn_detached_proxy(&proxy);
                }
                return Err("生成线程崩溃：会话已按落盘转录保留，可继续".to_string());
            }
        };
        // 事实只进事件台（这一回合的行由 sink 逐轮入台，起止两条运行态也已入台）；
        // 回包只给头部序号——想看的端自己按 since 订阅，不在命令里捎带事实。
        // 交回核心只做"重新插入"：转录也已逐轮增量落盘。
        let parent = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.put_single(&sid, session))
        })?;
        // 这是**子会话**完成：叫醒父会话推进任务链（脱离本次调用，不等它跑完）。
        if let Some(parent) = parent {
            // 代理模式的父：叫醒它去判断下一步（不替它决定）；协作的父：推进任务链。
            let is_proxy = self
                .call({
                    let p = parent.clone();
                    move |core| Ok(core.session_mode_str(&p) == "proxy")
                })
                .unwrap_or(false);
            if is_proxy {
                self.spawn_detached_proxy(&parent);
            } else {
                self.spawn_detached_collab(&parent);
            }
        }
        Ok(Advance { head: seq })
    }

    /// 主线程侧：为一个成员回合取该 agent 的会话，跑完把回合结果带回来（并落进它自己的会话）。
    pub(crate) fn run_member_turn(
        &self,
        sid: &str,
        req: &AskReq,
    ) -> Result<crate::capabilities::collab::api::MemberTurn, String> {
        let child = format!("{}--{}", sid, req.agent);
        // 会话不存在就按需建（名单确认时已建，这里兜底）。
        {
            let (parent, agent, name) = (sid.to_string(), req.agent.clone(), child.clone());
            self.call(move |core| {
                if core.history_open(&name).is_err() {
                    core.spawn_agent_session(&parent, &agent)?;
                }
                Ok(())
            })?;
        }
        let session = self.call({
            let name = child.clone();
            move |core| core.take_single(&name)
        })?;
        // 这一回合的调用参数（流式 + 预算）：与单 agent 共用同一份全局设置。
        let llm = crate::capabilities::llm::api::LlmOpts {
            stream: req.opts.stream,
            timeout_secs: req.opts.timeout_secs,
        };
        let cancel = std::sync::Arc::clone(&req.cancel);
        let systools = req.systools.clone();
        let turn = req.turn.clone();
        let identity = req.identity.clone();
        let round = req.round;
        let turn_id = req.turn_id;
        // 子会话自己的**权威行**（逐轮）与**运行态收尾**都走它自己的事件台，
        // 并且**边产边落盘**：这个会话不经过主会话那条 sink，落盘手柄随线程带过去。
        let persister = self.call({
            let name = child.clone();
            move |core| Ok(core.persister(&name))
        })?;
        let bus = Arc::clone(&self.bus);
        let child_bus = Arc::clone(&self.bus);
        let child_sid = child.clone();
        let approvals = Arc::clone(&self.approvals);
        let approval_enabled = self
            .approval_enabled
            .load(std::sync::atomic::Ordering::Relaxed);
        let joined = std::thread::Builder::new()
            .name("solomni-member".to_string())
            .spawn(move || {
                let mut s = session;
                // **短暂事件**（流式增量）：按子会话的 sid 外送——打开它的会话就能看到它逐字在说。
                let mut emit = {
                    let bus = Arc::clone(&bus);
                    let sid = child_sid.clone();
                    move |ev: crate::capabilities::session::api::SessionEvent| {
                        bus.push(&sid, std::slice::from_ref(&ev));
                    }
                };
                let mut live = crate::capabilities::session::api::Live {
                    llm,
                    cancel: Arc::clone(&cancel),
                    emit: &mut emit,
                    approval: approval_enabled.then(|| {
                        crate::kernel::api::ApprovalCtx::new(Arc::clone(&approvals), &child_sid)
                    }),
                };
                // 权威行与通知也进它自己的台，并在产出的当下落盘（重建与实时同源）。
                let mut sink = |ev: crate::capabilities::session::api::SessionEvent| {
                    bus.push(&child_sid, std::slice::from_ref(&ev));
                    if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                        bus.push(
                            &child_sid,
                            std::slice::from_ref(&SessionEvent::Notice(warn)),
                        );
                    }
                };
                // 这一回合的工具面**由角色表发放**（讨论席：动词 + 只读核实工具）。
                let face = systools.role_face("discussant");
                // 这一回合：身份块（现渲染）+ 本回合工具面 + 该会话的**对话** + 开场/轮转词 + 表态。
                let ran = crate::capabilities::collab::api::discussion_turn(
                    &mut s, &identity, face, turn, turn_id, round, &mut live, &mut sink,
                );
                (s, ran)
            })
            .map_err(|e| format!("起成员线程失败：{}", e))?
            .join();
        let (s, ran) = match joined {
            Ok(x) => x,
            Err(_) => {
                self.call({
                    let name = child.clone();
                    move |core| {
                        core.abort_running(&name);
                        Ok(())
                    }
                })?;
                // 崩溃也要给子会话**收尾**：否则它的标签页永远停在"在跑"、流式层一直挂着。
                child_bus.push(&child, &[SessionEvent::Working { agent: None }]);
                child_bus.push(
                    &child,
                    &[SessionEvent::Notice(
                        crate::capabilities::session::api::interrupted_note("成员线程崩溃"),
                    )],
                );
                return Err("成员线程崩溃：该会话已按落盘转录保留".to_string());
            }
        };
        let turn = match ran {
            Ok(t) => t,
            Err(err) => {
                self.call({
                    let name = child.clone();
                    move |core| {
                        core.put_single(&name, s);
                        Ok(())
                    }
                })?;
                // 子会话也要**收尾**：失败/被停时没有定稿行，流式层必须撤下并如实说一句，
                // 否则那个标签页的光标一直挂着、按钮一直停在「停止」。
                let stopped = req.cancel.load(std::sync::atomic::Ordering::Relaxed);
                let why = if stopped {
                    crate::capabilities::session::api::stopped_note()
                } else {
                    crate::capabilities::session::api::interrupted_note(&err)
                };
                child_bus.push(&child, &[SessionEvent::Working { agent: None }]);
                child_bus.push(&child, &[SessionEvent::Notice(why)]);
                return Err(err);
            }
        };
        // 该回合的产出在跑的时候就已经落进**它自己的会话**并入了它自己的事件台（逐轮、逐条落盘）；
        // 这里只做运行态收尾：主会话有 working{None}，子会话也要有——否则那个标签页一直显示
        // "正在工作"、按钮一直停在「停止」。
        child_bus.push(
            &child,
            &[crate::capabilities::session::api::SessionEvent::Working { agent: None }],
        );
        self.call({
            let name = child.clone();
            move |core| {
                core.put_single(&name, s);
                Ok(())
            }
        })?;
        Ok(turn)
    }

    /// 手动压缩（`/compact`）：**让 AI 自己压**——核心只给 compact 工具，不是系统替它总结。
    /// 与单 agent 同一条 own-and-return：队列只占"取/交"，模型调用在工作线程上（界面不被阻塞）。
    /// 压不动就**如实说**（通知 + 继续用完整上下文），不静默降级、不假装压过。
    pub fn compact(&self, sid: &str) -> Result<Advance, String> {
        let bus = Arc::clone(&self.bus);
        // 计划必须在**取走会话之前**读：compact_plan 要现读会话的身份块与压缩点。
        let (prompt, decl, up_to, identity) = self.call({
            let sid = sid.to_string();
            move |core| core.compact_plan(&sid)
        })?;
        let session = self.call({
            let sid = sid.to_string();
            move |core| core.take_single(&sid)
        })?;
        let joined = std::thread::Builder::new()
            .name("solomni-compact".to_string())
            .spawn(move || {
                let mut s = session;
                let made = crate::capabilities::collab::api::compact_turn(
                    &mut s,
                    &prompt,
                    decl.as_ref(),
                    &identity,
                );
                if let Ok(summary) = &made {
                    s.compact(up_to, summary);
                }
                (s, made)
            })
            .map_err(|e| format!("起压缩线程失败：{}", e))?
            .join();
        let (s, made) = match joined {
            Ok(x) => x,
            Err(_) => {
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.abort_running(&sid);
                        Ok(())
                    }
                })?;
                return Err("压缩线程崩溃：会话已按落盘转录保留".to_string());
            }
        };
        let summary = match made {
            Ok(sm) => sm,
            Err(err) => {
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.put_single(&sid, s);
                        Ok(())
                    }
                })?;
                let note =
                    SessionEvent::Notice(crate::capabilities::session::api::interrupted_note(
                        &format!("压缩没成功：{}", err),
                    ));
                let head = bus.push(sid, std::slice::from_ref(&note));
                return Ok(Advance { head });
            }
        };
        let ev = SessionEvent::Compacted { up_to, summary };
        self.call({
            let sid = sid.to_string();
            let ev = ev.clone();
            move |core| {
                core.put_single(&sid, s);
                core.persister(&sid).persist(std::slice::from_ref(&ev));
                Ok(())
            }
        })?;
        let head = bus.push(sid, std::slice::from_ref(&ev));
        Ok(Advance { head })
    }

    /// 起一轮**脱离调用方**的节点执行：核心注入任务 + 不等它跑完。
    /// 完成后由叫醒逻辑推进父会话——所以这里只是"点火"。
    pub(crate) fn spawn_detached_node(&self, sid: &str, objective: &str) {
        // 节点执行也用整场工作的同一套回合计数（回档同步靠两边同一套编号）。
        let _ = self.call({
            let sid = sid.to_string();
            move |core| {
                core.bump_turn_of_child(&sid);
                Ok(())
            }
        });
        let me = self.clone();
        let (sid, objective) = (sid.to_string(), objective.to_string());
        let _ = std::thread::Builder::new()
            .name("solomni-node".to_string())
            .spawn(move || {
                let _ = me.node_generation(&sid, &objective);
            });
    }

    /// 把**已经落盘**的一批事实推到某个会话的事件台（不重复落盘）：代理建子工作的开场事实走这条。
    pub(crate) fn publish(&self, sid: &str, events: Vec<SessionEvent>) {
        if events.is_empty() {
            return;
        }
        self.bus.push(sid, &events);
    }

    /// 机制往一个会话记一条**可回放通知**：落盘（它进历史重放）+ 推它的事件台（在场的前端立刻看到）。
    /// 代理转达的来源记录与控制记录走这条——两条都要"既留得下、也看得见"。
    pub(crate) fn record_notice(&self, sid: &str, ev: SessionEvent) {
        let target = sid.to_string();
        let mut batch = vec![ev.clone()];
        let ok = self
            .call(move |core| {
                core.record_events(&target, &mut batch);
                Ok(())
            })
            .is_ok();
        if ok {
            self.bus.push(sid, std::slice::from_ref(&ev));
        }
    }

    /// 起一轮**脱离调用方**的"继续"（恢复被暂停的单 agent 会话用）：不等它跑完。
    /// 与"派活"的区别：不注入新任务，接着上一次的断点走。
    pub(crate) fn spawn_detached_continue(&self, sid: &str) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-resume".to_string())
            .spawn(move || {
                let _ = me.single_generation(&sid, None, Output::Stream);
            });
    }

    /// 起一次**脱离调用方**的代理回合（子会话停下后叫醒代理用）：不等它跑完。
    /// 代理那边是一轮普通成员会话生成；它自己决定继续观察、追问、返工还是收敛。
    pub(crate) fn spawn_detached_proxy(&self, sid: &str) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-proxy".to_string())
            .spawn(move || {
                let _ = me.single_generation(&sid, None, Output::Stream);
            });
    }
    /// 起一次**脱离调用方**的协作阶段步（代理把消息转达到协作子会话用）：不等它跑完。
    /// 与“叫醒父会话”的区别：这条带一个明确的阶段步（开工 / 代答），不是从断点继续。
    pub(crate) fn spawn_detached_collab_step(&self, sid: &str, step: CollabStep, text: &str) {
        let me = self.clone();
        let (sid, text) = (sid.to_string(), text.to_string());
        let _ = std::thread::Builder::new()
            .name("solomni-proxy-collab".to_string())
            .spawn(move || {
                let _ = me.collab_generation(&sid, CollabWork::Step(step), &text);
            });
    }
    /// 起一次**脱离调用方**的协作推进（叫醒父会话用）：不等它跑完。
    pub(crate) fn spawn_detached_collab(&self, sid: &str) {
        let me = self.clone();
        let sid = sid.to_string();
        let _ = std::thread::Builder::new()
            .name("solomni-chain".to_string())
            .spawn(move || {
                let _ = me.collab_generation(&sid, CollabWork::Resume, "");
            });
    }

    /// 协作的长步骤（开始讨论 / 回答 / 继续）：与单 agent 同一条 own-and-return——
    /// 队列只占"取/交"两步，泵在工作线程上跑；事件**边产边送**事件台，界面因此能看着讨论推进。
    ///
    /// **核心驱动**（见 docs/session/session-model.md 二之二）：泵只决定"该问谁"，
    /// 成员回合由主线程取该 agent 的会话去跑（它才拿得到那些会话）。所以泵线程与主线程**握手**：
    /// 泵让出 → 发 AskReq → 主线程跑完回 MemberTurn → 泵继续。
    pub(crate) fn collab_generation(
        &self,
        sid: &str,
        work: CollabWork,
        text: &str,
    ) -> Result<Advance, String> {
        let bus = Arc::clone(&self.bus);
        let jobs = Arc::clone(&self.jobs);
        // **先登记再取会话**：登记早于派发，所以「停止」从派发那一刻起就能生效。
        let cancel = jobs.register(sid);
        let session = match self.call({
            let sid = sid.to_string();
            move |core| core.take_collab(&sid)
        }) {
            Ok(s) => s,
            Err(e) => {
                jobs.unregister(sid);
                return Err(e);
            }
        };
        // 把「停止」接到泵上：它在每次模型调用前与**调用中途**都看这个标志。
        let mut session = session;
        session.set_cancel(Arc::clone(&cancel));
        // 增量落盘手柄：泵产出一条定稿事件就落一条，中途刷新页面因此能看到已产生的部分。
        let persister = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.persister(&sid))
        })?;
        let text = text.to_string();
        // 提醒上限由设置来（用户可调，见 session-model.md 二）：起线程前问一次核心。
        // 调用次数**没有上限**：模型继续核实就继续跑，直到它给出表态（或用户点停止）。
        let remind_cap = self.call(|core| Ok(core.discuss_remind_cap())).unwrap_or(3);
        let handle = self.clone();
        // 握手通道：泵 → 主线程（要一个成员回合）；主线程 → 泵（回合结果）。
        let (ask_tx, ask_rx) = std::sync::mpsc::channel::<AskReq>();
        let (turn_tx, turn_rx) = std::sync::mpsc::channel::<
            Result<crate::capabilities::collab::api::MemberTurn, String>,
        >();
        let worker = {
            let sid = sid.to_string();
            let bus = Arc::clone(&bus);
            std::thread::Builder::new()
                .name("solomni-collab".to_string())
                .spawn(move || {
                    let mut c = session;
                    let mut seq = 0u64;
                    // 安全网计数（见循环尾）：提醒/重问必须有终点，不能让泵空转。
                    let mut guard = 0usize;
                    {
                        // 边产边送 + 边落盘：长流程里用户能看着讨论一轮轮推进，
                        // 中途刷新页面也能看到已产生的部分（不再等整段结束才一次性出现）。
                        let mut sink = |ev: SessionEvent| {
                            seq = bus.push(&sid, std::slice::from_ref(&ev));
                            // 落盘失败要**如实告知**（落一条警告进事件台），不静默丢历史。
                            if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                                bus.push(&sid, std::slice::from_ref(&SessionEvent::Notice(warn)));
                            }
                        };
                        match work {
                            CollabWork::Step(CollabStep::Begin) => {
                                c.begin(text.contains("allow"), &mut sink)
                            }
                            // 提请裁决 / 方案过审 / 节点放行都走这条（自由文本 + 核心判定）。
                            CollabWork::Step(CollabStep::Decide) => c.decide(&text, &mut sink),
                            CollabWork::Step(_) => {}
                            CollabWork::Resume => c.resume(&mut sink),
                        }
                        // 核心驱动：泵让出"该问谁"就回头找主线程（它才拿得到各 agent 的会话）。
                        loop {
                            // 先看有没有已经让出的那一步；没有就推一步（推完再看一次）。
                            let ask = match c.take_ask() {
                                Some(a) => Some(a),
                                None => {
                                    c.pump_with(&mut sink);
                                    c.take_ask()
                                }
                            };
                            let Some((i, identity, turn)) = ask else { break };
                            let Some(agent) = c.member_id(i) else { break };
                            // 提醒时要往它自己的会话里写（那边用的是同一个名字）。
                            let agent_name = agent.clone();
                            // 主会话据此显示"某某正在工作"（按钮切换与占位动画都读它）。
                            bus.push(
                                &sid,
                                &[crate::capabilities::session::api::SessionEvent::Working {
                                    agent: Some(agent_name.clone()),
                                }],
                            );
                            // **它自己的标签页同样要进"在跑"**：不然打开它只会看到上一次的运行态
                            // （或者按钮一直停在「停止」）。收尾那条在成员回合的收尾处推。
                            bus.push(
                                &format!("{}--{}", sid, agent_name),
                                &[crate::capabilities::session::api::SessionEvent::Working {
                                    agent: Some(agent_name.clone()),
                                }],
                            );
                            let req = AskReq {
                                agent,
                                identity,
                                turn,
                                systools: c.systools(),
                                cancel: c.disc_cancel(),
                                opts: c.disc_opts(),
                                round: c.round(),
                                turn_id: c.next_turn_id(),
                            };
                            let turn_id = req.turn_id;
                            if ask_tx.send(req).is_err() {
                                break;
                            }
                            // 等主线程跑完这一回合（它取会话、跑模型、落盘，再把结果送回来）。
                            let Ok(res) = turn_rx.recv() else { break };
                            match res {
                                Ok(turn) => {
                                    // **核心只提醒、不强制**（见 session-model.md 二）：
                                    // 没表态时按计数决定"注入提醒后重问"还是"记未回应后放过"。
                                    let after = c.after_member_turn(
                                        i,
                                        turn.verb.is_some(),
                                        c.cancelled(),
                                        remind_cap,
                                    );
                                    match after {
                                        crate::capabilities::collab::api::AfterTurn::Done => {
                                            c.feed_with(i, turn, turn_id, &mut sink)
                                        }
                                        crate::capabilities::collab::api::AfterTurn::Remind => {
                                            // 提醒进**它自己的会话**（系统消息）；不 feed——泵重问同一个人。
                                            let text = c.reminder_text();
                                            let child =
                                                format!("{}--{}", sid, agent_name);
                                            let target = child.clone();
                                            if let Ok(evs) = handle.call(move |core| {
                                                core.note_system(&child, &text)
                                            }) {
                                                // 提醒属于**它自己的会话**：按子会话的 sid 外送，
                                                // 不能混进主会话的事件流（否则主会话会冒出系统行）。
                                                for e in evs {
                                                    bus.push(&target, std::slice::from_ref(&e));
                                                }
                                            }
                                        }
                                        crate::capabilities::collab::api::AfterTurn::Unanswered => {
                                            c.pass_over(i, &mut sink)
                                        }
                                    }
                                    // 安全网：提醒/重问必须有终点（计数有上限，这里再兜一层）。
                                    guard += 1;
                                    if guard > 500 {
                                        sink(SessionEvent::Notice(
                                            "[警告] 讨论推进次数异常（已到安全上限），已停下等用户处理"
                                                .to_string(),
                                        ));
                                        break;
                                    }
                                }
                                Err(err) => {
                                    // 如实交回（由泵统一外送中断通知），不再往下推。
                                    c.note_turn_failure(err);
                                    c.pump_with(&mut sink);
                                    break;
                                }
                            }
                        }
                        // 泵停了（问完了 / 出错 / 被中断）：主会话回到"空闲"。
                        // 下一次问谁时会再推一条带名字的，所以这里只需要收尾这一条。
                        bus.push(
                            &sid,
                            &[crate::capabilities::session::api::SessionEvent::Working { agent: None }],
                        );
                    }
                    (c, seq)
                })
                .map_err(|e| format!("起协作线程失败：{}", e))?
        };
        // 主线程驱动每个请求：取该 agent 的会话、跑这一回合、把结果发回泵。
        // 模型调用在成员线程上（own-and-return），核心队列只占"取/交"两步——界面因此不被阻塞。
        while let Ok(req) = ask_rx.recv() {
            match self.run_member_turn(sid, &req) {
                Ok(turn) => {
                    if turn_tx.send(Ok(turn)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    // 把失败交回泵（它统一外送中断通知），不再往下推。
                    let _ = turn_tx.send(Err(err));
                    break;
                }
            }
        }
        drop(turn_tx);
        let joined = worker.join();
        jobs.unregister(sid);
        let (c, mut seq) = match joined {
            Ok(x) => x,
            Err(_) => {
                // 线程崩了：会话对象没了，但转录在盘上——解除"生成中"，下次访问按盘重建。
                self.call({
                    let sid = sid.to_string();
                    move |core| {
                        core.abort_running(&sid);
                        Ok(())
                    }
                })?;
                return Err("协作线程崩溃：会话已按落盘转录保留，可继续".to_string());
            }
        };
        // 交回核心：重新插入 + **为就绪节点派发子会话**（返回派发事件与待起生成的节点）。
        let (spawned, todo) = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.put_collab(&sid, c))
        })?;
        // 协作会话停下 / 交付时也通知代理父（顶层协作没有代理父：返回 None，什么都不做）。
        if let Some(proxy) = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.notify_proxy_of_child(&sid))
        })? {
            self.spawn_detached_proxy(&proxy);
        }
        // 派发事件（"[节点] 开工"等）**也要落盘**：它们是在这里产生的，不经过上面那条 sink——
        // 只推不落的话，刷新后回放会少掉"节点开工"那几行。
        let persister = self.call({
            let sid = sid.to_string();
            move |core| Ok(core.persister(&sid))
        })?;
        for ev in spawned {
            seq = bus.push(sid, std::slice::from_ref(&ev));
            if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                bus.push(sid, std::slice::from_ref(&SessionEvent::Notice(warn)));
            }
        }
        // 派发：每个就绪节点在**它自己的子会话**里起一轮生成（脱离本次调用，不等它跑完）。
        for (_node, child, objective) in todo {
            self.spawn_detached_node(&child, &objective);
        }
        Ok(Advance { head: seq })
    }
}
