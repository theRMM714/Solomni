//! 名单的**收束规则**（纯函数）：模式决定最终交出去的是几条。

use crate::capabilities::session::api::AgentMeta;
use crate::capabilities::slate::api::{Mode, Pick};

/// 按模式收束：协作 = 原样（N 个独立 agent）；单 agent = 最多一条。
pub fn collapse(picks: Vec<Pick>, mode: Mode) -> Vec<Pick> {
    match mode {
        Mode::Collab => picks,
        // 单 agent：核心只给一条就原样采纳（模块数与 reuse 都保持它自己的）；
        // 给多条就把它们的模块并成一个临时 agent（并过的不是任何单个已存 agent）。
        Mode::Single => match picks.len() {
            0 | 1 => picks,
            _ => vec![merged(picks)],
        },
    }
}

/// 把多条并成一个临时 agent（模块按出现顺序去重、模型取首项、理由合并）。
fn merged(picks: Vec<Pick>) -> Pick {
    let mut modules: Vec<String> = Vec::new();
    let mut model: Option<String> = None;
    let mut why: Vec<String> = Vec::new();
    for p in picks {
        if model.is_none() {
            model = p.agent.model.clone();
        }
        for id in p.agent.modules {
            if !modules.contains(&id) {
                modules.push(id);
            }
        }
        if !p.why.trim().is_empty() {
            why.push(p.why);
        }
    }
    Pick {
        agent: AgentMeta {
            name: "组合".to_string(),
            // 并过的不是任何单个已存 agent：它是这次临时组装出来的。
            transient: true,
            modules,
            model,
            permissions: Default::default(),
        },
        why: why.join("；"),
    }
}
