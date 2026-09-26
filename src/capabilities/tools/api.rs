//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::tools::domain::fence::FenceSpec;
pub use crate::capabilities::tools::domain::module_tools::check_tools;
pub use crate::capabilities::tools::domain::patch::{apply_edits, parse, Block, EditFault, Fault};
pub use crate::capabilities::tools::domain::roles::{RoleTable, SystemTools};
pub use crate::capabilities::tools::domain::schema::{ArgFault, ToolBook, ToolSchema};
pub use crate::capabilities::tools::domain::systool::{
    arg_fault_text, execute, is_builtin, is_freeform, names, patch_decl, refuse, tool_notes,
    Observations, ToolNotes, PATCH, REPORT,
};
