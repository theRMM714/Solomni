//! 目的：机制与席位声明同系统工具总表的自洽断言——它是装配期与测试门禁的唯一判据。
//! 管：`problems`（席位 / 机制 / 工具面三者的双向一致性检查）。
//! 不管：机制与席位声明本身（在 `session_model/domain/mechanism.rs`）；工具总表（在 `tools`）；工具面怎么合成（在 `assemble`）。
//! 联动：`main.rs` 装配期调它把启动失败挡在最前面；测试门禁按它判席位与动作表是否同步（经会话单元的 `api::mechanism_problems`）。

use crate::units::agentloop::api::seat_face;
use crate::units::session_model::api::{MECHANISMS, SEATS, seat};
use crate::units::tools::api::ToolBook;

/// 目的：机制与席位声明同系统工具总表的自洽检查，空 = 一切正常。
/// 约束：每条都是机械断言（由装配期与测试门禁消费）。
///   1) 席位 id 全局唯一、提示词非空；2) 机制 id 唯一、席位都在册、流程机制核心席与成员席都有、静止机制恰好一席；
///   3) 核心席不授模块工具；4) 能发布必须先能写；5) 席位面的每个工具都在总表里；
///   6) `callers`（谁能调）与席位面是**同一关系的两面**——双向比对，任何一边漏写 / 多写都当场报错。
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
        if s.core && s.module_tools {
            out.push(format!("席位 {} 是核心席，不得授模块工具", s.id));
        }
        if s.publish && !s.write {
            out.push(format!(
                "席位 {} 能发布却不能写：发布要先能写自己的沙箱",
                s.id
            ));
        }
        for id in seat_face(s) {
            if !book.contains_key(id) {
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
    for (id, schema) in book {
        for c in &schema.callers {
            if c == "user" || c.starts_with("user:") {
                continue;
            }
            match seat(c) {
                None => out.push(format!(
                    "动作 {} 的 callers 里有不认识的席位：{}（只能是席位 id 或 user / user:…）",
                    id, c
                )),
                Some(s) if !seat_face(s).contains(&id.as_str()) => out.push(format!(
                    "动作 {} 的 callers 里有席位 {}，但它的工具面里没有这条动作",
                    id, c
                )),
                Some(_) => {}
            }
        }
        for s in SEATS {
            if seat_face(s).contains(&id.as_str()) && !schema.callers.iter().any(|c| c == s.id) {
                out.push(format!(
                    "席位 {} 的工具面有 {}，但动作表里 {} 的 callers 没有它",
                    s.id, id, id
                ));
            }
        }
    }
    out
}
