//! 目的：单 agent 与节点的生成循环：取出会话、在工作线程上跑、交回核心。
//! 管：`run_prepared` 是两者的公共收尾与逐轮外送。
//! 不管：命令分发、协作泵、压缩。
//! 联动：`api/handle.rs`、`api/generation/collab.rs`、`service/`

use super::super::*;

use crate::capabilities::conductor::service::Prepared;
use crate::capabilities::session::api::Live;

impl ConductorHandle {
    /// 目的：单 agent 生成：**核心队列只占两步短命令**（取出会话 / 交回会话），生成本身在工作线程上跑。 为什么必须这样：生成要跑几十秒到几分钟，它若占着唯一的命令队列， 读接口（历史列表 / 状态）与其它会话的命令全排在它后面——界面因此"假死"。 不变量：会话状态只被一个线程碰——生成期间由工作线程独占，核心表里只留"生成中"这一态。
    pub(crate) fn single_generation(
        &self,
        sid: &str,
        text: Option<String>,
        out: Output,
    ) -> Result<Advance, String> {
        // 虚拟机档：生成前确保这棵树（或 proxy 子工作）的 guest 已经起来；本机档/已有活 guest = no-op。
        self.ensure_guest(sid)?;
        // ① 短命令：检查 + 把会话取出来 + 定下本次调用参数（都在核心线程上，毫秒级）。
        let prepared = self.call({
            let sid = sid.to_string();
            let t = text.clone();
            move |core| core.prepare_single(&sid, t.as_deref(), out == Output::Stream)
        })?;
        self.run_prepared(sid, prepared, text)
    }

    /// 目的：一个节点的执行回合：**核心把任务提示词作为系统消息注入**（不是用户发言），再生成。 与 CLI 的 `Conductor::drive_node` 同一条语义——各前端只做各自的界面，管道只有这一条。
    pub(crate) fn node_generation(&self, child: &str, objective: &str) -> Result<Advance, String> {
        self.ensure_guest(child)?;
        let prepared = self.call({
            let (child, objective) = (child.to_string(), objective.to_string());
            move |core| core.prepare_node(&child, &objective)
        })?;
        self.run_prepared(child, prepared, None)
    }

    /// 目的：生成的工作线程与收尾：用户发言、**核心注入的任务**、继续都走这一条。
    pub(crate) fn run_prepared(
        &self,
        sid: &str,
        prepared: Prepared,
        text: Option<String>,
    ) -> Result<Advance, String> {
        let bus = Arc::clone(&self.bus);
        let jobs = Arc::clone(&self.jobs);
        let (session, identity, prefix, task, llm, persister) = match prepared {
            // 不用跑模型（例如"末条是 AI 发言"）：提示本身也是事实，进事件台；回包只给头部。
            Prepared::Immediate(events) => {
                let head = bus.push(sid, &events);
                return Ok(Advance { head });
            }
            // 协作会话的「继续」同样 own-and-return：泵在它自己的工作线程上跑（见 collab_generation）。
            Prepared::NotSingle => {
                if text.is_some() {
                    return Err("该会话不是单 agent 模式".to_string());
                }
                // 协作会话的"继续"：走同一条 own-and-return（泵在工作线程上）。
                return self.collab_generation(sid, CollabWork::Resume);
            }
            Prepared::Run {
                session,
                identity,
                prefix,
                task,
                llm,
                persister,
            } => (*session, identity, prefix, task, llm, persister),
        };
        let mut session = session;
        // 代理会话：把代理工具的成员侧执行面装进这一回合（经队列桥回核心线程执行）。
        self.inject_proxy_handler(sid, &mut session)?;
        let session = session;
        // 取消标志在**派发时**就登记：生成一开始「停止」就能生效（它不进命令队列）。
        let cancel = jobs.register(sid);
        // 收尾时按这面旗的身份摘登记（同名的新生成可能已经登记了它自己那面）。
        let job_flag = Arc::clone(&cancel);
        // **运行态**：这条会话开始干活，推给它自己的事件台——节点执行、单 agent 发言、继续都走这里，
        // 打开它的标签页要立刻看到占位与「停止」按钮，而不是等 3 秒的状态轮询。事实只进事件台；命令回包只给头部序号（谁要看谁自己订阅）。
        let start_working = SessionEvent::Working {
            agent: Some(session.params().agent.clone()),
        };
        bus.push(sid, std::slice::from_ref(&start_working));
        // ② 工作线程：跑生成。短暂事件（流式增量 / 工具行）直送事件台——它是独立锁，不进核心队列。
        let decisions = self.decisions_of(sid)?;
        // 工具层的提问端口：与工具级确认**同一条通道**（同一份裁决队）——没有交互前端就没有它。
        let ask = decisions
            .as_ref()
            .map(|door| self.session_ask(door, sid, Arc::clone(&bus), persister.clone()));
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
                        decisions,
                        ask,
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
                    match (task.as_deref(), text.as_deref()) {
                        // 节点执行：工作线程先压缩本轮之前的上下文，再注入任务。
                        (Some(t), _) => crate::capabilities::collab::api::dispatch_task(
                            &mut session,
                            t,
                            &identity,
                            &mut live,
                            &mut sink,
                        ),
                        (None, Some(t)) => crate::capabilities::collab::api::say(
                            &mut session,
                            t,
                            &identity,
                            &mut live,
                            &mut sink,
                        ),
                        (None, None) => crate::capabilities::collab::api::continue_reply(
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
        jobs.unregister(sid, &job_flag);
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
        // 回包只给头部序号——想看的端自己按 since 订阅，不在命令里捎带事实。交回核心只做"重新插入"：转录也已逐轮增量落盘。
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
}
