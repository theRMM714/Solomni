//! 目的：执行端口——一席的执行环境只回答「把这条命令跑掉，回执拿回来」，本机与客机的差别全在实现里。
//! 管：`ExecKind`（shell / 模块工具）、`ExecRequest`（cwd、围栏、超时、预算、取消与提问口）、`ExecOutcome`、`Sandbox` trait。
//! 不管：围栏怎么派生、命令怎么拼（调用方）；具体怎么跑（本机在 `tools`，客机在 `vm`）。
//! 联动：`agentloop` 的引擎按它跑 shell 与模块工具，因此不认识 vm、也不认识进程执行面。

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::kernel::api::budget::ResultBudget;
use crate::kernel::api::fence::FencePlan;
use crate::kernel::api::types::ToolOutcome;
use crate::kernel::ports::AskUser;

/// 目的：这次要跑的是什么——一条 shell，或一个模块工具（模块自己声明的那条命令）。
/// 约束：不认识的种类加变体即可，引擎只按端口调用、不按种类分支。
pub enum ExecKind<'a> {
    /// 目的：一条 shell 脚本（本机档走本机进程，客机档走 guest 通道）。
    Shell {
        /// 目的：要跑的脚本。
        script: &'a str,
    },
    /// 目的：一个模块工具——命令由模块声明给，参数是信封里的原始 JSON。
    Module {
        /// 目的：模块 id。
        module: &'a str,
        /// 目的：模块声明的命令模板（已展开）。
        command: &'a str,
        /// 目的：参数 JSON（原样透传，参数契约已由调用方校验）。
        args_json: &'a str,
        /// 目的：该模块要给的环境变量。
        env: &'a [(String, String)],
        /// 目的：这个工具是不是可以并行（实现方决定要不要排队）。
        parallel: bool,
    },
}

/// 目的：一次执行的请求——围栏已派生好，端口只负责如实跑掉。
pub struct ExecRequest<'a> {
    /// 目的：要跑什么。
    pub kind: ExecKind<'a>,
    /// 目的：工作目录（宿主路径；客机侧由实现自己译）。
    pub cwd: &'a Path,
    /// 目的：这次的围栏意图（路径授予、网络、租约）。
    pub fence: FencePlan,
    /// 目的：超时。
    pub timeout: Duration,
    /// 目的：要用户点头时的提问口（围栏装不上时由实现自己问）。
    pub ask: Option<&'a dyn AskUser>,
    /// 目的：取消开关。
    pub cancel: Option<&'a AtomicBool>,
    /// 目的：结果预算（回执截断由调用方照旧做）。
    pub budget: ResultBudget,
}

/// 目的：一次执行的结果——回执，加上这次装围栏花掉的时间（可观测用）。
pub struct ExecOutcome {
    /// 目的：工具回执。
    pub outcome: ToolOutcome,
    /// 目的：装围栏的总耗时（秒），没装就是 None。
    pub fence_prep: Option<Duration>,
}

impl ExecOutcome {
    /// 目的：只要回执、不记围栏耗时（客机侧的通道自己管装填）。
    pub fn plain(outcome: ToolOutcome) -> ExecOutcome {
        ExecOutcome {
            outcome,
            fence_prep: None,
        }
    }
}

/// 目的：一席的执行句柄——按请求跑掉，别的什么都不回答。
/// 约束：实现必须**先自检、再执行**：`precheck` 返回的那句话由调用方原样作为拒绝回执，不产生副作用。
/// 目的：开不出执行面时的兜底句柄——**什么都不跑**，把开不出来的理由原样当拒绝回执。
/// 约束：装配不因为一个策略开不出来就整场失败；一调用就拒绝，理由要能让人看出是哪个策略的哪条限制。
pub struct Refusing {
    why: String,
}

impl Refusing {
    /// 目的：用一个理由造兜底句柄（这个理由会在每次调用时回到模型面前）。
    pub fn new(why: String) -> Self {
        Self { why }
    }
}

impl Sandbox for Refusing {
    fn precheck(&self, _req: &ExecRequest<'_>) -> Option<String> {
        Some(self.why.clone())
    }

    fn run(&self, _req: &ExecRequest<'_>) -> ExecOutcome {
        ExecOutcome::plain(crate::kernel::api::types::ToolOutcome::new(
            false,
            self.why.clone(),
        ))
    }
}

/// 目的：按理由造一个兜底执行面（策略开不出来时装配机用它，不打断整场会话）。
pub fn refused(why: String) -> std::sync::Arc<dyn Sandbox> {
    std::sync::Arc::new(Refusing::new(why))
}
pub trait Sandbox: Send + Sync {
    /// 目的：这次请求在自己这一侧根本办不到时的理由（空 = 能办）；自检不执行任何命令。
    fn precheck(&self, req: &ExecRequest<'_>) -> Option<String> {
        let _ = req;
        None
    }

    /// 目的：把这次请求跑掉。
    fn run(&self, req: &ExecRequest<'_>) -> ExecOutcome;
}
