//! 目的：会话本体——回合状态（`TurnState`）加上装配一席所需的能力面，一处持有。
//! 管：`Turn` 的字段与构造、会话在世的访问器转发、运行中文案、能力面的取用（`session_deps`）、压缩通道工厂。
//! 不管：生成驱动、落盘与命令队列（在 `runtime`）；工具面与会话对象的拼装（在同级 `assemble`）；纯派生规则（在 `turn::domain`）。
//! 联动：`runtime` 持一份 `Turn` 并经它放取会话；`assemble` 从 `session_deps` 取能力面。

use std::path::PathBuf;
use std::sync::Arc;

use crate::capabilities::llm::api::Llm;
use crate::capabilities::modules::api::Modules;
use crate::capabilities::packages::api::Packages;
use crate::capabilities::prompt::api::Prompt;
use crate::capabilities::residents::api::ResidentOps;
use crate::capabilities::secrets::api::SecretOps;
use crate::capabilities::session::api::{CompactChatFactory, Gates, History, ShellLive, ShellSlot};
use crate::capabilities::tools::api::{ToolExec, Tools};
use crate::capabilities::turn::api::{Session, TurnState};
use crate::capabilities::turn::service::assemble::SessionDeps;
use crate::capabilities::vm::api::Vm;
use crate::capabilities::workspace::api::Workspace;
use crate::kernel::api::SessionId;
use crate::kernel::ports::Log;

/// 目的：会话本体：在世表与运行态（`TurnState`）连同装配一席要用的能力面，一处持有。
/// 约束：只在核心线程上读写；能力面只经 `api` 取。
pub struct Turn {
    /// 目的：在世会话表、运行态与创建占位、裁决队、每会话的 shell 活进程槽。
    pub(super) state: TurnState,
    /// 目的：工作区用例面：一席的 `work_*` 工具执行者按它造。
    pub(super) workspace: Arc<dyn Workspace + Send + Sync>,
    /// 目的：虚拟机用例面：一席的 guest 事实随工具面交给机制层。
    pub(super) vm: Arc<dyn Vm + Send + Sync>,
    /// 目的：常驻服务管理面：已就绪服务的操作并入一席的工具面。
    pub(super) residents: Arc<dyn ResidentOps + Send + Sync>,
    /// 目的：隐秘字段管理面：装配期按模块解析注入项。
    pub(super) secrets: Arc<dyn SecretOps + Send + Sync>,
    /// 目的：工具执行面：一席的工具环境要它跑外部 / 内置工具。
    pub(super) tools: Arc<dyn ToolExec + Send + Sync>,
    /// 目的：工具总表与角色表：发放名单与内置工具表从它读。
    pub(super) systools: Arc<dyn Tools>,
    /// 目的：提示词册：一席的工具说明与压缩提示从它取段。
    pub(super) prompt: Arc<dyn Prompt>,
    /// 目的：llm 用例面：一席的收发句柄按它造。
    pub(super) llm: Arc<dyn Llm + Send + Sync>,
    /// 目的：日志面：装配时记一条「单 agent 工作」。
    pub(super) log: Arc<dyn Log + Send + Sync>,
    /// 目的：压缩通道工厂：新会话与重建会话共用同一份。
    pub(super) compact: Arc<dyn CompactChatFactory>,
    /// 目的：项目内基础工具链的只读安装根（python / node / cc），由组合根注入。
    pub(super) base_roots: Vec<PathBuf>,
    /// 目的：产品根：外部工作区绑定校验与 guest 宿主翻译都以它为锚。
    pub(super) root: PathBuf,
    /// 目的：会话落盘与名录：重建会话、准入与派发闸从它读元数据与事件。
    pub(super) history: Arc<dyn History + Send + Sync>,
    /// 目的：模块登记：重建会话按清单复原一席的模块与目录。
    pub(super) modules: Arc<dyn Modules + Send + Sync>,
    /// 目的：包管理面：重建会话时按档位取库根与基础包根。
    pub(super) packages: Arc<dyn Packages + Send + Sync>,
}

/// 目的：`Turn` 的构造与取用。
impl Turn {
    /// 目的：造一份会话本体：回合状态随之就绪，能力面与基础工具链根由组合根注入。
    // 会话本体把"装配一席要的能力面"一次说全；收口成参数对象只是把参数挪个地方。
    // 这是有意的设计取舍（见 docs/testing/quality-isolation.md 的 allow 清单），不是没修。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace: Arc<dyn Workspace + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        residents: Arc<dyn ResidentOps + Send + Sync>,
        secrets: Arc<dyn SecretOps + Send + Sync>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        systools: Arc<dyn Tools>,
        prompt: Arc<dyn Prompt>,
        llm: Arc<dyn Llm + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        compact: Arc<dyn CompactChatFactory>,
        base_roots: Vec<PathBuf>,
        root: PathBuf,
        history: Arc<dyn History + Send + Sync>,
        modules: Arc<dyn Modules + Send + Sync>,
        packages: Arc<dyn Packages + Send + Sync>,
    ) -> Turn {
        Turn {
            state: TurnState::new(),
            workspace,
            vm,
            residents,
            secrets,
            tools,
            systools,
            prompt,
            llm,
            log,
            compact,
            base_roots,
            root,
            history,
            modules,
            packages,
        }
    }

    /// 目的：换掉基础工具链的只读安装根（组合根读到 active.json 后调用）。
    pub fn set_base_roots(&mut self, roots: Vec<PathBuf>) {
        self.base_roots = roots;
    }

    /// 目的：装配一席要用的能力面——按 `Turn` 现持的那几份克隆出来。
    pub fn session_deps(&self) -> SessionDeps {
        SessionDeps {
            llm: Arc::clone(&self.llm),
            log: Arc::clone(&self.log),
            prompt: Arc::clone(&self.prompt),
            systools: Arc::clone(&self.systools),
            tools: Arc::clone(&self.tools),
            vm: Arc::clone(&self.vm),
            residents: Arc::clone(&self.residents),
            workspace: Arc::clone(&self.workspace),
            secrets: Arc::clone(&self.secrets),
            compact: Arc::clone(&self.compact),
        }
    }

    /// 目的：注入给新会话 / 重建会话的压缩通道工厂。
    pub fn compact_factory(&self) -> Arc<dyn CompactChatFactory> {
        Arc::clone(&self.compact)
    }

    /// 目的：生成期间"会话不在表里"的三种进入点共用这一句（错误文案要一致，别处不再各写一份）。
    pub fn running_refusal(sid: &str) -> String {
        format!("会话 {} 正在生成中：先「停止」或等它结束，再做这一步", sid)
    }

    /// 目的：取一份在世会话的只读引用。
    pub fn session(&self, sid: &str) -> Option<&Session> {
        self.state.session(sid)
    }

    /// 目的：取一份在世会话的可写引用。
    pub fn session_mut(&mut self, sid: &str) -> Option<&mut Session> {
        self.state.session_mut(sid)
    }

    /// 目的：这份会话是否在世。
    pub fn holds(&self, sid: &str) -> bool {
        self.state.holds(sid)
    }

    /// 目的：把会话放进在世表（生成结束后放回，或新建时放入）。
    pub fn put(&mut self, sid: impl Into<SessionId>, session: Session) {
        self.state.put(sid, session);
    }

    /// 目的：从在世表取出会话（生成开始时取出，表里不留对象）。
    pub fn take(&mut self, sid: &str) -> Option<Session> {
        self.state.take(sid)
    }

    /// 目的：这份会话是否正在生成。
    pub fn is_running(&self, sid: impl AsRef<str>) -> bool {
        self.state.is_running(sid)
    }

    /// 目的：标记开始生成（生成期间拒绝改配 / 回档 / 删除）。
    pub fn mark_running(&mut self, sid: impl Into<SessionId>) {
        self.state.mark_running(sid);
    }

    /// 目的：撤销运行态（区间结束或中断收尾）。
    pub fn unmark_running(&mut self, sid: impl AsRef<str>) {
        self.state.unmark_running(sid);
    }

    /// 目的：正在生成的会话（工作列表要如实列出，虽然对象在工作线程上）。
    pub fn running_iter(&self) -> impl Iterator<Item = &SessionId> {
        self.state.running_iter()
    }

    /// 目的：遍历在世会话表（只读，工作列表与界面用）。
    pub fn sessions_iter(&self) -> impl Iterator<Item = (&SessionId, &Session)> {
        self.state.sessions_iter()
    }

    /// 目的：这份会话是否正在创建 / 启动。
    pub fn is_starting(&self, sid: impl AsRef<str>) -> bool {
        self.state.is_starting(sid)
    }

    /// 目的：占住创建位（并发建同名会话时后到的那次直接拒绝）。
    pub fn mark_starting(&mut self, sid: impl Into<String>) {
        self.state.mark_starting(sid);
    }

    /// 目的：放开创建位。
    pub fn unmark_starting(&mut self, sid: impl AsRef<str>) {
        self.state.unmark_starting(sid);
    }

    /// 目的：全会话的裁决通道表。
    pub fn desk(&self) -> &Arc<dyn crate::capabilities::session::api::Decisions> {
        self.state.desk()
    }

    /// 目的：一份会话的裁决闸：已开队直接取，未开队按 `issued` 现开一条。
    /// 参数：`issued` = 会话落盘里的裁决代号；同一条队重复取到的是同一把闸。
    pub fn desk_for(&self, sid: &str, issued: u64) -> Arc<dyn Gates> {
        self.state.desk_for(sid, issued)
    }

    /// 目的：这份会话的 shell 活进程槽（第一次用时建，之后共用）。
    pub fn shell_slot(&self, sid: &str) -> Arc<ShellSlot> {
        self.state.shell_slot(sid)
    }

    /// 目的：读会话最近一次 shell 的快照；没跑过就是 None。
    pub fn shell_view(&self, sid: &str) -> Option<ShellLive> {
        self.state.shell_view(sid)
    }
}
