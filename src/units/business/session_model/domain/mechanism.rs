//! 目的：会话的**机制**声明——机制是回合的身份来源，席位是它底下的具体身份组（提示词 + 机制动作）。
//! 管：`Seat` / `Mechanism` 两种声明与 `MECHANISMS` / `SEATS` 两张表，以及按 id 取机制 / 席位 / 流程清单。
//! 不管：工具面怎么合成（在 `agentloop` 的 `assemble`，按会话类型挂的工具来源各自交 id）；席位与系统工具总表的自洽断言（`problems` 在会话单元的 `seat_check`）；流程怎么调度回合（在 `collab` / `turn`）。
//! 联动：会话类型表（`profile`）引用机制 id，并声明挂哪些工具来源；`turn` 的每回合席位从这里取，工具面交给 `agentloop` 的 `assemble` 现拼。

/// 目的：一个**席位**——机制里的具体身份组：带哪段提示词、发哪些机制动作。
/// 约束：`core` 的席位由核心通道出演（不进成员表），其余由成员会话出演。文件面与模块面不在这里：前者由 agent 的能力组决定，后者由它装了哪些模块决定（见 `tools` 的 `ToolSource`）。
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
    // 会话控制这 6 个是 **system 工具**：由会话类型声明、建会话时展开进 meta（不挂在席位上）。
    tools: &[],
};

/// 目的：讨论席：只表态与核实，不动手（没有写面、没有模块工具）。
const DISCUSSANT: Seat = Seat {
    id: "discussant",
    prompt: "roles/discussant",
    min: 2,
    core: false,
    tools: &["say", "agree", "leave", "ask"],
};

/// 目的：计划席：拆节点、看节点完成、给验收结论（只读，不改文件）。
const PLANNER: Seat = Seat {
    id: "planner",
    prompt: "roles/planner",
    min: 0,
    core: true,
    tools: &["plan", "slate", "verdict"],
};

/// 目的：验收席：逐个节点判完成与清单核对（只读，不改文件）。
const ORCHESTRATOR: Seat = Seat {
    id: "orchestrator",
    prompt: "roles/orchestrator",
    min: 0,
    core: true,
    tools: &["node_verdict", "checklist"],
};

/// 目的：执行席：在自己的沙箱里干活，只有它能发布（`work_*`）并回报节点结论。
const EXECUTOR: Seat = Seat {
    id: "executor",
    prompt: "roles/executor",
    min: 1,
    core: false,
    tools: &["submit_report"],
};

/// 目的：单机席：一个 agent 直接干活（没有流程时的静止身份，与执行席同面但不报节点结论）。
const SOLO: Seat = Seat {
    id: "solo",
    prompt: "roles/executor",
    min: 0,
    core: false,
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
