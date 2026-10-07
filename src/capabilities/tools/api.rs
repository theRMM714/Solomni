//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::tools::domain::fence::FenceSpec;
pub use crate::capabilities::tools::domain::patch::{EditFault, Fault};
pub use crate::capabilities::tools::domain::roles::{action_audit, RoleTable, SystemTools};
pub use crate::capabilities::tools::domain::schema::{ArgFault, ToolBook, ToolSchema};
pub use crate::capabilities::tools::domain::systool::{
    arg_fault_text, is_builtin, is_freeform, names, patch_decl, refuse, tool_notes, Observations,
    ToolNotes, ToolOutcome, PATCH, REPORT,
};

/// 工具总表与角色表的**能力面**：别的能力只问"这一席能用哪些工具"，**看不见两张表的字段**。
///
/// 表本体（`SystemTools`）是纯数据、没有端口；组合根装载一次，`conductor` 只持 `Arc<dyn Tools>`
/// 并与协作会话共享（见 `service.rs`）。
pub trait Tools: Send + Sync {
    /// 按角色组装工具面：角色引用的 id 逐个解析成工具声明（顺序即角色表里的顺序）；
    /// 未知名**如实报错**（悬空引用是装配错误）。
    fn tool_face(&self, role: &str) -> Result<Vec<(&str, &ToolSchema)>, String>;

    /// 一个角色这一回合的**工具面**（id 清单 + 是否给它自己模块的工具）。
    fn role_face(&self, role: &str) -> (Vec<String>, bool);

    /// 这个身份能不能用它自己模块的工具（论据：角色表的 `module_tools`）。
    fn allows_module_tools(&self, role: &str) -> bool;

    /// 内置工具总表（工具**是什么**的声明书）：调用方要持有它时拿走一份。
    fn book(&self) -> ToolBook;

    /// 目录保留名（`systools/names.yaml`）：`session/<工作>/` 下由布局固定占用的目录名，
    /// agent 实例名不得占用。名单在表里，不在代码里。
    fn reserved_names(&self) -> Vec<String>;

    /// 悬空引用与缺能力（空 = 一切正常）：装配期自检与测试门禁读它。
    fn problems(&self) -> Vec<String>;
}

/// 工具能力的**执行面**（`service.rs` 实现）：别的能力要执行工具、要释放围栏授权，走这里；
/// 三个出站端口（`ToolRunner` / `SysIo` / `FenceHost`）**只由它持有**（R12）。
///
/// 为什么 `SysIo` 的读写不在这里：它只被本能力自己的 domain（内置工具实现）用，
/// 别人要的是"跑一个工具"，不是"按路径读写文件"。
pub trait ToolExec: Send + Sync {
    /// 执行一次**外部工具**（模块声明的那种）：围栏安装、守门进程、超时杀树、截断都在端口后面。
    fn run_module(
        &self,
        fence: &crate::capabilities::tools::api::FenceSpec,
        command: &str,
        args_json: &str,
    ) -> ToolOutcome;

    /// 执行一次**内置工具**：放行、寻址、参数校验在本能力的 domain，机制在 `SysIo` 后面。
    fn run_builtin(
        &self,
        sb: &crate::capabilities::workspace::api::Sandbox,
        builtin_tools: &ToolBook,
        observations: &mut Observations,
        name: &str,
        args_json: &str,
    ) -> ToolOutcome;

    /// 会话删除时请求一次：把该会话各 agent 的围栏授权撤掉（调用方只提出请求，不碰任何 ACL）。
    fn release_fence(
        &self,
        spec: &crate::capabilities::tools::api::FenceSpec,
    ) -> Result<(), String>;
}
