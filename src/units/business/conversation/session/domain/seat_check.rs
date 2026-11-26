//! 目的：机制与席位声明同系统工具总表的自洽断言——它是装配期与测试门禁的唯一判据。
//! 管：`problems`（席位 / 机制 / 工具面三者的自洽检查）。
//! 不管：机制与席位声明本身（在 `session_model/domain/mechanism.rs`）；工具总表（在 `tools`）；工具面怎么合成（在 `assemble`）。
//! 联动：`main.rs` 装配期调它把启动失败挡在最前面；测试门禁按它判席位与动作表是否同步（经会话单元的 `api::mechanism_problems`）。

use crate::units::business::session_model::api::{MECHANISMS, SEATS, seat};
use crate::units::business::tools::api::ToolBook;

/// 目的：机制与席位声明的自洽检查，空 = 一切正常。
/// 约束：每条都是机械断言（由装配期与测试门禁消费）。
///   1) 席位 id 全局唯一、提示词非空；2) 机制 id 唯一、席位都在册、流程机制核心席与成员席都有、静止机制恰好一席；
///   3) 席位声明的每个机制动作都在总表里（谁能拿到哪条工具不在这里比对：来源各自交 id，授权归权限端口）。
pub fn problems(book: &ToolBook) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for s in SEATS {
        if seen.contains(&s.id) {
            out.push(format!("席位 id 重复：{}", s.id));
        }
        seen.push(s.id);
        if s.prompt.trim().is_empty() {
            out.push(format!("席位 {} 没声明提示词", s.id));
        }
        for id in s.tools {
            if !book.contains_key(*id) {
                out.push(format!(
                    "席位 {} 的工具面引用了不存在的系统工具：{}",
                    s.id, id
                ));
            }
        }
    }
    let mut mids: Vec<&str> = Vec::new();
    for m in MECHANISMS {
        if mids.contains(&m.id) {
            out.push(format!("机制 id 重复：{}", m.id));
        }
        mids.push(m.id);
        if m.seats.is_empty() {
            out.push(format!("机制 {} 一个席位都没声明", m.id));
        }
        let mut has_core = false;
        let mut has_member = false;
        for id in m.seats {
            match seat(id) {
                None => out.push(format!("机制 {} 引用了不存在的席位：{}", m.id, id)),
                Some(s) if s.core => has_core = true,
                Some(_) => has_member = true,
            }
        }
        if m.flow && !(has_core && has_member) {
            out.push(format!("流程机制 {} 要同时有核心席与成员席", m.id));
        }
        if !m.flow && m.seats.len() != 1 {
            out.push(format!(
                "静止机制 {} 只能有一个席位（它是没有流程的会话的静止身份）",
                m.id
            ));
        }
    }
    out
}
