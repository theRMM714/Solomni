//! 目的：会话的**流程段**（loop）与**角色**声明——一段流程自带它的角色位与机制动作，工具面因此跟段走、不跟全局席位走。
//! 管：`Role` / `Cast` / `Loop` 三种声明与 `ROLES` / `LOOPS` 两张表，以及按 id 取角色 / 段 / 某个段里的某个角色位。
//! 不管：工具面怎么合成（在 `agentloop` 的 `assemble`，按会话类型挂的工具来源各自交 id）；会话类型表（`profile` 引用段 id）。
//! 联动：`profile` 的会话类型声明一串流程段或一个静止段；`turn` 的每回合按「段 + 角色」取身份与机制动作。

/// 目的：一个**角色**——身份本身：带哪段提示词、由核心通道还是成员会话出演。
/// 约束：角色只是身份；人数与机制动作属于「它在某一段里坐的那个位」（`Cast`）。
///   文件面与模块面不在这里：前者由 agent 的能力组决定，后者由它装了哪些模块决定（见 `tools` 的 `ToolSource`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Role {
    /// 目的：角色 id（全局唯一）：同时是运行时动作授权里的身份名与提示词的匹配键。
    pub id: &'static str,
    /// 目的：提示词在册子里的位置（`prompts/roles/<名字>.yaml`）。
    pub prompt: &'static str,
    /// 目的：坐核心通道（真）还是坐成员会话（假）。
    pub core: bool,
}

/// 目的：某一段流程里的一个**角色位**——这一段的这个角色要几个人、能用哪些机制动作。
/// 约束：机制动作只在这一段里有效：同一个角色在另一段里可以有另一套动作（或一套都没有）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cast {
    /// 目的：角色 id（引用 `ROLES`）。
    pub role: &'static str,
    /// 目的：这一段至少要这个角色几个人（`0` = 核心通道出演，不占 agent 名额）。
    pub min: usize,
    /// 目的：这一段发给这个角色的机制动作 id（`tools` 提供实现与参数契约）。
    pub tools: &'static [&'static str],
}

/// 目的：一段**流程**（loop）：一串角色位，跑起来是流程或静止身份。
#[derive(Debug, Clone, Copy)]
pub struct Loop {
    /// 目的：段 id（流程会话按它调度；静止会话按它定位唯一的角色位）。
    pub id: &'static str,
    /// 目的：能不能当流程串进会话（真 = 由流程调度回合；假 = 没有流程的会话坐的静止段）。
    pub flow: bool,
    /// 目的：这一段的角色位（按声明顺序；核心角色与成员角色都在这里）。
    pub cast: &'static [Cast],
}

/// 目的：核心代理角色：把用户的话转成对其它会话的动作（开、发、看、停）。
const CORE_PROXY: Role = Role {
    id: "core_proxy",
    prompt: "roles/core_proxy",
    core: true,
};

/// 目的：讨论角色：只表态与核实，不动手（没有写面、没有模块工具）。
const DISCUSSANT: Role = Role {
    id: "discussant",
    prompt: "roles/discussant",
    core: false,
};

/// 目的：计划角色：拆节点、看节点完成、给验收结论（只读，不改文件）。
const PLANNER: Role = Role {
    id: "planner",
    prompt: "roles/planner",
    core: true,
};

/// 目的：验收角色：逐个节点判完成与清单核对（只读，不改文件）。
const ORCHESTRATOR: Role = Role {
    id: "orchestrator",
    prompt: "roles/orchestrator",
    core: true,
};

/// 目的：执行角色：在自己的沙箱里干活，并回报节点结论。
const EXECUTOR: Role = Role {
    id: "executor",
    prompt: "roles/executor",
    core: false,
};

/// 目的：单机角色：一个 agent 直接干活（没有流程时的静止身份，与执行角色同提示词、没有机制动作）。
const SOLO: Role = Role {
    id: "solo",
    prompt: "roles/executor",
    core: false,
};

/// 目的：全部角色的唯一定义处（角色 id 全局唯一：段只引用 id，不复制声明）。
pub const ROLES: &[Role] = &[
    CORE_PROXY,
    DISCUSSANT,
    PLANNER,
    ORCHESTRATOR,
    EXECUTOR,
    SOLO,
];

/// 目的：讨论段：核心代理主持、讨论角色表态（讨论角色至少两人）。
const DISCUSS: Loop = Loop {
    id: "discuss",
    flow: true,
    cast: &[
        Cast {
            role: CORE_PROXY.id,
            min: 0,
            tools: &[],
        },
        Cast {
            role: DISCUSSANT.id,
            min: 2,
            tools: &["say", "agree", "leave", "ask"],
        },
    ],
};

/// 目的：执行段：计划角色拆节点、验收角色判完成、执行角色干活并回报。
/// 约束：会话控制那 6 个是 **system 工具**：由会话类型声明、建会话时展开进 meta，不挂在任何角色位上。
const EXECUTE: Loop = Loop {
    id: "execute",
    flow: true,
    cast: &[
        Cast {
            role: PLANNER.id,
            min: 0,
            tools: &["plan", "slate", "verdict"],
        },
        Cast {
            role: ORCHESTRATOR.id,
            min: 0,
            tools: &["node_verdict", "checklist"],
        },
        Cast {
            role: EXECUTOR.id,
            min: 1,
            tools: &["submit_report"],
        },
    ],
};

/// 目的：单机段：一个 agent 直接干活（静止身份，没有机制动作）。
const STANDARD: Loop = Loop {
    id: "standard",
    flow: false,
    cast: &[Cast {
        role: SOLO.id,
        min: 0,
        tools: &[],
    }],
};

/// 目的：代理段：核心代理一个角色（会话控制工具由会话类型声明，不在这里）。
const PROXY: Loop = Loop {
    id: "proxy",
    flow: false,
    cast: &[Cast {
        role: CORE_PROXY.id,
        min: 0,
        tools: &[],
    }],
};

/// 目的：全部流程段的唯一定义处：两个能当流程用的（讨论、执行）与两个静止段（单机、代理）。
pub const LOOPS: &[Loop] = &[DISCUSS, EXECUTE, STANDARD, PROXY];

/// 目的：按 id 找一段流程；没有返回 `None`（调用方如实报错，不静默退化）。
pub fn loop_of(id: &str) -> Option<&'static Loop> {
    LOOPS.iter().find(|l| l.id == id)
}

/// 目的：能当流程用的段 id（会话类型表只认这些）。
pub fn flow_ids() -> Vec<&'static str> {
    LOOPS.iter().filter(|l| l.flow).map(|l| l.id).collect()
}

/// 目的：按角色 id 找角色声明（全局唯一，段只引用 id）。
pub fn role(id: &str) -> Option<&'static Role> {
    ROLES.iter().find(|r| r.id == id)
}

/// 目的：某一段里某个角色的位置（人数与机制动作）；这一段没有这个角色返回 `None`。
pub fn cast_of(loop_id: &str, role_id: &str) -> Option<&'static Cast> {
    loop_of(loop_id)?.cast.iter().find(|c| c.role == role_id)
}
