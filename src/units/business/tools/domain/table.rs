//! 目的：系统工具总表（`systools/tools.yaml`）——每条工具的名字、说明与参数契约。
//! 管：`SystemTools`（工具总表 + 名称保留名）与 `problems()` 的名字断言。
//! 不管：谁拿到哪些工具——来源各自声明（agent / module / loop / workspace / profile），会话只收 id；
//!   授权由权限端口判定。本单元不认识席位，也不判断「该发给谁」。
//! 联动：表由 `detail/yaml_systools.rs` 读入；引擎按 id 取形状与实现。

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
    pub tools: crate::units::business::tools::api::ToolBook,
    /// 目的：agent 保留名（`systools/names.yaml` 的 `agents`）：`session/<工作>/` 下由布局固定
    ///    占用的目录名，agent 实例名不得占用。名单不硬编码在代码里，布局改了就改表。
    pub reserved_agents: Vec<String>,
    /// 目的：模块保留名（同表 `modules` 块）：模块 id 不得占用——它会成为动作 id 前缀
    ///    `module.<模块>.<工具>`。产品命令名与别名由 Op 目录在装配期并进来，不抄第二份。
    pub reserved_modules: Vec<String>,
}

impl SystemTools {
    /// 目的：总表自身的自洽检查，空 = 一切正常。
    /// 约束：每条都是机械断言（由测试门禁消费，不靠人看）：id 非空、不含空白。
    ///   能不能被谁调不在这张表里——来源交 id，授权在权限端口。
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for id in self.tools.keys() {
            if id.trim().is_empty() || id.chars().any(char::is_whitespace) {
                out.push(format!("工具 id「{}」不合法（不能为空或含空白）", id));
            }
        }
        out
    }
}
