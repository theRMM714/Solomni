//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::tools::domain::fence::FenceSpec;
pub use crate::capabilities::tools::domain::patch::{apply_edits, parse, Block, EditFault, Fault};
pub use crate::capabilities::tools::domain::roles::{RoleTable, SystemTools};
pub use crate::capabilities::tools::domain::schema::{ArgFault, ToolBook, ToolSchema};
pub use crate::capabilities::tools::domain::systool::{
    arg_fault_text, execute, is_builtin, is_freeform, names, patch_decl, refuse, tool_notes,
    Observations, ToolNotes, PATCH, REPORT,
};

/// 工具总表与角色表的**能力面**：别的能力只问"这一席能用哪些工具"，**看不见两张表的字段**。
///
/// 表本体（`SystemTools`）是纯数据、没有端口；组合根装载一次，`core` 只持 `Arc<dyn Tools>`
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

    /// 悬空引用与缺能力（空 = 一切正常）：装配期自检与测试门禁读它。
    fn problems(&self) -> Vec<String>;
}
