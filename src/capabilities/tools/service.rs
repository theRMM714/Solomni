//! 工具总表与角色表的**能力面实现与装配入口**。
//!
//! 状态就是 `domain` 的 `SystemTools`（**纯数据，没有端口**）：装载一次之后只答不问，
//! 所以这里不再包一层结构体，直接把 `api::Tools` 挂在它身上——**持有者因此全进程只有一处**
//! （core 的 `Arc<dyn Tools>`），协作会话与它共享同一份（批次 17）。
//!
//! 加载机制在 `detail/yaml_systools.rs`；本文件不碰文件系统。

use crate::capabilities::tools::api::{ToolBook, ToolSchema, Tools};
use crate::capabilities::tools::domain::roles::SystemTools;
use crate::capabilities::tools::ports::SystoolsSource;
use std::sync::Arc;

impl Tools for SystemTools {
    fn tool_face(&self, role: &str) -> Result<Vec<(&str, &ToolSchema)>, String> {
        SystemTools::tool_face(self, role)
    }

    fn role_face(&self, role: &str) -> (Vec<String>, bool) {
        SystemTools::role_face(self, role)
    }

    fn allows_module_tools(&self, role: &str) -> bool {
        SystemTools::allows_module_tools(self, role)
    }

    fn book(&self) -> ToolBook {
        self.tools.clone()
    }

    fn problems(&self) -> Vec<String> {
        SystemTools::problems(self)
    }
}

/// 组合根专用：装载两张表（缺文件 / 缺键 = 装配错误，如实报错）。
pub fn load(source: &dyn SystoolsSource) -> Result<Arc<dyn Tools>, String> {
    Ok(Arc::new(source.load()?))
}
