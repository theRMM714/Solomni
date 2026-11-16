//! 目的：运行时的核心手柄（Handle）——命令队列这一侧的唯一入口：呈现层经 entry/faces 拿它拆面。
//! 管：起线程（spawn）、投递命令（call）、事件台发布与提示、交互前端开关，以及回合宿主端口 TurnHost 的实现。
//! 不管：长步骤编排（在 activation / rewind）、动作分发（在 action）、代理入站桥（在 bridge）。
//! 联动：kernel::api::Actor 是机制壳；回合引擎在下层 turn，它要的宿主能力由本文件实现。

use super::*;

/// 目的：核心手柄：呈现层用它发命令。克隆廉价（内部只是通道 + 两个共享句柄）。
#[derive(Clone)]
pub struct Handle {
    /// 目的：命令队列：核心状态只在它自己那条线程上被执行（机制壳在 `src/kernel/domain/actor.rs`）。
    pub(super) actor: crate::kernel::api::Actor<Core>,
    pub(super) jobs: Arc<JobRegistry>,
    /// 目的：有没有"能作答"的交互前端（Web 在服务时打开）：打开才把工具级确认接进裁决队。
    ///  纯终端同步生成不打开，避免生成线程空等一个没人回答的问题。
    pub(super) interactive: Arc<std::sync::atomic::AtomicBool>,
    pub(super) bus: Arc<dyn Events>,
    /// 目的：日志句柄：呈现层经 LogOps 能力写日志，拿不到这个端口对象本身。
    pub(super) log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 目的：代理会话注入成员侧执行者要的两份装配材料（与核心共享同一份，不在桥这一侧重装）。
    pub(super) book: crate::capabilities::tools::api::ToolBook,
    /// 目的：工具执行面："人直接跑一个模块工具"（无会话）也走它，不另起一套执行机制。
    pub(super) tools: Arc<dyn crate::capabilities::tools::api::ToolExec + Send + Sync>,
    /// 目的：常驻服务面（与核心共享同一份）：ResidentOps 由它转发。
    pub(crate) residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
    /// 目的：隐秘字段面（与核心共享同一份）：SecretOps 由它转发。
    pub(crate) secrets: Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync>,
    /// 目的：模块公地能力面（与核心共享同一份）：模块事实与启停写面由它转发。
    pub(super) modules: Arc<dyn crate::capabilities::modules::api::Modules + Send + Sync>,
    /// 目的：构建能力面（与核心共享同一份）：创建会话前在**调用方线程**补编模块产物。
    pub(super) builds: Arc<dyn crate::capabilities::builds::api::Builds + Send + Sync>,
    /// 目的：正在启动的 guest 租约：同一台 guest 的并发启用/生成只能有一个在拉，别互相替换。
    pub(super) booting: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
}

impl Handle {
    /// 目的：把核心搬到它自己的执行线程：此后所有核心状态只被这一个线程碰。 全部手柄丢弃后线程自然结束（通道断开即退出）。
    pub fn spawn(core: Core) -> Result<Handle, String> {
        let worker_log = core.log_handle();
        let jobs = JobRegistry::new();
        let interactive = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let bus = crate::capabilities::turn::api::event_bus();
        let book = core.systools_handle().book();
        let tools = core.tools_handle();
        let residents = core.residents_handle();
        let secrets = core.secrets_handle();
        let modules = core.modules_handle();
        let builds = core.builds_handle();
        // 队列与线程是纯机制：起线程、逐条执行、接住 panic 都在 kernel 的壳里。
        let actor = crate::kernel::api::Actor::spawn(
            "solomni-core",
            "runtime::api",
            Arc::clone(&worker_log),
            core,
        )?;
        let handle = Handle {
            actor,
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
        Ok(handle)
    }

    /// 目的：发一条命令并等回复：呈现层因此仍是「调用即拿结果」，不需要改成异步。
    pub(crate) fn call<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Core) -> Result<T, String> + Send + 'static,
    {
        self.actor.call(f)
    }

    /// 目的：接上工具级确认：有交互前端（Web）在服务时才调，纯终端不调（生成线程不能空等一个没人回答的问题）。
    pub fn allow_tool_cards(&self) {
        self.interactive
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// 目的：本核心的事件台（多端订阅）。
    pub fn events(&self) -> Arc<dyn Events> {
        Arc::clone(&self.bus)
    }

    /// 目的：这个会话的**工具层提问端口**：一张卡的登记 + 外送/落盘 + 停会话，全走同一个会话的那条道。
    /// 参数：`door` = 该会话的裁决通道（与核心各关卡共用）；`bus` / `persister` = 卡的外送与落盘。
    pub(super) fn session_ask(
        &self,
        door: &Arc<dyn crate::capabilities::session::api::Gates>,
        sid: &str,
        bus: Arc<dyn Events>,
        persister: crate::capabilities::session::api::Persister,
    ) -> Arc<dyn crate::kernel::ports::AskUser> {
        let owner = sid.to_string();
        let emit: Box<dyn Fn(SessionEvent) + Send + Sync> = Box::new(move |ev: SessionEvent| {
            bus.push(&owner, std::slice::from_ref(&ev));
            if let Some(warn) = persister.persist(std::slice::from_ref(&ev)) {
                bus.push(&owner, std::slice::from_ref(&SessionEvent::Notice(warn)));
            }
        });
        // 停会话仍走命令队列手柄（它是唯一能级联停的那一层）：闭包持它，本层类型不外泄。
        let handle = Arc::new(std::sync::Mutex::new(self.clone()));
        let stop: Box<dyn Fn(&str) + Send + Sync> = Box::new(move |sid: &str| {
            let handle = handle.lock().unwrap_or_else(|e| e.into_inner());
            crate::capabilities::turn::api::TurnHost::stop(&*handle, sid);
        });
        crate::capabilities::turn::api::session_ask(Arc::clone(door), sid, emit, stop)
    }

    /// 目的：这个会话的**裁决队**（工具级确认进它）：只有接了交互前端才给——否则这一趟不接确认。
    /// 约束：在**派发之前**取（生成线程一开始就可能要问），取的是同一份句柄。
    pub(super) fn decisions_of(
        &self,
        sid: &str,
    ) -> Result<Option<Arc<dyn crate::capabilities::session::api::Gates>>, String> {
        if !self.interactive.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(None);
        }
        let sid = sid.to_string();
        self.call(move |core| Ok(core.turn.desk_of(&sid))).map(Some)
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
            let mut events = core.turn.notice_row(&target, &text);
            core.turn.record_events(&target, &mut events);
            Ok(events)
        }) {
            self.bus.push(sid, &events);
        }
    }

    /// 目的：测试专用：注入一条必定 panic 的命令，验证「一条命令 panic 不带垮整个核心」。
    #[cfg(test)]
    pub(crate) fn panic_probe(&self) -> Result<(), String> {
        self.call(|_core| -> Result<(), String> { panic!("入站契约测试注入的 panic") })
    }
}

impl crate::capabilities::turn::api::TurnHost for Handle {
    fn core<T>(
        &self,
        f: impl FnOnce(&mut crate::capabilities::turn::api::Turn) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String>
    where
        T: Send + 'static,
    {
        self.call(move |core| f(&mut core.turn))
    }

    fn core_env<T>(
        &self,
        f: impl FnOnce(
            &mut crate::capabilities::turn::api::Turn,
            &dyn crate::capabilities::registry::api::Registry,
            &dyn crate::kernel::ports::HostProbe,
        ) -> Result<T, String>
        + Send
        + 'static,
    ) -> Result<T, String>
    where
        T: Send + 'static,
    {
        // 登记处与宿主探针留在核心字段上（回档重建要它们与 &mut turn 分开借用）。
        self.call(move |core| f(&mut core.turn, core.registry.as_ref(), core.probe.as_ref()))
    }

    fn events(&self) -> Arc<dyn Events> {
        Arc::clone(&self.bus)
    }

    fn ensure_guest(&self, sid: &str) -> Result<(), String> {
        Handle::ensure_guest(self, sid)
    }

    fn stop(&self, sid: &str) {
        let _ = RuntimeOps::stop(self, sid);
    }

    fn decisions_of(
        &self,
        sid: &str,
    ) -> Result<Option<Arc<dyn crate::capabilities::session::api::Gates>>, String> {
        Handle::decisions_of(self, sid)
    }

    fn session_ask(
        &self,
        door: &Arc<dyn crate::capabilities::session::api::Gates>,
        sid: &str,
        bus: Arc<dyn Events>,
        persister: crate::capabilities::session::api::Persister,
    ) -> Arc<dyn crate::kernel::ports::AskUser> {
        Handle::session_ask(self, door, sid, bus, persister)
    }

    fn inject_proxy_handler(
        &self,
        session: crate::capabilities::session::api::AgentSession,
        sid: &str,
    ) -> Result<crate::capabilities::session::api::AgentSession, String> {
        Handle::inject_proxy_handler(self, session, sid)
    }

    fn register_job(&self, sid: &str) -> Arc<std::sync::atomic::AtomicBool> {
        self.jobs.register(sid)
    }

    fn unregister_job(&self, sid: &str, flag: &Arc<std::sync::atomic::AtomicBool>) {
        self.jobs.unregister(sid, flag)
    }

    fn answer_card(
        &self,
        sid: &str,
        card: &str,
        option: &str,
        note: &str,
    ) -> Result<crate::capabilities::turn::api::Advance, String> {
        TurnOps::answer_card(self, sid, card, option, note)
    }
}
