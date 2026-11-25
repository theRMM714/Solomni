//! 目的：回合状态本体——在世表与运行态（`TurnState`）加上装配一席所需的入站面，一处持有。
//! 管：`Turn` 的字段与构造、会话在世的访问器转发、运行中文案、装配端口的取用（`session_ports`）、压缩通道工厂。
//! 不管：生成驱动、落盘与命令队列（在 `runtime`）；工具面与会话对象的拼装（在 `agentloop::api`）；纯派生规则（在 `agentloop::domain::assemble`）。
//! 联动：`runtime` 持一份 `Turn` 并经它放取会话；取事实的一侧从 `session_ports` 取入站面，再交 `agentloop` 装配。

use std::path::PathBuf;
use std::sync::Arc;

use crate::kernel::api::types::SessionId;
use crate::kernel::ports::Log;
use crate::units::agentloop::api::{CompactChatFactory, ShellLive, ShellSlot};
use crate::units::catalog::modules::api::Modules;
use crate::units::catalog::packages::api::Packages;
use crate::units::catalog::secrets::api::SecretOps;
use crate::units::llm::api::Llm;
use crate::units::prompt::api::Prompt;
use crate::units::residents::api::ResidentOps;
use crate::units::tools::api::{ToolExec, Tools};
use crate::units::turn_state::api::{Flows, History};
use crate::units::turn_state::domain::decisions::Gates;
use crate::units::turn_state::domain::live::{Session, TurnState};
use crate::units::agentloop::api::SessionPorts;
use crate::units::vm::api::Vm;
use crate::units::workspace::api::Workspace;

/// 目的：回合状态本体：在世表与运行态（`TurnState`）连同装配一席要用的入站面，一处持有。
/// 约束：只在核心线程上读写；入站面由 `session_ports` 交出——回合驱动与重建在本单元外（`turn`），
///   故状态表与端口按 crate 可见，新的直取点必须同次说明理由。
pub struct Turn {
    /// 目的：在世会话表、运行态与创建占位、裁决队、每会话的 shell 活进程槽。
    pub(crate) state: TurnState,
    /// 目的：工作区用例面：一席的 `work_*` 工具执行者按它造。
    pub(crate) workspace: Arc<dyn Workspace + Send + Sync>,
    /// 目的：虚拟机用例面：一席的 guest 事实随工具面交给机制层。
    pub(crate) vm: Arc<dyn Vm + Send + Sync>,
    /// 目的：常驻服务管理面：已就绪服务的操作并入一席的工具面。
    pub(crate) residents: Arc<dyn ResidentOps + Send + Sync>,
    /// 目的：隐秘字段管理面：装配期按模块解析注入项。
    pub(crate) secrets: Arc<dyn SecretOps + Send + Sync>,
    /// 目的：工具执行面：一席的工具环境要它跑外部 / 内置工具。
    pub(crate) tools: Arc<dyn ToolExec + Send + Sync>,
    /// 目的：工具总表与机制席位：发放名单与内置工具表从它读。
    pub(crate) systools: Arc<dyn Tools>,
    /// 目的：提示词册：一席的工具说明与压缩提示从它取段。
    pub(crate) prompt: Arc<dyn Prompt>,
    /// 目的：llm 用例面：一席的收发句柄按它造。
    pub(crate) llm: Arc<dyn Llm + Send + Sync>,
    /// 目的：日志面：装配时记一条「单 agent 工作」。
    pub(crate) log: Arc<dyn Log + Send + Sync>,
    /// 目的：压缩通道工厂：新会话与重建会话共用同一份。
    pub(crate) compact: Arc<dyn CompactChatFactory>,
    /// 目的：项目内基础工具链的只读安装根（python / node / cc），由组合根注入。
    pub(crate) base_roots: Vec<PathBuf>,
    /// 目的：产品根：外部工作区绑定校验与 guest 宿主翻译都以它为锚。
    pub(crate) root: PathBuf,
    /// 目的：会话落盘与名录：重建会话、准入与派发闸从它读元数据与事件。
    pub(crate) history: Arc<dyn History + Send + Sync>,
    /// 目的：模块登记：重建会话按清单复原一席的模块与目录。
    pub(crate) modules: Arc<dyn Modules + Send + Sync>,
    /// 目的：包管理面：重建会话时按档位取库根与基础包根。
    pub(crate) packages: Arc<dyn Packages + Send + Sync>,
    /// 目的：流程工厂（组合根注入）：重建会话按会话类型表里的流程序列逐段造，本层不认识任何具体流程。
    pub(crate) flows: Arc<dyn Flows>,
}

/// 目的：`Turn` 的构造与取用。
impl Turn {
    /// 目的：造一份回合状态本体：回合状态随之就绪，入站面与基础工具链根由组合根注入。
    // 回合状态本体把"装配一席要的入站面"一次说全；收口成参数对象只是把参数挪个地方。
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
        flows: Arc<dyn Flows>,
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
            flows,
        }
    }

    /// 目的：换掉基础工具链的只读安装根（组合根读到 active.json 后调用）。
    pub fn set_base_roots(&mut self, roots: Vec<PathBuf>) {
        self.base_roots = roots;
    }

    /// 目的：装配一席要用的入站面（端口对象）——按 `Turn` 现持的那几份克隆出来，交 `session` 的装配入口。
    pub fn session_ports(&self) -> SessionPorts {
        SessionPorts {
            llm: Arc::clone(&self.llm),
            log: Arc::clone(&self.log),
            prompt: Arc::clone(&self.prompt),
            systools: Arc::clone(&self.systools),
            tools: Arc::clone(&self.tools),
            vm: Arc::clone(&self.vm),
            residents: Arc::clone(&self.residents),
            workspace: Arc::clone(&self.workspace),
            compact: Arc::clone(&self.compact),
        }
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
    pub fn desk(&self) -> &Arc<dyn crate::units::turn_state::api::Decisions> {
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
