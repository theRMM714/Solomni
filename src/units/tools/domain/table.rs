//! 目的：系统工具总表（`systools/tools.yaml`）——每条工具是什么、谁能调它、它会不会改共享工作区。
//! 管：`SystemTools`（工具总表 + 名称保留名）与 `problems()` 的自洽断言（能力标签、调用者、发布面）。
//! 不管：谁拿到哪些工具——机制与席位的声明在会话能力（`session::domain::mechanism`），面由它按总表合成；
//!   本能力不认识席位，也就不存在「按席位发面」这一层。
//! 联动：表由 `detail/yaml_systools.rs` 读入；`session` 的机制声明按总表解析席位面；`runtime` 的动作授权读 `callers`。

/// 目的：一次动作的**审计记录**（同一形状：谁、调了什么、成没成）。
/// 约束：人经呈现层的动作与模型经工具调用的同一条动作共用它——审计只有一处格式化。
pub fn action_audit(caller: &str, action: &str, ok: bool, detail: &str) -> String {
    let tail = if ok {
        "成功".to_string()
    } else {
        format!("失败：{}", detail)
    };
    format!("[动作] {} 调 {}：{}", caller, action, tail)
}

/// 目的：系统工具总表与命名保留名（装配期读 `systools/` 得来）。
#[derive(Debug, Clone, Default)]
pub struct SystemTools {
    pub tools: crate::units::tools::api::ToolBook,
    /// 目的：agent 保留名（`systools/names.yaml` 的 `agents`）：`session/<工作>/` 下由布局固定
    ///    占用的目录名，agent 实例名不得占用。名单不硬编码在代码里，布局改了就改表。
    pub reserved_agents: Vec<String>,
    /// 目的：模块保留名（同表 `modules` 块）：模块 id 不得占用——它会成为动作 id 前缀
    ///    `module.<模块>.<工具>`。产品命令名与别名由 Op 目录在装配期并进来，不抄第二份。
    pub reserved_modules: Vec<String>,
}

impl SystemTools {
    /// 目的：总表自身的自洽检查，空 = 一切正常。
    /// 约束：每条都是机械断言（由测试门禁消费，不靠人看）：每条工具都有能力标签、都声明了调用者；
    ///   `publish` 与工作区的动作面一一对应（能改共享区的只有 `work_*`）；
    ///   谁能调谁、席位发什么，由会话能力的 `mechanism::problems` 按席位面双向比对。
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, schema) in &self.tools {
            if schema.capability.trim().is_empty() {
                out.push(format!("工具 {} 没声明 capability（权限标签）", id));
            }
            if schema.callers.is_empty() {
                out.push(format!("动作 {} 没声明 callers（谁能调它）", id));
            }
            if schema.publish != crate::units::workspace::api::is_work_tool(id) {
                out.push(format!(
                    "动作 {} 的发布面标记与工作区的动作面不一致（publish 只能是 work_*）",
                    id
                ));
            }
        }
        out
    }
}
