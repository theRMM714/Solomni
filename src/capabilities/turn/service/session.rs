//! 目的：会话本体的**形状**——`Turn` 持五张内存表 + 一席装配要的能力面，是"会话在世"这件事的唯一持有者。
//! 管：`Turn` 的字段与构造（`new`）、对五张表的同名转发访问器、`session_deps` / `compact_factory` 两个装配注入、
//!   `running_refusal`（生成中拒绝的统一文案）。
//! 不管：会话怎么装配与重建（`assemble.rs` / `rebuild.rs`）、取事实（`facts.rs` / `plan.rs` / `tree.rs`）、
//!   落盘与生成驱动（仍在 `conductor`）。
//! 联动：`conductor` 持一整个 `Turn` 并直连它的方法；单元地图见 docs/turn/unit-map.md。

use std::path::PathBuf;
use std::sync::Arc;

use crate::capabilities::llm::api::Llm;
use crate::capabilities::modules::api::Modules;
use crate::capabilities::packages::api::Packages;
use crate::capabilities::prompt::api::Prompt;
use crate::capabilities::residents::api::ResidentOps;
use crate::capabilities::secrets::api::SecretOps;
use crate::capabilities::session::api::{
    CompactChatFactory, DecisionDesk, History, ShellLive, ShellSlot,
};
use crate::capabilities::tools::api::{ToolExec, Tools};
use crate::capabilities::turn::api::Session;
use crate::capabilities::turn::service::assemble::SessionDeps;
use crate::capabilities::turn::service::state::TurnState;
use crate::capabilities::vm::api::Vm;
use crate::capabilities::workspace::api::Workspace;
use crate::kernel::api::SessionId;
use crate::kernel::ports::Log;

/// 目的：**会话本体**：会话在世表 + 一席装配要的能力面（组合根注入一次，核心线程独占）。
///  为什么整块搬来这里：在世表协议、装配、从落盘重建、树序与身份都是"会话"这件事本身——
///  它们只读会话能力与其它能力面，与"命令队列 / 生成驱动"无关（见 ARCHITECTURE.md §一 turn 行）。
/// 约束：`turn` 不落盘、不引 `conductor`；登记处与宿主探测按需传参（`registry_mut` 要 `&mut`，共享会逼出 `RefCell`）。
pub struct Turn {
    /// 目的：产品根（虚拟根这类"装在项目内"的东西按它解析成绝对路径）。
    pub(crate) root: PathBuf,
    /// 目的：五张内存表（在世表 / 运行态 / 占名表 / 裁决队句柄 / shell 槽）。
    state: TurnState,
    /// 目的：会话历史直连面（造/读/写/删会话都走它；**不持它的端口**，R12）。
    pub(crate) history: Arc<dyn History + Send + Sync>,
    /// 目的：工作区用例面（工作区目录与版本化走它）。
    pub(crate) workspace: Arc<dyn Workspace + Send + Sync>,
    /// 目的：虚拟机档用例面（guest 生命周期与档位就绪检测走它）。
    pub(crate) vm: Arc<dyn Vm + Send + Sync>,
    /// 目的：运行包能力（包库、选型诊断与计划走它）。
    pub(crate) packages: Arc<dyn Packages + Send + Sync>,
    /// 目的：模块能力面——模块清单事实的唯一上游。
    pub(crate) modules: Arc<dyn Modules + Send + Sync>,
    /// 目的：常驻服务的统一管理面（装配期按已就绪服务填工具面）。
    pub(crate) residents: Arc<dyn ResidentOps + Send + Sync>,
    /// 目的：隐秘字段的统一管理面（装配期按模块解析注入项并脱敏）。
    pub(crate) secrets: Arc<dyn SecretOps + Send + Sync>,
    /// 目的：工具执行面（跑外部/内置工具、释放围栏授权都走它）。
    pub(crate) tools: Arc<dyn ToolExec + Send + Sync>,
    /// 目的：工具总表与角色表的能力面（装配期按角色发放工具面）。
    pub(crate) systools: Arc<dyn Tools>,
    /// 目的：提示词册能力（装配期按名字取段；与协作会话共享同一份）。
    pub(crate) prompt: Arc<dyn Prompt>,
    /// 目的：llm 用例面（按解析出来的通道造收发句柄）。
    pub(crate) llm: Arc<dyn Llm + Send + Sync>,
    /// 目的：日志端口（装配期记一行"单 agent 工作"的事实）。
    pub(crate) log: Arc<dyn Log + Send + Sync>,
    /// 目的：压缩通道工厂（会话在压缩触发时现取，不刷新会话）。
    pub(crate) compact: Arc<dyn CompactChatFactory>,
    /// 目的：项目内基础工具链的只读安装根（宿主档放行；VM 档由 guest 只读挂载提供）。
    pub(crate) base_roots: Vec<PathBuf>,
}

impl Turn {
    /// 目的：装配会话本体：五张表一律空着起步，能力面按组合根注入的那一份共享。
    /// 参数：`root` = 产品根；其余为各能力面；`base_roots` = 基础工具链安装根（可后置补）。
    /// 返回：可直接插进 `conductor` 的会话本体。
    // 组合根注入的构造函数：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        history: Arc<dyn History + Send + Sync>,
        workspace: Arc<dyn Workspace + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        packages: Arc<dyn Packages + Send + Sync>,
        modules: Arc<dyn Modules + Send + Sync>,
        residents: Arc<dyn ResidentOps + Send + Sync>,
        secrets: Arc<dyn SecretOps + Send + Sync>,
        tools: Arc<dyn ToolExec + Send + Sync>,
        systools: Arc<dyn Tools>,
        prompt: Arc<dyn Prompt>,
        llm: Arc<dyn Llm + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        compact: Arc<dyn CompactChatFactory>,
    ) -> Turn {
        Turn {
            root,
            state: TurnState::new(),
            history,
            workspace,
            vm,
            packages,
            modules,
            residents,
            secrets,
            tools,
            systools,
            prompt,
            llm,
            log,
            compact,
            base_roots: Vec::new(),
        }
    }

    /// 目的：给会话本体注入基础工具链的只读安装根（组合根读到 active.json 后调用）。
    pub(crate) fn set_base_roots(&mut self, roots: Vec<PathBuf>) {
        self.base_roots = roots;
    }

    /// 目的：一席会话装配要的十个能力面——装配构件按它取用，不碰端口。
    pub(crate) fn session_deps(&self) -> SessionDeps {
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
    pub(crate) fn compact_factory(&self) -> Arc<dyn CompactChatFactory> {
        Arc::clone(&self.compact)
    }

    /// 目的：生成期间"会话不在表里"的三种进入点共用这一句（错误文案要一致，别处不再各写一份）。
    /// 参数：`sid` = 会话标识。
    /// 返回：给用户看的拒绝文案。
    /// 约束：文案是用户可见契约，改动要同步 `docs/` 里对应描述。
    pub(crate) fn running_refusal(sid: &str) -> String {
        format!("会话 {} 正在生成中：先「停止」或等它结束，再做这一步", sid)
    }

    /// 目的：取这个会话的实例（不在世、或对象已交给工作线程时为 `None`）。
    pub fn session(&self, sid: &str) -> Option<&Session> {
        self.state.session(sid)
    }

    /// 目的：取这个会话的可变实例（核心线程独占，改完即生效）。
    pub fn session_mut(&mut self, sid: &str) -> Option<&mut Session> {
        self.state.session_mut(sid)
    }

    /// 目的：这个会话是否在世（对象交给工作线程期间如实返回 false）。
    pub fn holds(&self, sid: &str) -> bool {
        self.state.holds(sid)
    }

    /// 目的：遍历在世的会话。
    pub fn iter(&self) -> impl Iterator<Item = (&SessionId, &Session)> {
        self.state.iter()
    }

    /// 目的：把会话放进在世表；同 id 已在世时覆盖。
    pub fn put(&mut self, sid: &str, s: Session) {
        self.state.put(sid, s);
    }

    /// 目的：把会话从在世表取出（交出给工作线程走这条）。
    pub(crate) fn take_session(&mut self, sid: &str) -> Option<Session> {
        self.state.take(sid)
    }

    /// 目的：这个会话的回合是否还攥在生成线程手里。
    pub fn is_running(&self, sid: &str) -> bool {
        self.state.is_running(sid)
    }

    /// 目的：标记这个会话的回合已交给生成线程。
    pub fn mark_running(&mut self, sid: &str) {
        self.state.mark_running(sid);
    }

    /// 目的：解除这个会话的运行态（回合交回或中止时调）。
    pub fn unmark_running(&mut self, sid: &str) {
        self.state.unmark_running(sid);
    }

    /// 目的：遍历正在生成的会话 id。
    pub fn running_iter(&self) -> impl Iterator<Item = &SessionId> {
        self.state.running_iter()
    }

    /// 目的：这些会话里是否有任何一个的回合还攥在生成线程手里。
    pub fn any_running(&self, ids: &[String]) -> bool {
        self.state.any_running(ids)
    }

    /// 目的：这个会话名是否已被一次创建占住（创建分两段，慢的那一段在调用方线程上跑）。
    pub fn is_starting(&self, name: &str) -> bool {
        self.state.is_starting(name)
    }

    /// 目的：占住一个会话名：同名并发创建因此不会都通过「名字不存在」那道检查而互相覆盖。
    pub fn mark_starting(&mut self, name: &str) {
        self.state.mark_starting(name);
    }

    /// 目的：放掉创建占名（创建成功或失败时都要调，别让名字一直占着）。
    pub fn unmark_starting(&mut self, name: &str) {
        self.state.unmark_starting(name);
    }

    /// 目的：裁决队句柄：核心各关卡与工具级确认共用同一条队。
    pub fn desk(&self) -> &DecisionDesk {
        self.state.desk()
    }

    /// 目的：取（必要时创建）一个会话的 shell 实时状态槽；成员工具面与状态栏共用同一份。
    pub fn shell_slot(&self, sid: &str) -> Arc<ShellSlot> {
        self.state.shell_slot(sid)
    }

    /// 目的：读会话最近一次 shell 的快照（没运行过就是 `None`）。
    pub fn shell_view(&self, sid: &str) -> Option<ShellLive> {
        self.state.shell_view(sid)
    }
}
