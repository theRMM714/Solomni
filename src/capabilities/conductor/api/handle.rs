//! 目的：核心手柄——把 Conductor 搬到它自己的执行线程，命令与事件都经这里。
//! 管：命令队列的「取 / 交」两步与长步骤在核心线程上的执行；事件台的订阅出口；代理会话的工具注入。
//! 不管：长步骤本身的业务逻辑（在 service）；停止 / 取消（由 kernel::api::JobRegistry 承担、不进队列）。
//! 联动：呈现层与核心共用 api 的 Ops 契约（docs/presentation/contracts.md）；事件台由核心独占生产。

use super::*;
use crate::capabilities::conductor::service::Conductor;
pub use crate::capabilities::session::api::SessionEvent;
use crate::kernel::api::JobRegistry;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{self};
impl ConductorHandle {
    /// 目的：把核心搬到它自己的执行线程：此后所有核心状态只被这一个线程碰。 全部手柄丢弃后线程自然结束（通道断开即退出）。
    pub fn spawn(core: Conductor) -> Result<ConductorHandle, String> {
        let worker_log = core.log_handle();
        let jobs = JobRegistry::new();
        let interactive = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let bus = EventBus::new();
        let (tx, rx) = mpsc::channel::<Job>();
        let book = core.systools_book();
        let tools = core.tools_handle();
        let residents = core.residents_handle();
        let secrets = core.secrets_handle();
        let modules = core.modules_handle();
        let builds = core.builds_handle();
        let handle = ConductorHandle {
            tx,
            jobs,
            interactive,
            bus,
            log: Arc::clone(&worker_log),
            book,
            tools,
            residents,
            secrets,
            modules,
            builds,
            booting: Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new())),
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

    /// 目的：发一条命令并等回复：呈现层因此仍是「调用即拿结果」，不需要改成异步。
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

    /// 目的：创建会话前的补编——本次名单里的模块若在目标环境没有入口、且声明了 `build:`，先在这里编好。
    /// 参数：`ids` = 本次会话的模块 id（已去重保序）；`tier` / `guest` = 目标环境。
    /// 错误：构建失败或环境不齐时如实拒绝（错误里带目标与构建输出）。
    /// 约束：跑在**调用方线程**（不占核心队列），核心线程不陪着等编译器；入口已在场的模块不碰。
    pub(crate) fn ensure_built(
        &self,
        ids: &[String],
        tier: crate::kernel::api::Tier,
        guest: Option<&str>,
    ) -> Result<(), String> {
        use crate::capabilities::builds::api::Target;
        if ids.is_empty() {
            return Ok(());
        }
        let target = match tier {
            crate::kernel::api::Tier::Host => Target::Host,
            crate::kernel::api::Tier::Vm => match guest.map(str::trim).filter(|g| !g.is_empty()) {
                Some(g) => Target::Guest(g.to_string()),
                // 虚拟根没给：准备段会如实拒绝（这里不猜默认值）。
                None => return Ok(()),
            },
        };
        let roster = self.modules.roster();
        let net = self.call(|core| Ok(core.network_allowed()))?;
        // 一批一起交给构建能力：虚拟机档只起一台 guest、一块缓存卷，不按模块反复起停。
        let modules: Vec<&crate::capabilities::modules::api::Module> = ids
            .iter()
            .filter_map(|id| roster.modules.iter().find(|m| &m.manifest.id == id))
            .collect();
        if modules.is_empty() {
            return Ok(());
        }
        let built = self
            .builds
            .ensure_many(&modules, &target, net)
            // 单模块时指名模块；多个模块时内层错误已经点出是哪一个。
            .map_err(|e| match modules.as_slice() {
                [one] => format!(
                    "模块 {} 构建失败（目标 {}）：{}",
                    one.manifest.id,
                    target.label(),
                    e
                ),
                _ => format!("构建失败（目标 {}）：{}", target.label(), e),
            })?;
        for b in built.iter().filter(|b| !b.skipped) {
            self.log.info(
                "conductor::create_session",
                &format!("已补编模块 {}（目标 {}）", b.module, b.target),
            );
        }
        Ok(())
    }

    /// 目的：接上工具级确认：有交互前端（Web）在服务时才调，纯终端不调（生成线程不能空等一个没人回答的问题）。
    pub fn allow_tool_cards(&self) {
        self.interactive
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// 目的：这个会话的**工具层提问端口**：一张卡的登记 + 外送/落盘 + 停会话，全走同一个会话的那条道。
    /// 参数：`door` = 该会话的裁决队（与核心各关卡共用）；`bus` / `persister` = 卡的外送与落盘。
    pub(super) fn session_ask(
        &self,
        door: &Arc<crate::capabilities::session::api::DecisionDoor>,
        sid: &str,
        bus: Arc<EventBus>,
        persister: crate::capabilities::conductor::service::Persister,
    ) -> Arc<dyn crate::kernel::ports::AskUser> {
        let owner = sid.to_string();
        let emit: Box<dyn Fn(SessionEvent) + Send + Sync> = Box::new(move |ev: SessionEvent| {
            bus.push(&owner, std::slice::from_ref(&ev));
            if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                bus.push(&owner, std::slice::from_ref(&SessionEvent::Notice(warn)));
            }
        });
        Arc::new(
            crate::capabilities::conductor::service::ask_user::SessionAsk::new(
                Arc::clone(door),
                sid,
                emit,
                self.clone(),
            ),
        )
    }

    /// 目的：这个会话的**裁决队**（工具级确认进它）：只有接了交互前端才给——否则这一趟不接确认。
    /// 约束：在**派发之前**取（生成线程一开始就可能要问），取的是同一份句柄。
    pub(super) fn decisions_of(
        &self,
        sid: &str,
    ) -> Result<Option<Arc<crate::capabilities::session::api::DecisionDoor>>, String> {
        if !self.interactive.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(None);
        }
        let sid = sid.to_string();
        self.call(move |core| Ok(core.desk_of(&sid))).map(Some)
    }

    /// 目的：本核心的事件台（多端订阅）。
    pub fn events(&self) -> Arc<EventBus> {
        Arc::clone(&self.bus)
    }

    /// 目的：回档并推重置快照（`SessionOps` 与动作表共用这一条收尾）。
    /// 约束：回档会改写盘上转录，事件台必须同时收到 `rewound` 才能让刷新/多端重置；两个入口各写一份，迟早一处漏推。
    pub(crate) fn rewind_with_reset(
        &self,
        sid: &str,
        target: RewindTarget,
        force: bool,
    ) -> Result<RewindOutcome, String> {
        // 三种回档都改写转录，删除还一起回退工作区文件：在跑的回合先停稳，
        // 否则它收尾时的写会落到已被改写的旧线或刚铺好的树上。
        if matches!(
            target,
            RewindTarget::Archive(_) | RewindTarget::Restore(_) | RewindTarget::Delete(_)
        ) {
            self.settle_tree(sid)?;
        }
        let outcome = self.call({
            let sid = sid.to_string();
            move |core| core.rewind_outcome(&sid, target, force)
        })?;
        self.publish_rewound(sid, &outcome);
        Ok(outcome)
    }

    /// 目的：留档 / 删除 / 恢复前把整棵子树在跑的回合停下来，并等它们把会话交回核心。
    /// 返回：实际停下的会话 id。
    /// 约束：等待期间**不占核心队列**——收尾要靠核心交回会话；30 秒仍未收尾 = 拒绝，文件与转录都还没动。
    fn settle_tree(&self, root: &str) -> Result<Vec<String>, String> {
        let root_owned = root.to_string();
        let tree: Vec<String> = self.call(move |core| Ok(core.subtree_of(&root_owned)))?;
        if !tree.iter().any(|s| self.jobs.is_running(s)) {
            return Ok(Vec::new());
        }
        // 等用户裁决的卡先作废（停止 = 拒绝），生成线程才不会卡在确认上。
        self.void_gates(root);
        let stopped = self.stop_tree(root)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let jobs_busy = tree.iter().any(|s| self.jobs.is_running(s));
            let ids = tree.clone();
            let core_busy = self.call(move |core| Ok(core.running_any(&ids)))?;
            if !jobs_busy && !core_busy {
                return Ok(stopped);
            }
            if std::time::Instant::now() >= deadline {
                return Err(
                    "整棵子树里有回合 30 秒内没停下来：这次操作没有动文件与转录，请稍后重试"
                        .to_string(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// 目的：把一次回档结果发布成事件台上的重置快照（受影响的每个会话各一条 `rewound`）。
    /// 约束：主/子会话都要发——子会话也有自己的历史视图与刷新路径。
    pub(crate) fn publish_rewound(&self, sid: &str, outcome: &RewindOutcome) {
        for (child, events) in &outcome.children {
            self.bus.push_rewound(child, events.clone());
        }
        self.bus.push_rewound(sid, outcome.main.clone());
    }

    /// 目的：**级联停止**：把 root 及其整棵子树（按 `meta.parent`）里正在生成的会话都停下来。 返回实际停下的会话 id——不假装“停止了一个本来就没在跑的会话”。 代理核心的 `stop` 走这条：主会话停 = 所有相关工作一起停（用户看得见的那一下）。
    pub(crate) fn stop_tree(&self, root: &str) -> Result<Vec<String>, String> {
        let root_owned = root.to_string();
        let tree = self.call(move |core| Ok(core.subtree_of(&root_owned)))?;
        let mut stopped = Vec::new();
        for sid in tree {
            // 在等的工具确认由「停止 = 整队作废」解开（见 SessionOps::stop），这里只管取消生成。
            if self.jobs.stop(&sid) {
                stopped.push(sid);
            }
        }
        Ok(stopped)
    }

    /// 目的：把 root 及其整棵子树里等用户裁决的队**一律作废**（停止 = 拒绝），并落盘 + 外送。
    /// 约束：作废是会话的事实——与回答同一条路（落进转录），重启后按它重建，作废过的卡不再挂起。
    pub(crate) fn void_gates(&self, root: &str) {
        let root_owned = root.to_string();
        let Ok(voids) = self.call(move |core| Ok(core.void_gates_in_subtree(&root_owned))) else {
            return;
        };
        for (sid, events) in voids {
            let warn = self
                .call({
                    let sid = sid.clone();
                    let events = events.clone();
                    move |core| Ok(core.persister(&sid).persist(&events))
                })
                .ok()
                .flatten();
            self.bus.push(&sid, &events);
            if let Some(w) = warn {
                self.bus
                    .push(&sid, std::slice::from_ref(&SessionEvent::Notice(w)));
            }
        }
    }

    /// 目的：代理会话——把代理工具的成员侧执行面（`ProxyHandler`）装进这一回合的工具环境。
    ///   执行经队列桥回到核心线程，核心状态的所有权不变；非代理会话、或已装过，什么都不做。
    pub(super) fn inject_proxy_handler(
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
            dp::ProxyCall {
                grant,
                parent: Some(sid.to_string()),
                now,
            },
        );
        tools.handlers = vec![std::sync::Arc::new(handler)];
        Ok(())
    }
    /// 目的：测试专用：注入一条必定 panic 的命令，验证「一条命令 panic 不带垮整个核心」。
    #[cfg(test)]
    pub(crate) fn panic_probe(&self) -> Result<(), String> {
        self.call(|_core| -> Result<(), String> { panic!("入站契约测试注入的 panic") })
    }

    /// 目的：串行同一台 guest 的启动；离开作用域自动摘掉标记（即便启动失败/ panic）。
    fn run_guest_boot(
        &self,
        boot: crate::capabilities::conductor::service::guest_job::GuestBoot,
    ) -> Result<(), String> {
        struct Slot<'a> {
            set: &'a std::sync::Mutex<std::collections::BTreeSet<String>>,
            lease: String,
        }
        impl Drop for Slot<'_> {
            fn drop(&mut self) {
                if let Ok(mut set) = self.set.lock() {
                    set.remove(&self.lease);
                }
            }
        }
        let lease = boot.lease().to_string();
        {
            let mut set = self
                .booting
                .lock()
                .map_err(|_| "guest 启动锁坏了".to_string())?;
            if !set.insert(lease.clone()) {
                return Err(format!("guest 正在启动（租约 {}）：稍后再试", lease));
            }
        }
        let _slot = Slot {
            set: &self.booting,
            lease,
        };
        boot.run()
    }

    /// 目的：确保这条会话的 guest 已经起来（启用 / 发言 / 成员回合 / 节点共用的兜底）。
    /// 约束：只准备与启动，不改会话运行态；已有活 guest 或本机档 = 什么都不做。
    pub(crate) fn ensure_guest(&self, sid: &str) -> Result<(), String> {
        let sid = sid.to_string();
        let boot = self.call(move |core| core.prepare_guest_boot(&sid))?;
        if let Some(boot) = boot {
            self.run_guest_boot(boot)?;
        }
        Ok(())
    }

    /// 目的：启用一条历史会话（只恢复，不生成）：重建、确保 guest、解冻整棵子树。
    pub(crate) fn activate_session(&self, sid: &str) -> Result<SessionView, String> {
        let sid = sid.to_string();
        // 启用历史会话也要先确保目标环境的模块产物：老会话可能是在宿主上建的，
        // 切到/重开 VM 档时不能把宿主 native 产物直接带进 guest。
        let (ids, tier, guest) = self.call({
            let sid = sid.clone();
            move |core| core.activation_build_request(&sid)
        })?;
        self.ensure_built(&ids, tier, guest.as_deref())?;
        let prepared = self.call(move |core| core.prepare_activation(&sid))?;
        let prepared = match prepared {
            crate::capabilities::conductor::service::PreparedActivation::Ready(ready) => {
                let crate::capabilities::conductor::service::ReadyActivation { sid, boot, lease } =
                    *ready;
                if let Some(boot) = boot {
                    self.run_guest_boot(boot)?;
                }
                crate::capabilities::conductor::service::PreparedActivation::Ready(Box::new(
                    crate::capabilities::conductor::service::ReadyActivation {
                        sid,
                        boot: None,
                        lease,
                    },
                ))
            }
            other => other,
        };
        self.call(move |core| core.commit_activation(prepared))
    }

    pub(crate) fn publish(&self, sid: &str, events: Vec<SessionEvent>) {
        if events.is_empty() {
            return;
        }
        self.bus.push(sid, &events);
    }

    /// 目的：机制往一个会话记一条**内容类提示**：能行化就行化（有 id、可回档），否则退回瞬时提示； 落盘（它进历史重放）+ 推它的事件台（在场的前端立刻看到）——代理转达与控制记录走这条。
    pub(crate) fn record_notice(&self, sid: &str, ev: SessionEvent) {
        let SessionEvent::Notice(text) = ev else {
            return;
        };
        let target = sid.to_string();
        if let Ok(events) = self.call(move |core| {
            let mut events = core.notice_row(&target, &text);
            core.record_events(&target, &mut events);
            Ok(events)
        }) {
            self.bus.push(sid, &events);
        }
    }
}
