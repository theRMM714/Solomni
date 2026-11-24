//! 目的：会话的**机制**声明——机制是回合的身份来源，席位是它底下的具体身份组，工具面按席位合成。
//! 管：`Seat` / `Mechanism` 两种声明与 `MECHANISMS` / `SEATS` 两张表；按席位合成工具面（`face` / `resolve`）；
//!   席位与系统工具总表的自洽断言（`problems`）。
//! 不管：工具的声明与执行（在 `tools`）；流程怎么调度回合（在 `collab` / `turn`）。
//! 联动：会话类型表（`profile`）引用机制 id；`turn` 的每回合身份与工具面都从这里取，`assemble` 按席位装配。

use crate::capabilities::tools::api::{
    DELETE, EDIT, GREP, LIST, PATCH, READ, SHELL, ToolBook, ToolSchema, WRITE,
};
use crate::capabilities::workspace::api::WORK_TOOLS;

/// 目的：一个**席位**——机制里的具体身份组：发哪些工具、能不能写、能不能发布、带哪段提示词。
/// 约束：`core` 的席位由核心通道出演（不进成员表），其余由成员会话出演；核心席不得授模块工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    /// 目的：席位 id（全局唯一）：同时是运行时动作授权里的身份名与提示词的匹配键。
    pub id: &'static str,
    /// 目的：提示词在册子里的位置（`prompts/roles/<名字>.yaml`）。
    pub prompt: &'static str,
    /// 目的：这一席至少要几个人（`0` = 核心席，由核心通道出演，不占 agent 名额）。
    pub min: usize,
    /// 目的：坐核心通道（真）还是坐成员会话（假）。
    pub core: bool,
    /// 目的：发不发写面（`write` / `edit` / `patch` / `delete`）——只能写自己的沙箱，改不到共享区。
    pub write: bool,
    /// 目的：发不发发布面（`work_*`，会改共享工作区）——这是「只读 / 可写」的唯一分野。
    pub publish: bool,
    /// 目的：发不发它自己所属模块的工具（讨论席与核心席拿不到干活的手段）。
    pub module_tools: bool,
    /// 目的：这一席独有的机制动作（机制自己声明、`tools` 提供实现与参数契约）。
    pub tools: &'static [&'static str],
}

/// 目的：一个**机制**——回合身份的来源：一组席位，跑起来是流程或静止身份。
#[derive(Debug, Clone, Copy)]
pub struct Mechanism {
    /// 目的：机制 id（流程会话按它调度；静止会话按它定位唯一一席）。
    pub id: &'static str,
    /// 目的：能不能当流程串进会话（真 = 由流程调度回合；假 = 没有流程的会话坐的静止机制）。
    pub flow: bool,
    /// 目的：这个机制的席位 id（按声明顺序；核心席与成员席都在这里）。
    pub seats: &'static [&'static str],
}

/// 目的：核心代理席：把用户的话转成对其它会话的动作（开、发、看、停）。
const CORE_PROXY: Seat = Seat {
    id: "core_proxy",
    prompt: "roles/core_proxy",
    min: 0,
    core: true,
    write: false,
    publish: false,
    module_tools: false,
    tools: &[
        "catalog_agents",
        "create_session",
        "send_session_message",
        "observe_session",
        "read_session_messages",
        "control_session",
    ],
};

/// 目的：讨论席：只表态与核实，不动手（没有写面、没有模块工具）。
const DISCUSSANT: Seat = Seat {
    id: "discussant",
    prompt: "roles/discussant",
    min: 2,
    core: false,
    write: false,
    publish: false,
    module_tools: false,
    tools: &["say", "agree", "leave", "ask"],
};

/// 目的：计划席：拆节点、看节点完成、给验收结论（只读，不改文件）。
const PLANNER: Seat = Seat {
    id: "planner",
    prompt: "roles/planner",
    min: 0,
    core: true,
    write: false,
    publish: false,
    module_tools: false,
    tools: &["plan", "slate", "verdict"],
};

/// 目的：验收席：逐个节点判完成与清单核对（只读，不改文件）。
const ORCHESTRATOR: Seat = Seat {
    id: "orchestrator",
    prompt: "roles/orchestrator",
    min: 0,
    core: true,
    write: false,
    publish: false,
    module_tools: false,
    tools: &["node_verdict", "checklist"],
};

/// 目的：执行席：在自己的沙箱里干活，只有它能发布（`work_*`）并回报节点结论。
const EXECUTOR: Seat = Seat {
    id: "executor",
    prompt: "roles/executor",
    min: 1,
    core: false,
    write: true,
    publish: true,
    module_tools: true,
    tools: &["submit_report"],
};

/// 目的：单机席：一个 agent 直接干活（没有流程时的静止身份，与执行席同面但不报节点结论）。
const SOLO: Seat = Seat {
    id: "solo",
    prompt: "roles/executor",
    min: 0,
    core: false,
    write: true,
    publish: true,
    module_tools: true,
    tools: &[],
};

/// 目的：全部席位的唯一定义处（席位 id 全局唯一：机制只引用 id，不复制声明）。
pub const SEATS: &[Seat] = &[
    CORE_PROXY,
    DISCUSSANT,
    PLANNER,
    ORCHESTRATOR,
    EXECUTOR,
    SOLO,
];

/// 目的：代理式会话坐的那一席（没有流程的会话按静止机制解析，桥接与测试造代理会话时直接用它）。
#[cfg(test)]
pub const PROXY_SEAT: &str = CORE_PROXY.id;

/// 目的：全部机制——两个能当流程用的（讨论、执行）与两个静止机制（单机、核心代理）。
pub const MECHANISMS: &[Mechanism] = &[
    Mechanism {
        id: "discuss",
        flow: true,
        seats: &["core_proxy", "discussant"],
    },
    Mechanism {
        id: "execute",
        flow: true,
        seats: &["planner", "orchestrator", "executor"],
    },
    Mechanism {
        id: "standard",
        flow: false,
        seats: &["solo"],
    },
    Mechanism {
        id: "proxy",
        flow: false,
        seats: &["core_proxy"],
    },
];

/// 目的：按 id 找一个机制；没有返回 `None`（调用方如实报错，不静默退化）。
pub fn mechanism_of(id: &str) -> Option<&'static Mechanism> {
    MECHANISMS.iter().find(|m| m.id == id)
}

/// 目的：能当流程用的机制 id（会话类型表与临时拼装只认这些）。
pub fn flow_ids() -> Vec<&'static str> {
    MECHANISMS.iter().filter(|m| m.flow).map(|m| m.id).collect()
}

/// 目的：按席位 id 找席位声明（全局唯一，机制只引用 id）。
pub fn seat(id: &str) -> Option<&'static Seat> {
    SEATS.iter().find(|s| s.id == id)
}

/// 目的：一个席位这一回合的**工具面**（id 清单，按四源并集）：
///   读面恒发（`read` / `list` / `grep` / `shell`）→ `write` 决定写面 → `publish` 决定发布面 → 机制动作。
/// 约束：读面恒发是安全的——围栏只给私有沙箱写权，`shell` 也改不到共享区；真正能改共享区的只有 `work_*`。
pub fn face(seat: &Seat) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut push = |id: &'static str| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    for id in [READ, LIST, GREP, SHELL] {
        push(id);
    }
    if seat.write {
        for id in [WRITE, EDIT, PATCH, DELETE] {
            push(id);
        }
    }
    if seat.publish {
        for id in WORK_TOOLS {
            push(id);
        }
    }
    for id in seat.tools {
        push(id);
    }
    out
}

/// 目的：把席位的工具面逐项解析成工具声明（顺序即 `face` 的顺序）——装配与会话包装只用这一个口径。
/// 错误：席位引用了总表里没有的工具（装配错误，如实报错，不静默跳过）。
pub fn resolve<'a>(
    seat: &Seat,
    book: &'a ToolBook,
) -> Result<Vec<(&'static str, &'a ToolSchema)>, String> {
    let mut out = Vec::new();
    for id in face(seat) {
        let schema = book
            .get(id)
            .ok_or_else(|| format!("席位 {} 的工具面引用了总表里没有的工具：{}", seat.id, id))?;
        out.push((id, schema));
    }
    Ok(out)
}

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
        for id in face(s) {
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
                Some(s) if !face(s).contains(&id.as_str()) => out.push(format!(
                    "动作 {} 的 callers 里有席位 {}，但它的工具面里没有这条动作",
                    id, c
                )),
                Some(_) => {}
            }
        }
        for s in SEATS {
            if face(s).contains(&id.as_str()) && !schema.callers.iter().any(|c| c == s.id) {
                out.push(format!(
                    "席位 {} 的工具面有 {}，但动作表里 {} 的 callers 没有它",
                    s.id, id, id
                ));
            }
        }
    }
    out
}
