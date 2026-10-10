//! 目的：机制端口——全项目共享的机制接口。
//! 管：`Log` / `ToolHandler` / `HostProbe` / `AskUser` 等端口的 trait 定义与它们的纯数据形态。
//! 不管：机制实现（文件、时间戳、目录在 `detail/`）；谁在什么时机调用（策略在调用方）；某个能力私有的端口（那是它自己的 `ports.rs`）。
//! 联动：实现见 `src/kernel/detail/`；按 R12 的例外全项目共享，不属于任何能力。

/// 目的：运行日志端口（三级）。
pub trait Log: Send + Sync {
    fn info(&self, at: &str, msg: &str);
    fn warn(&self, at: &str, msg: &str);
    fn error(&self, at: &str, msg: &str);
}

/// 目的：测试与纯逻辑场景的无声日志（不落任何盘）。
pub struct NoopLog;
impl Log for NoopLog {
    fn info(&self, _at: &str, _msg: &str) {}
    fn warn(&self, _at: &str, _msg: &str) {}
    fn error(&self, _at: &str, _msg: &str) {}
}

use crate::kernel::domain::fence::FenceSpec;
use crate::kernel::domain::types::{Ask, AskOutcome, ToolOutcome};
use std::path::Path;

/// 目的：外部进程回执里那些收尾标记的文案端口——进程机制不硬编码文案，由上层适配后注入。
/// 约束：文案从哪来不由本端口决定（提示词册在 `capabilities/prompt`，由组合根适配后注入）；
///   围栏拒绝/放行那 5 个方法只在 Windows 的容器围栏那一路用（unix 无使用点，如实放行死代码）。
#[allow(dead_code)]
pub trait ProcessTexts: Send + Sync {
    /// stderr 段的头（工具进程有 stderr 时拼在回执里）。
    fn stderr_header(&self) -> String;
    /// 超时说明（到时连根杀树后补的那句）。
    fn timeout(&self) -> String;
    /// 围栏没装上（守门进程用固定退出码报明，命令没被执行）。
    fn fence_failed(&self) -> String;
    /// 输出截断的尾部说明（chars = 原字符数，limit = 上限）。
    fn truncated(&self, chars: &str, limit: &str) -> String;
    /// 必要落点授不上且不执行时的主句（part / path / why / fix 四段）。
    fn fence_blocked(&self, part: &str, path: &str, why: &str, fix: &str) -> String;
    /// 用户明确没有放行这次调用。
    fn denied_by_user(&self) -> String;
    /// 没人答、按声明默认项收场（option = 按哪个选项办的）。
    fn no_answerer_defaulted(&self, part: &str, path: &str, option: &str) -> String;
    /// 没人答、也没声明默认项。
    fn no_answerer_refused(&self, part: &str, path: &str) -> String;
    /// 无围栏跑一次时的如实标注（part / path / why）。
    fn fence_unfenced(&self, part: &str, path: &str, why: &str) -> String;
}

/// 目的：**提问端口**——需要用户裁决的机制（围栏、工具执行层，今后任何 yes/no）经它推一条问题，
///   并**阻塞**等回答（走会话的**统一裁决通道**：同一条队、同一张卡、同一条回答命令）。
/// 约束：它是全项目共享的机制接口（R12 的例外，与 `Log` / `ToolHandler` 同一类）；
///   实现方负责**阻塞**、把用户选中的**选项 id** 原样带回，并在没有可回答的前端时按 fail-closed 收场。
pub trait AskUser: Send + Sync {
    /// 目的：把这条问题推给用户并**阻塞**等他答（不设超时——不点不继续）。
    /// 参数：`ask` = 谁在问 + 消息三段 + 选项集 + 没人答时的默认项；选项集**不得为空**（契约禁止置灰）。
    /// 返回：**这次照哪个选项办、或为什么没办**——用户答的、按声明默认项收场的、没人答的、
    ///   用户停止的、构不出可用选项的，各自如实分开（见 `AskOutcome`）；"为什么"由端口自己落进会话。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    fn ask(&self, ask: &Ask) -> AskOutcome;

    /// 目的：这一环**构不出可用选项**时的收场：**不发起裁决**，改为**停掉这个会话 + 落一条警告**。
    /// 参数：`why` = 哪一环做不下去、要补什么（请求方给的原话，写进那条警告）。
    fn halt(&self, why: &str);
}

/// 目的：一次提问的**统一入口**——有前端就问它，没前端就按发起方声明的默认项收场（默认 = 不办）。
/// 约束：声明只在**属于这张卡选项集**时才算数（不属于就按"没人答"处置，不静默改写）；
///   发起方只该用 `AskOutcome::decided` 取结果，不必逐态 match。
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub fn ask_user(port: Option<&dyn AskUser>, ask: &Ask) -> AskOutcome {
    match port {
        Some(p) => p.ask(ask),
        None => match ask.on_unanswered.as_deref().filter(|id| ask.has_option(id)) {
            Some(id) => AskOutcome::Defaulted(id.to_string()),
            None => AskOutcome::NoAnswer,
        },
    }
}

/// 目的：一类工具的执行者——**按名字认领**，不靠“内置 / 模块”的两分法。
/// 约束：成员循环只问“这一回合的工具面里有没有它、谁认领它”，三类工具走同一条派发路径；它是 R12 的例外（全项目共享）。
pub trait ToolHandler: Send + Sync {
    /// 这个名字归不归我（只看名字；模块归属另由模块工具那条路判）。
    fn owns(&self, name: &str) -> bool;
    /// 跑一次调用；上下文见 `ToolCtx`。
    fn run(&self, ctx: &ToolCtx, name: &str, args_json: &str) -> ToolOutcome;
}

/// 目的：核心自有工具的执行上下文——这一席是谁、属于哪个工作、**提交锚在哪一行**。
/// 约束：行锚让共享区提交能按转录行精确定位，回档才能把共享区物化回那一刻。
pub struct ToolCtx<'a> {
    /// 目的：顶层工作名（共享区与版本库的归属）。
    pub work: &'a str,
    /// 目的：这一席的 agent 实例名。
    pub agent: &'a str,
    /// 目的：下一条转录行的 id。
    pub line: u64,
}

pub trait HostProbe: Send + Sync {
    /// 这个路径存在且是文件。
    fn is_file(&self, path: &Path) -> bool;
    /// 这个路径存在且是目录。
    fn is_dir(&self, path: &Path) -> bool;
    /// PATH 上有没有这个可执行文件（只查存在性，不执行它；平台扩展名由适配层处理）。
    fn has_exe(&self, name: &str) -> bool;
    /// 本机能不能起硬件虚拟化（只问事实，不起任何虚拟机）。
    fn hypervisor_available(&self) -> bool;
}

/// 目的：一次外部进程执行端口——机制（围栏安装、进程拉起、stdin 送参、超时杀树、截断）在适配层。
/// 参数：`fence` = 这次执行的可达范围与网络；`command` = 模块声明的启动命令；`args_json` = 经 stdin 送进去的参数；
///   `env` = 该模块隐私字段的注入项（只经环境变量进进程，**不进命令行**）；`ask` = 这一趟的提问端口
///   （围栏必要落点授不上时经它问用户；`None` = 没有可回答的前端）。
/// 返回：一次执行的事实回执；成功与否看退出码，细节由适配层如实拼装。
/// 约束：策略（哪个模块能调哪个工具、命令行映射、可达哪些根）由调用方派生后传入；本端口只做机制。
pub trait ProcessRunner: Send + Sync {
    fn run(
        &self,
        fence: &FenceSpec,
        command: &str,
        args_json: &str,
        env: &[(String, String)],
        ask: Option<&dyn AskUser>,
    ) -> ToolOutcome;
}

/// 目的：一个**长驻**外部进程会话的规格（起常驻服务用）：围栏 + 命令 + 注入项。
/// 约束：只描述事实；管道、守门进程与杀树机制在 `SessionHost` 的实现里。
pub struct SessionSpec {
    pub fence: FenceSpec,
    pub command: String,
    pub env: Vec<(String, String)>,
}

/// 目的：一个已拉起的**长驻**进程会话——按行收发与关闭（MCP / ACP 这类按行的协议用它）。
/// 约束：`recv` 阻塞读一行；进程结束（EOF）如实返回错误，不假装拿到空行。
pub trait Session: Send {
    /// 目的：写一行（自动补换行并 flush）。
    fn send(&mut self, line: &str) -> Result<(), String>;
    /// 目的：读一行（不含行尾）；进程已结束 = 错误。
    fn recv(&mut self) -> Result<String, String>;
    /// 目的：关闭这个会话（连根杀进程；可重复调用）。
    fn kill(&mut self);
    /// 目的：这个会话此刻还活着吗（进程结束 / 管道断了 = false）；默认 true，实现按自己掌握的事实回答。
    fn alive(&self) -> bool {
        true
    }
}

/// 目的：**长驻进程**执行端口——起一个守门进程并保住它，供按行协议长跑（与一次性的 `ProcessRunner` 并列）。
pub trait SessionHost: Send + Sync {
    fn open(&self, spec: &SessionSpec) -> Result<Box<dyn Session>, String>;
}

/// 目的：围栏授权释放端口——会话删除时由核心请求一次，把该会话各 agent 的围栏授权撤掉。
/// 参数：`spec` = 该席的围栏（按 `lease` 区分同名 agent 的并发会话）。
/// 返回：撤权失败或平台限制时如实报错。
/// 约束：调用方只提出请求，不碰任何 ACL；本平台没有该机制时实现为空操作。
pub trait FenceHost: Send + Sync {
    fn release(&self, spec: &FenceSpec) -> Result<(), String>;
}
