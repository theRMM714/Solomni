//! 目的：conductor 的核心层——端口抽象、编排业务（会话 / 引擎 / 会话中心）与队列桥。
//! 管：Conductor 门面与各 service 子模块的编排：会话在世表、命令队列、运行态、跨能力用例。
//! 不管：文件读、网络请求、stdin/stdout（机制在各能力的 detail 与 kernel/detail）；装配（new 适配器）只在 main 组合根。
//! 联动：api 层经 ConductorHandle 把命令与事件交给这里；代理宿主动作经 `proxy::api::ProxyHost`（本能力的 `ProxyBridge` 实现）。

use crate::capabilities::builds::api::Builds;
use crate::capabilities::conductor::api::RewindTarget;
use crate::capabilities::llm::api::Llm;
use crate::capabilities::packages::api::Packages;
use crate::capabilities::provision::api::Provision;
use crate::capabilities::session::api::History;
#[cfg(test)]
pub(crate) use crate::capabilities::session::api::Live;
use crate::capabilities::session::api::{Answered, GateTicket, SessionEvent};
use crate::capabilities::vm::api::Vm;
use crate::capabilities::work::api::Work;
use crate::capabilities::workspace::api::Workspace;

use crate::capabilities::collab::api::AfterTurn;
use crate::capabilities::llm::api::Channel;
#[cfg(test)]
use crate::capabilities::llm::api::Msg;
use crate::capabilities::modules::api::Modules;
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::registry::api::Registry;
use crate::capabilities::session::api::{AgentMeta, HistoryView, SessionMeta};
use crate::capabilities::tools::api::{ToolExec, Tools};
use crate::capabilities::turn::api::Session;
use compact::{CompactChoice, CompactRuntime};
use std::sync::{Arc, RwLock};

/// 目的：一次单 agent 生成的**准备结果**（核心线程上只算到这里，模型调用在工作线程上）。
pub(crate) enum Prepared {
    /// 可以跑：会话已从核心表取出，由工作线程独占。
    Run {
        /// 装箱：这个变体比其它两个大得多（会话本体），而它本来就是**一次性移交**给工作线程的。
        session: Box<crate::capabilities::session::api::AgentSession>,
        /// 本回合的**身份块**：按当前提示词册与登记处现渲染（不进会话的消息列表）。
        identity: String,
        /// 生成前要先给用户的事件（例如工具形态变更的提示）。
        prefix: Vec<SessionEvent>,
        /// 本回合要注入的**派发任务**（节点执行）：在工作线程上先压缩、再注入，任务不会被压掉。
        task: Option<String>,
        llm: crate::capabilities::llm::api::LlmOpts,
        /// 增量落盘手柄：逐轮外送的同时就落盘（中途刷新页面因此看得到已产生的部分）。
        persister: crate::capabilities::session::api::Persister,
    },
    /// 不用跑模型：直接把这批事件回给调用方（例如"末条是 AI 发言"）。
    Immediate(Vec<SessionEvent>),
    /// 不是单 agent 会话：交回调用方走它自己那条路（协作）。
    NotSingle,
}

/// 目的：**应用服务**：持有注入的端口、能力面与会话中心；前端拿不到它（只拿 `conductor::api::Ops`）。
///  组合根在 `ConductorHandle::spawn` 里把它**移进核心自己的执行线程**——此后状态只被那一个线程碰，
///  所以这里不加任何锁（并发不变式见 ARCHITECTURE.md §一）。
pub struct Conductor {
    /// 目的：登记处能力——**四份 yaml 的状态在它里面**，conductor 只按 `Registry` 调用（看不见它的字段）。
    pub(crate) registry: Box<dyn Registry>,
    /// 会话历史直连面（**不持它的端口**，R12）：造/读/写/删会话都走它。
    history: Arc<dyn History + Send + Sync>,
    /// 工作区用例面（**不持它的端口**，R12）：工作区目录与版本化走它。
    workspace: Arc<dyn Workspace + Send + Sync>,
    /// 虚拟机档用例面（**不持它的端口**，R12）：guest 生命周期与档位就绪检测走它。
    vm: Arc<dyn Vm + Send + Sync>,
    /// 目的：模块能力（**不持它的端口**，R12）——模块清单事实的唯一上游。
    modules: Arc<dyn Modules + Send + Sync>,
    /// 目的：构建能力面（**不持它的端口**，R12）——创建会话前补编声明了 `build:` 的模块。
    builds: Arc<dyn Builds + Send + Sync>,
    /// 压缩通道工厂：全局设置只在这里对账；会话在压缩触发时现取（不刷新会话）。
    compact: Arc<CompactRuntime>,
    /// 工具执行面（**不持它的端口**，R12）：跑外部/内置工具、释放围栏授权都走它。
    tools: Arc<dyn ToolExec + Send + Sync>,
    log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 宿主能力探测（读环境、查路径存在性都在它后面；conductor 因此不碰 std::env 与文件系统）。
    probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
    /// 常驻服务的统一管理面（**不持它的端口**）：动作面与会话回收经它。
    residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
    /// 隐秘字段的统一管理面：工具 / 服务起进程时按它解析注入项并脱敏。
    secrets: Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync>,
    /// 目的：工作形态用例——建工作 / 配置 / 视图 / 报告 / 激活 / 动作都在这个能力里，核心只经它调用。
    ///   与 `turn` 同为**核心持有的唯一实例**（不共享、不加锁）。
    pub(crate) work: Work,
    /// 提示词册能力：**册子本体在它里面**（只有一处），conductor 只按名字取段。
    /// 用 `Arc`：协作会话要与核心**共享**这一份（逐处克隆整本册子是白费）。
    prompt: Arc<dyn Prompt>,
    /// 工具总表与角色表的能力面（授权两张表；名字表由组合根另取）：**表本体在工具能力里**，与册子互不依赖。
    systools: Arc<dyn Tools>,
    /// 目的：会话本体（在世表、装配、从落盘重建、树与裁决队）——`turn` 能力的唯一持有者；conductor 只经它读写会话。
    pub(crate) turn: crate::capabilities::turn::api::Turn,
}

pub(crate) mod ask_user;
mod compact;
mod env;
mod flow;
mod history;
pub mod proxy;
mod rewind;
mod turn;
pub(crate) mod work_tools;

impl Conductor {
    /// 组合根专用：main 负责创建适配器并注入；conductor 不自建任何具体实现。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: std::path::PathBuf,
        registry: Box<dyn Registry>,
        history: Arc<dyn History + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        packages: Arc<dyn Packages + Send + Sync>,
        provision: Arc<dyn Provision + Send + Sync>,
        modules: Arc<dyn Modules + Send + Sync>,
        builds: Arc<dyn Builds + Send + Sync>,
        llm: Arc<dyn Llm + Send + Sync>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        prompt: Arc<dyn Prompt>,
        systools: Arc<dyn Tools>,
        log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
        probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
        residents: Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync>,
        secrets: Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync>,
    ) -> Conductor {
        let compact = Arc::new(CompactRuntime::new(Arc::clone(&llm)));
        // 会话本体与核心共享同一批能力面与压缩选择；产品根两边各留一份（会话重建要它认虚拟根）。
        let turn = crate::capabilities::turn::api::Turn::new(
            root.clone(),
            Arc::clone(&history),
            Arc::clone(&workspace),
            Arc::clone(&vm),
            Arc::clone(&packages),
            Arc::clone(&modules),
            Arc::clone(&residents),
            Arc::clone(&secrets),
            Arc::clone(&tools),
            Arc::clone(&systools),
            Arc::clone(&prompt),
            Arc::clone(&llm),
            Arc::clone(&log),
            compact.clone(),
        );
        // 工作形态用例与核心共享同一批能力面；`handlers` 是核心自己的装配件，
        // 注入给它以免两个能力互相引用（能力之间只能经对方的 api）。
        let work = Work::new(
            root.clone(),
            Arc::clone(&history),
            Arc::clone(&workspace),
            Arc::clone(&vm),
            Arc::clone(&packages),
            Arc::clone(&modules),
            Arc::clone(&provision),
            Arc::clone(&llm),
            Arc::clone(&prompt),
            Arc::clone(&systools),
            Arc::clone(&tools),
            Arc::clone(&log),
            Arc::clone(&probe),
            Box::new(crate::capabilities::conductor::service::work_tools::handlers),
        );
        let c = Conductor {
            registry,
            history,
            workspace,
            vm,
            modules,
            builds,
            compact,
            tools,
            log,
            probe,
            residents,
            secrets,
            prompt,
            systools,
            turn,
            work,
        };
        c.sync_compact_choice();
        c
    }

    /// 目的：从登记处重算压缩选择（设置 / 模型 / 供应商变更后调用）。
    /// 约束：只读登记处、只换共享选择；不重建任何会话。
    pub(crate) fn sync_compact_choice(&self) {
        let app = self.registry.app();
        let id = app.compact_model.trim();
        let choice = if id.is_empty() {
            CompactChoice::Session
        } else {
            match self.registry.channel(Some(id)) {
                Ok(channel) => CompactChoice::Model {
                    id: id.to_string(),
                    channel,
                    window: self.registry.context_of(Some(id)),
                },
                Err(e) => CompactChoice::Unavailable(format!("压缩模型 {} 不可用：{}", id, e)),
            }
        };
        self.compact.set(choice);
    }

    /// 目的：注入项目内基础工具链的只读安装根；组合根读到 active.json 后调用。
    /// 约束：核心与会话本体各留一份（工具链根既是会话装配的输入，也是核心自己起 shell 的依据），同一处注入保持一致。
    pub fn with_base_roots(mut self, roots: Vec<std::path::PathBuf>) -> Conductor {
        self.work.set_base_roots(roots.clone());
        self.turn.set_base_roots(roots);
        self
    }

    /// 目的：宿主能力探测面（会话重建要它认虚拟根与可执行文件）。
    pub(crate) fn probe(&self) -> &dyn crate::kernel::ports::HostProbe {
        self.probe.as_ref()
    }

    /// 目的：把会话本体与它按需取用的两个事实面**拆开借**：会话装配途中要 `&mut` 自己，
    ///   同时读登记处与宿主探测（两者不是会话的字段——`registry_mut` 要 `&mut`，共享会逼出内部可变性）。
    /// 返回：会话本体、登记处、宿主探测。
    pub(crate) fn turn_parts(
        &mut self,
    ) -> (
        &mut crate::capabilities::turn::api::Turn,
        &dyn Registry,
        &dyn crate::kernel::ports::HostProbe,
    ) {
        (&mut self.turn, self.registry.as_ref(), self.probe.as_ref())
    }

    /// 目的：模块的**有效**启用（用户覆盖 ?? 作者默认）。
    pub fn module_enabled(&self, id: &str) -> bool {
        self.modules.module_enabled(id)
    }

    /// 目的：服务的**有效**启用（模块停用 ⇒ false）。
    pub fn service_enabled(&self, module: &str, service: &str) -> bool {
        self.modules.service_enabled(module, service)
    }

    /// 目的：模块能力面（手柄与核心共享同一份；启停写面经它）。
    pub(crate) fn modules_handle(&self) -> Arc<dyn Modules + Send + Sync> {
        Arc::clone(&self.modules)
    }

    /// 目的：构建能力面（手柄与核心共享同一份；创建会话前在调用方线程补编，核心线程不陪着等编译）。
    pub(crate) fn builds_handle(&self) -> Arc<dyn Builds + Send + Sync> {
        Arc::clone(&self.builds)
    }

    /// 目的：常驻服务面（手柄与核心共享同一份；动作与会话回收经它）。
    pub(crate) fn residents_handle(
        &self,
    ) -> Arc<dyn crate::capabilities::residents::api::ResidentOps + Send + Sync> {
        Arc::clone(&self.residents)
    }

    /// 目的：隐秘字段面（手柄与核心共享同一份）。
    pub(crate) fn secrets_handle(
        &self,
    ) -> Arc<dyn crate::capabilities::secrets::api::SecretOps + Send + Sync> {
        Arc::clone(&self.secrets)
    }

    /// 目的：日志端口句柄：入站手柄（conductor::api）与组合根共用同一份事实记录。
    pub fn log_handle(&self) -> Arc<dyn crate::kernel::ports::Log + Send + Sync> {
        Arc::clone(&self.log)
    }

    /// 目的：工具执行面句柄——入站手柄跑"人直接用的模块工具"要用同一份执行面（不重装）。
    pub fn tools_handle(&self) -> Arc<dyn ToolExec + Send + Sync> {
        Arc::clone(&self.tools)
    }

    /// 目的：登记处能力面（只读）：组合根与测试读登记处的事实走这里。
    pub fn registry(&self) -> &dyn Registry {
        self.registry.as_ref()
    }

    /// 目的：登记处能力面（可写）：登记处的用例只经它调用。
    pub fn registry_mut(&mut self) -> &mut dyn Registry {
        self.registry.as_mut()
    }

    /// 目的：登记处写操作的统一出口——写完重算压缩选择，避免汇总模型指向旧通道。
    pub(crate) fn registry_write<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Registry) -> Result<T, String>,
    ) -> Result<T, String> {
        let out = f(self.registry.as_mut())?;
        self.sync_compact_choice();
        Ok(out)
    }

    /// 目的：工具总表的声明书（代理执行者在队列桥那一侧校验参数/拼回执要用）。
    pub(crate) fn systools_book(&self) -> crate::capabilities::tools::api::ToolBook {
        self.systools.book()
    }

    /// 目的：给压缩回合准备身份块；切分与模型调用都在工作线程上（界面不被阻塞）。
    /// 返回：本回合的身份块。
    /// 错误：会话不在世或不是单 agent 时如实报错。
    pub fn compact_plan(&mut self, sid: &str) -> Result<String, String> {
        // 会话不在世或不是单 agent：**报错**，不给出"空身份块"这种假计划
        // （压缩只对单 agent 会话成立；协作会话由各成员会话按阈值自己压）。
        let model = match self.turn.session(sid) {
            Some(Session::Single(s)) => s.model_id().map(str::to_string),
            _ => {
                return Err(format!(
                    "压缩只支持单 agent 会话（{} 不在世或不是单 agent）",
                    sid
                ));
            }
        };
        // 手动压缩也要读**当前**设置：用户刚把保留比例调低，不该等会话重建才生效。
        let (window, percent, keep) = self.turn.context_budget(model.as_deref(), self.registry());
        let Some(Session::Single(s)) = self.turn.session_mut(sid) else {
            return Err(format!(
                "压缩只支持单 agent 会话（{} 不在世或不是单 agent）",
                sid
            ));
        };
        s.set_context_budget(window, percent, keep);
        Ok(s.params().identity(&*self.prompt, s.tool_mode()))
    }

    /// 目的：往某个会话注入一条**系统消息**（讨论的提醒走这条；见 session-model.md 二"系统消息"）。
    ///  返回要外送/落盘的事件。
    pub fn note_system(&mut self, sid: &str, text: &str) -> Result<Vec<SessionEvent>, String> {
        let mut s = self.turn.take_single(sid)?;
        let events = s.note_system(text);
        // 会话本体不落盘：它把「要通知父会话」的事实交回来，落盘仍由核心做。
        let back = self.turn.put_single(sid, s);
        if let Some((parent, mut evs)) = back.notice {
            self.record_events(&parent, &mut evs);
        }
        self.persister(sid).persist(&events);
        Ok(events)
    }

    /// 目的：给某个会话接下来的行打上**整场工作的下一个回合 id**（节点执行也用同一套编号）。
    ///  一个 agent 一个会话：它的回合计数来自父会话——回档同步靠两边同一套编号。
    pub(crate) fn bump_turn_of_child(&mut self, child: &str) -> u64 {
        let parent = self
            .history
            .load(child)
            .ok()
            .and_then(|(m, _)| m.parent.clone());
        let Some(parent) = parent else {
            return 0;
        };
        let tid = match self.turn.session_mut(&parent) {
            Some(Session::Collab(c)) => c.next_turn_id(),
            _ => 0,
        };
        if let Some(Session::Single(s)) = self.turn.session_mut(child) {
            s.set_turn(tid);
        }
        tid
    }

    // ---- 会话历史 ----
}

// 测试同步入口：跨段的活儿（准备 → 起 guest → 提交）在这里一次跑完，测试不必在两段之间等 QEMU。
// 生产面没有这些方法：它们只是把测试对工作能力的调用转给 `self.work`，不参与任何生产路径。
#[cfg(test)]
impl Conductor {
    /// 目的：测试用同步建工作（生产走句柄的两段 `prepare_work` / `commit_work`）。
    pub fn create_work(
        &mut self,
        spec: crate::capabilities::work::api::WorkSpec,
    ) -> Result<crate::capabilities::work::api::WorkOpened, String> {
        self.work.create_work(&mut self.turn, &*self.registry, spec)
    }

    /// 目的：测试用同步建会话（人经呈现层与核心代理共用的那一条的同步形态）。
    pub fn create_session(
        &mut self,
        spec: &crate::capabilities::proxy::api::NewSession,
    ) -> Result<
        (
            crate::capabilities::proxy::api::Created,
            Vec<crate::capabilities::session::api::SessionEvent>,
        ),
        String,
    > {
        self.work
            .create_session(&mut self.turn, &*self.registry, spec)
    }

    #[allow(clippy::too_many_arguments)]
    /// 目的：测试用同步建代理工作（生产走句柄的 `create_proxy`）。
    pub fn create_proxy_with_facts(
        &mut self,
        name: &str,
        granted_at: i64,
        tier: crate::kernel::api::Tier,
        guest: Option<&str>,
        profile: crate::capabilities::permission::api::ProfileId,
        workspace: Option<crate::capabilities::session::api::WorkspaceBinding>,
    ) -> Result<(String, Vec<crate::capabilities::session::api::SessionEvent>), String> {
        self.work.create_proxy_with_facts(
            &mut self.turn,
            &*self.registry,
            name,
            granted_at,
            tier,
            guest,
            profile,
            workspace,
        )
    }

    /// 目的：测试用同步改权限档（生产走 `registry_write` 的那条命令）。
    pub fn set_permission_profile(
        &mut self,
        sid: &str,
        profile: crate::capabilities::permission::api::ProfileId,
    ) -> Result<(), String> {
        self.work
            .set_permission_profile(&mut self.turn, &*self.registry, sid, profile)
    }

    /// 目的：测试用同步改配（生产走句柄的两段 `prepare_edit` / `commit_edit`）。
    pub fn edit_session(
        &mut self,
        sid: &str,
        edit: crate::capabilities::work::api::SessionEdit,
    ) -> Result<(), String> {
        self.work
            .edit_session(&mut self.turn, &*self.registry, sid, edit)
    }

    /// 目的：测试用改配的第一段（校验 + 出计划）。
    pub fn prepare_edit(
        &mut self,
        sid: &str,
        edit: crate::capabilities::work::api::SessionEdit,
    ) -> Result<
        (
            crate::capabilities::work::api::PreparedEdit,
            Option<crate::capabilities::work::api::EditGuest>,
        ),
        String,
    > {
        self.work
            .prepare_edit(&mut self.turn, &*self.registry, sid, edit)
    }

    /// 目的：测试用改配的第二段（写回 + 丢掉内存会话）。
    pub fn commit_edit(
        &mut self,
        prepared: crate::capabilities::work::api::PreparedEdit,
    ) -> Result<(), String> {
        self.work
            .commit_edit(&mut self.turn, &*self.registry, prepared)
    }

    /// 目的：测试用启用的第一段（判是否已在跑 + 准备 guest）。
    pub fn prepare_activation(
        &self,
        sid: &str,
    ) -> Result<crate::capabilities::work::api::PreparedActivation, String> {
        self.work
            .prepare_activation(&self.turn, &*self.registry, sid)
    }

    /// 目的：测试用启用的第二段（重建 + 解冻整棵子树）。
    pub fn commit_activation(
        &mut self,
        prepared: crate::capabilities::work::api::PreparedActivation,
    ) -> Result<crate::capabilities::work::api::SessionView, String> {
        self.work
            .commit_activation(&mut self.turn, &*self.registry, prepared)
    }

    /// 目的：测试读档位可选项（与某条会话无关的机器能力）。
    pub fn tier_choices(&self) -> crate::capabilities::work::api::TierChoices {
        self.work.tier_choices(&*self.registry)
    }

    /// 目的：测试用投喂文件到本次工作的 work/。
    pub fn work_upload(
        &mut self,
        sid: &str,
        name: &str,
        bytes: &[u8],
        overwrite: bool,
    ) -> Result<bool, String> {
        self.work
            .work_upload(&mut self.turn, sid, name, bytes, overwrite)
    }

    /// 目的：测试用切换上下文新鲜度。
    pub fn set_context_freshness(&mut self, sid: &str, on: bool) -> Result<(), String> {
        self.work
            .set_context_freshness(&mut self.turn, &*self.registry, sid, on)
    }

    /// 目的：测试读会话配置视图。
    pub fn session_config(
        &self,
        sid: &str,
    ) -> Result<crate::capabilities::work::api::SessionConfig, String> {
        self.work.session_config(&self.turn, &*self.registry, sid)
    }

    /// 目的：测试判会话是否在世（按名字）。
    pub fn session_exists(&self, name: &str) -> bool {
        self.work.session_exists(&self.turn, name)
    }

    /// 目的：测试读一次生成要用的调用参数。
    pub fn llm_opts(&self, want_stream: bool) -> crate::capabilities::llm::api::LlmOpts {
        self.work.llm_opts(&*self.registry, want_stream)
    }

    /// 目的：测试读文件视图。
    pub fn files_view(
        &self,
        sid: &str,
    ) -> Result<crate::capabilities::work::api::FilesView, String> {
        self.work.files_view(&self.turn, &*self.registry, sid)
    }

    /// 目的：测试读运行能力报告。
    pub fn runtime_report(
        &self,
        tier: crate::kernel::api::Tier,
    ) -> crate::capabilities::work::api::RuntimeReport {
        self.work.runtime_report(&self.turn, &*self.registry, tier)
    }

    /// 目的：测试读模块名册。
    pub fn scan(&self) -> crate::capabilities::modules::api::Roster {
        self.work.scan()
    }

    /// 目的：测试读模型建议（按任务挑 agent 的候选）。
    pub fn suggest_models(
        &self,
        task: &str,
        mode: crate::capabilities::work::api::WorkMode,
    ) -> Result<
        (
            Vec<crate::capabilities::work::api::AgentSuggestion>,
            Vec<crate::capabilities::session::api::SessionEvent>,
        ),
        String,
    > {
        self.work.suggest_models(&*self.registry, task, mode)
    }
}

// 测试访问器：验证职责提示词已种入历史首条（回归：会话曾丢失 system 提示词）。
#[cfg(test)]
impl Conductor {
    pub fn single_history(&self, sid: &str) -> Option<Vec<Msg>> {
        match self.turn.session(sid) {
            Some(Session::Single(s)) => Some(s.dialogue().to_vec()),
            _ => None,
        }
    }

    // 测试访问器：读台账（验证"回放 = 实时"：重建后的事实与正文必须逐条一致）。
    pub fn single_reads(
        &self,
        sid: &str,
    ) -> Option<Vec<crate::capabilities::session::api::ReadAt>> {
        match self.turn.session(sid) {
            Some(Session::Single(s)) => Some(s.reads().to_vec()),
            _ => None,
        }
    }

    /// 目的：测试用：往某个会话的流水追加事件（造「压过之后再重启」这类历史）。
    pub fn history_append(
        &mut self,
        sid: &str,
        events: &[serde_json::Value],
    ) -> Result<(), String> {
        self.history.append(sid, events)
    }

    /// 目的：该会话此刻的**身份块**（按当前提示词册与形态现渲染）：测试用它断言"它被告诉了什么"。
    pub fn single_identity(&self, sid: &str) -> Option<String> {
        match self.turn.session(sid) {
            Some(Session::Single(s)) => Some(s.params().identity(&*self.prompt, s.tool_mode())),
            _ => None,
        }
    }
}

/// 继续被拦下时的提醒（单 agent 会话：该轮到用户发言）。
const NEED_USER: &str = "最后一条是 AI 发言：部分供应商不允许连续 AI 发言；请先发言再继续。";
