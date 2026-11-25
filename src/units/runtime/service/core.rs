//! 目的：运行时的核心本体（Core）——持有注入的端口、能力面与会话中心，只在核心自己的线程上被碰。
//! 管：容器各成员（端口 / 能力面 / 会话中心 / base roots）与访问器，以及对下层用例的薄转发（一行一次）。
//! 不管：命令队列与线程切换（在 handle）、动作分发（在 action）、回合与会话内部（在下层能力）。
//! 联动：组合根造它（main.rs）；service/handle 在 spawn 里把它移进执行线程；测试访问器供契约测试读会话内容。

use super::*;

/// 目的：**应用服务**：持有注入的端口、能力面与会话中心；入口与呈现层拿不到它（只拿 `presentation::faces::Faces`）。
///  组合根在 `Handle::spawn` 里把它**移进核心自己的执行线程**——此后状态只被那一个线程碰，
///  所以这里不加任何锁（并发不变式见 docs/architecture/layers.md）。
pub struct Core {
    /// 目的：登记处能力（**四份 yaml 的状态在它里面**）——runtime 只按 `Registry` 调用，看不见它的字段。
    pub(crate) registry: Box<dyn Registry>,
    /// 目的：会话历史直连面（**不持它的端口**，R12）——造/读/写/删会话都走它。
    pub(super) history: Arc<dyn History + Send + Sync>,
    /// 目的：虚拟机档用例面（**不持它的端口**，R12）——guest 生命周期与档位就绪检测走它。
    pub(super) vm: Arc<dyn Vm + Send + Sync>,
    /// 目的：模块能力（**不持它的端口**，R12）——模块清单事实的唯一上游。
    pub(super) modules: Arc<dyn Modules + Send + Sync>,
    /// 目的：构建能力面（**不持它的端口**，R12）——创建会话前补编声明了 `build:` 的模块。
    pub(super) builds: Arc<dyn Builds + Send + Sync>,
    /// 目的：压缩通道工厂——全局设置只在这里对账，会话在压缩触发时现取（不刷新会话）。
    pub(super) compact: Arc<CompactRuntime>,
    /// 目的：工具执行面（**不持它的端口**，R12）——跑外部/内置工具、释放围栏授权都走它。
    pub(super) tools: Arc<dyn ToolExec + Send + Sync>,
    pub(super) log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 目的：宿主能力探测——读环境、查路径存在性都在它后面，runtime 因此不碰 std::env 与文件系统。
    pub(crate) probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
    /// 目的：常驻服务的统一管理面（**不持它的端口**）——动作面与会话回收经它。
    pub(super) residents: Arc<dyn crate::units::residents::api::ResidentOps + Send + Sync>,
    /// 目的：隐秘字段的统一管理面——工具 / 服务起进程时按它解析注入项并脱敏。
    pub(super) secrets: Arc<dyn crate::units::catalog::secrets::api::SecretOps + Send + Sync>,
    /// 目的：项目内基础工具链的只读安装根（python / node / cc）——shell 每次调用都放行它们。
    ///  来源是组合根读到的 active.json；不按 agent 的模块重复注入。
    pub(super) base_roots: Vec<std::path::PathBuf>,
    /// 目的：工具总表与机制席位的能力面（授权两张表；名字表由组合根另取）——**表本体在工具能力里**，与册子互不依赖。
    pub(super) systools: Arc<dyn Tools>,
    /// 目的：回合状态与会话本体——在世会话表、运行态与创建占位、裁决队、每会话的 shell 活进程槽，以及一席的装配（在 `turn`）。
    pub(crate) turn: crate::units::turn_state::api::Turn,
    /// 目的：工作与会话配置的用例面（层 9）——建工作 / 改配 / 视图 / 报告 / guest 起停都经它。
    ///   本能力只做命令队列、协作链推进与代理工具族；work 用例的方法原样转发。
    pub(crate) work: Work,
    /// 目的：会话历史生命周期（层 9）——历史列出 / 只读打开 / 删除收尾 / 回档三模式都经它。
    ///   会话本体按形参交过去（`&mut turn`），它不持第二份。
    pub(crate) lifecycle: crate::units::lifecycle::api::Lifecycle,
}

/// 目的：在世会话形态（`turn` 的状态类型）：测试访问器用它读会话内容。
#[cfg(test)]
pub(crate) use crate::units::turn_state::api::Session;

/// 目的：会话本体（`turn` 的 `Turn`）：准入、派发闸、重建与代理由这里转调。
pub(crate) use crate::units::turn_state::api::Turn;

impl Core {
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
        residents: Arc<dyn crate::units::residents::api::ResidentOps + Send + Sync>,
        secrets: Arc<dyn crate::units::catalog::secrets::api::SecretOps + Send + Sync>,
        profiles: crate::units::session_model::api::Profiles,
    ) -> Core {
        let compact = Arc::new(CompactRuntime::new(Arc::clone(&llm)));
        // 流程工厂在这里注入：turn / work 只认 `session::api::Flows`，具体流程的类型只出现在协作能力里。
        let flows: Arc<dyn crate::units::turn_state::api::Flows> =
            Arc::new(crate::units::conversation::collab::api::CollabFlows::new(
                Arc::clone(&llm),
                Arc::clone(&vm),
                Arc::clone(&modules),
                Arc::clone(&prompt),
                Arc::clone(&systools),
                Arc::clone(&tools),
                Arc::clone(&log),
                profiles,
            ));
        // 会话本体收在 turn：它只认能力面与压缩工厂，不认 runtime 的端口（层 8 不得依赖层 9）。
        let turn = crate::units::turn_state::api::Turn::new(
            Arc::clone(&workspace),
            Arc::clone(&vm),
            Arc::clone(&residents),
            Arc::clone(&secrets),
            Arc::clone(&tools),
            Arc::clone(&systools),
            Arc::clone(&prompt),
            Arc::clone(&llm),
            Arc::clone(&log),
            compact.clone(),
            Vec::new(),
            root.clone(),
            Arc::clone(&history),
            Arc::clone(&modules),
            Arc::clone(&packages),
            Arc::clone(&flows),
        );
        // work 用例面与 runtime 共享同一批能力面（Arc 克隆，不新建实现）。
        let work = Work::new(
            root.clone(),
            Arc::clone(&history),
            Arc::clone(&workspace),
            Arc::clone(&vm),
            Arc::clone(&packages),
            Arc::clone(&provision),
            Arc::clone(&modules),
            Arc::clone(&llm),
            Arc::clone(&tools),
            Arc::clone(&prompt),
            Arc::clone(&systools),
            Arc::clone(&log),
            flows,
        );
        let lifecycle = crate::units::lifecycle::api::Lifecycle::new(
            Arc::clone(&history),
            Arc::clone(&workspace),
            Arc::clone(&tools),
            Arc::clone(&vm),
            Arc::clone(&residents),
            Arc::clone(&log),
        );
        let c = Core {
            registry,
            history,
            vm,
            modules,
            builds,
            compact,
            tools,
            log,
            probe,
            residents,
            secrets,
            base_roots: Vec::new(),
            systools,
            turn,
            work,
            lifecycle,
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
    pub fn with_base_roots(mut self, roots: Vec<std::path::PathBuf>) -> Core {
        self.base_roots = roots.clone();
        self.turn.set_base_roots(roots);
        self
    }

    /// 目的：模块的**有效**启用（用户覆盖 ?? 作者默认）。
    pub fn module_enabled(&self, id: &str) -> bool {
        self.modules.module_enabled(id)
    }

    /// 目的：服务的**有效**启用（模块停用 ⇒ false）。
    pub fn service_enabled(&self, module: &str, service: &str) -> bool {
        self.modules.service_enabled(module, service)
    }

    /// 目的：取历史落盘端口的句柄（与 `log_handle` 同形态），给 api 层与测试调会话树派生。
    pub(crate) fn history_handle(&self) -> Arc<dyn History + Send + Sync> {
        Arc::clone(&self.history)
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
    ) -> Arc<dyn crate::units::residents::api::ResidentOps + Send + Sync> {
        Arc::clone(&self.residents)
    }

    /// 目的：隐秘字段面（手柄与核心共享同一份）。
    pub(crate) fn secrets_handle(
        &self,
    ) -> Arc<dyn crate::units::catalog::secrets::api::SecretOps + Send + Sync> {
        Arc::clone(&self.secrets)
    }

    /// 目的：日志端口句柄：入站手柄（runtime::api）与组合根共用同一份事实记录。
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

    /// 目的：工具总表面（手柄与核心共享同一份）：代理执行者在队列桥那一侧校验参数、拼回执要用。
    pub(crate) fn systools_handle(&self) -> Arc<dyn crate::units::tools::api::Tools> {
        Arc::clone(&self.systools)
    }
}

#[cfg(test)]
impl Core {
    pub fn single_history(&self, sid: &str) -> Option<Vec<Msg>> {
        match self.turn.session(sid) {
            Some(Session::Single(s)) => Some(s.dialogue().to_vec()),
            _ => None,
        }
    }

    pub fn single_reads(&self, sid: &str) -> Option<Vec<crate::units::agentloop::api::ReadAt>> {
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
        self.turn.identity_block(sid)
    }
}

impl Core {
    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn activation_build_request(
        &self,
        sid: &str,
    ) -> Result<
        (
            Vec<String>,
            crate::units::pure::exec_model::api::Tier,
            Option<String>,
        ),
        String,
    > {
        self.work.activation_build_request(sid)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn prepare_activation(&self, sid: &str) -> Result<PreparedActivation, String> {
        self.work
            .prepare_activation(&self.turn, self.registry.as_ref(), &self.probe, sid)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn commit_activation(
        &mut self,
        prepared: PreparedActivation,
    ) -> Result<SessionView, String> {
        self.work.commit_activation(
            &mut self.turn,
            self.registry.as_ref(),
            self.probe.as_ref(),
            prepared,
        )
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn session_config(&self, sid: &str) -> Result<SessionConfig, String> {
        self.work
            .session_config(&self.turn, self.registry.as_ref(), self.probe.as_ref(), sid)
    }

    #[cfg(test)]
    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn edit_session(&mut self, sid: &str, edit: SessionEdit) -> Result<(), String> {
        self.work.edit_session(
            &mut self.turn,
            self.registry.as_ref(),
            &self.probe,
            sid,
            edit,
        )
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn prepare_edit(
        &mut self,
        sid: &str,
        edit: SessionEdit,
    ) -> Result<(PreparedEdit, Option<EditGuest>), String> {
        self.work.prepare_edit(
            &mut self.turn,
            self.registry.as_ref(),
            &self.probe,
            sid,
            edit,
        )
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn set_permission_profile(
        &mut self,
        sid: &str,
        profile: crate::units::pure::permission::api::ProfileId,
    ) -> Result<(), String> {
        self.work
            .set_permission_profile(&mut self.turn, self.registry.as_ref(), sid, profile)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn commit_edit(&mut self, prepared: PreparedEdit) -> Result<(), String> {
        self.work
            .commit_edit(&mut self.turn, self.registry.as_ref(), prepared)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn unique_work_name(&self, base: &str, fallback: &str) -> String {
        self.work.unique_work_name(&self.turn, base, fallback)
    }

    #[cfg(test)]
    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn create_work(&mut self, spec: WorkSpec) -> Result<WorkOpened, String> {
        self.work
            .create_work(&mut self.turn, self.registry.as_ref(), &self.probe, spec)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn prepare_work(
        &mut self,
        spec: WorkSpec,
        parent: Option<&str>,
    ) -> Result<(PreparedWork, Option<GuestBoot>), String> {
        self.work.prepare_work(
            &mut self.turn,
            self.registry.as_ref(),
            &self.probe,
            spec,
            parent,
        )
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn commit_work(&mut self, prepared: PreparedWork) -> Result<WorkOpened, String> {
        self.work.commit_work(
            &mut self.turn,
            self.registry.as_ref(),
            self.probe.as_ref(),
            prepared,
        )
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn abort_work(&mut self, prepared: PreparedWork) {
        self.work.abort_work(&mut self.turn, prepared)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn prepare_guest_boot(&self, sid: &str) -> Result<Option<GuestBoot>, String> {
        self.work
            .prepare_guest_boot(&self.turn, self.registry.as_ref(), &self.probe, sid)
    }

    /// 目的：测试访问器：本次模型调用的通道参数（判据在 turn，与生产同一条）。
    #[cfg(test)]
    pub(crate) fn llm_opts(&self, want_stream: bool) -> crate::units::llm::api::LlmOpts {
        crate::units::turn::turn::api::llm_opts(self.registry.as_ref(), want_stream)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn set_thinking(&mut self, sid: &str, level: &str) -> Result<(), String> {
        self.work
            .set_thinking(&mut self.turn, self.registry.as_ref(), sid, level)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）；保底 percent 缺省沿用现值。
    pub(crate) fn set_context_freshness(
        &mut self,
        sid: &str,
        on: bool,
        keep_percent: Option<u8>,
    ) -> Result<(), String> {
        self.work.set_context_freshness(
            &mut self.turn,
            self.registry.as_ref(),
            sid,
            on,
            keep_percent,
        )
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn runtime_report(&self, tier: crate::units::pure::exec_model::api::Tier) -> RuntimeReport {
        self.work
            .runtime_report(self.registry.as_ref(), self.probe.as_ref(), tier)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn tier_choices(&self) -> TierChoices {
        self.work
            .tier_choices(self.registry.as_ref(), self.probe.as_ref())
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn host_plan_now(
        &self,
        modules: &[Module],
    ) -> crate::units::catalog::packages::api::PackagePlan {
        self.work.host_plan_now(self.probe.as_ref(), modules)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn unavailable_modules_now(&self) -> BTreeMap<String, Vec<String>> {
        self.work.unavailable_modules_now(self.probe.as_ref())
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn suggest_models(
        &self,
        task: &str,
        kind: &str,
    ) -> Result<(Vec<AgentSuggestion>, Vec<SessionEvent>), String> {
        self.work.suggest_models(self.registry.as_ref(), task, kind)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn session_types(&self) -> Vec<crate::units::work::api::SessionTypeView> {
        self.work.session_types()
    }

    /// 目的：薄转发到 `work` 的同名方法（临时拼装时把流程 id 解析成声明）。
    pub fn flow_ids(&self, ids: &[String]) -> Result<Vec<String>, String> {
        self.work.flow_ids(ids)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn ambient_workspace(
        &self,
        raw: &std::path::Path,
        tier: crate::units::pure::exec_model::api::Tier,
    ) -> Result<std::path::PathBuf, String> {
        self.work.ambient_workspace(self.probe.as_ref(), raw, tier)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn session_views(&self, history: &[HistoryView]) -> Vec<SessionView> {
        self.work.session_views(
            &self.turn,
            self.registry.as_ref(),
            self.probe.as_ref(),
            history,
        )
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn session_exists(&self, name: &str) -> bool {
        self.work.session_exists(&self.turn, name)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn work_upload(
        &mut self,
        sid: &str,
        name: &str,
        bytes: &[u8],
        overwrite: bool,
    ) -> Result<bool, String> {
        self.work
            .work_upload(&self.turn, sid, name, bytes, overwrite)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn files_view(&self, sid: &str) -> Result<FilesView, String> {
        self.work
            .files_view(&self.turn, self.registry.as_ref(), self.probe.as_ref(), sid)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn work_log(
        &self,
        sid: &str,
        limit: usize,
        before: Option<u64>,
    ) -> Result<crate::units::workspace::api::WorkLog, String> {
        self.work.work_log(sid, limit, before)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn work_commit_view(
        &self,
        sid: &str,
        commit: u64,
    ) -> Result<crate::units::workspace::api::CommitDetail, String> {
        self.work.work_commit_view(sid, commit)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub fn work_commit_file(
        &self,
        sid: &str,
        commit: u64,
        path: &str,
    ) -> Result<crate::units::workspace::api::CommitFileView, String> {
        self.work.work_commit_file(sid, commit, path)
    }

    /// 目的：薄转发到 `work` 的同名方法（runtime 只做应用编排）。
    pub(crate) fn restore_work_commit(
        &mut self,
        sid: &str,
        commit: u64,
        expect_head: u64,
        force: bool,
    ) -> Result<(Option<u64>, Vec<SessionEvent>), String> {
        self.work
            .restore_work_commit(&mut self.turn, sid, commit, expect_head, force)
    }
}
