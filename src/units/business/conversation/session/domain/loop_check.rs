//! 目的：流程段与角色声明同系统工具总表的自洽断言——它是装配期与测试门禁的唯一判据。
//! 管：`problems`（角色 / 流程段 / 角色位 / 工具面四者的自洽检查）。
//! 不管：角色与流程段声明本身（在 `session_model/domain/mechanism.rs`）；工具总表（在 `tools`）；工具面怎么合成（在 `assemble`）。
//! 联动：`main.rs` 装配期调它把启动失败挡在最前面；测试门禁按它判角色与动作表是否同步（经会话单元的 `api::loop_problems`）。

use crate::units::business::session_model::api::{LOOPS, ROLES, role};
use crate::units::business::tools::api::ToolBook;

/// 目的：流程段与角色声明的自洽检查，空 = 一切正常。
/// 约束：每条都是机械断言（由装配期与测试门禁消费）——角色 id 全局唯一且提示词非空；流程段 id 唯一、角色位不为空、角色位引用的角色都在册、流程段要同时有核心角色与成员角色、静止段恰好一个角色位；角色位声明的每个机制动作都在总表里（谁能拿到哪条工具不在这里比对：来源各自交 id，授权归权限端口）。
pub fn problems(book: &ToolBook) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for r in ROLES {
        if seen.contains(&r.id) {
            out.push(format!("角色 id 重复：{}", r.id));
        }
        seen.push(r.id);
        if r.prompt.trim().is_empty() {
            out.push(format!("角色 {} 没声明提示词", r.id));
        }
    }
    let mut lids: Vec<&str> = Vec::new();
    for l in LOOPS {
        if lids.contains(&l.id) {
            out.push(format!("流程段 id 重复：{}", l.id));
        }
        lids.push(l.id);
        if l.cast.is_empty() {
            out.push(format!("流程段 {} 一个角色位都没声明", l.id));
        }
        let mut has_core = false;
        let mut has_member = false;
        for c in l.cast {
            match role(c.role) {
                None => out.push(format!("流程段 {} 引用了不存在的角色：{}", l.id, c.role)),
                Some(r) if r.core => has_core = true,
                Some(_) => has_member = true,
            }
            for id in c.tools {
                if !book.contains_key(*id) {
                    out.push(format!(
                        "流程段 {} 的角色位 {} 引用了不存在的系统工具：{}",
                        l.id, c.role, id
                    ));
                }
            }
        }
        if l.flow && !(has_core && has_member) {
            out.push(format!("流程段 {} 要同时有核心角色与成员角色", l.id));
        }
        if !l.flow && l.cast.len() != 1 {
            out.push(format!(
                "静止段 {} 只能有一个角色位（它是没有流程的会话的静止身份）",
                l.id
            ));
        }
    }
    out
}
