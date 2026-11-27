//! 目的：把「一席要什么」变成一份工作面——工作面清单的唯一构造点（工作台形状只在这里落值）。
//! 管：`seat_benches`（工作名 / 会话名 / 每席的私有根、模块目录、权限面与共享区归属）。
//! 不管：工作面与寻址的**形状与语义**（环 2 契约单元 `workbench`）；这些事实从哪来（在 `turn` 取齐）；策略注册与布局（在 service 与 `Workdirs`）。
//! 联动：`turn` 取好工作根与每席计划后转调；代理会话与普通会话走同一条路，不再各造一份。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::kernel::ports::PathCodec;
use crate::units::business::workbench::api::{Bench, Benches, WORKIGNORE, WorkRoots};
use crate::units::business::workspace::domain::scope::scope_from;
use crate::units::pure::permission::api::PermissionSet;

/// 目的：一个可用的模块目录（由调用方从清单解析好，工作台不认识清单类型）。
#[derive(Debug, Clone)]
pub struct ModuleDir {
    /// 目的：模块 id（工作面按它登记源码目录）。
    pub id: String,
    /// 目的：模块源码目录（只读可达）。
    pub root: std::path::PathBuf,
    /// 目的：模块私有区（有则可写）。
    pub private: Option<std::path::PathBuf>,
}

/// 目的：一席要的东西：谁、用哪些模块、权限面、以及沿链的工作台声明（最外层是根）。
#[derive(Debug, Clone)]
pub struct SeatPlan {
    /// 目的：agent 名（工作面的主人）。
    pub agent: String,
    /// 目的：这一席用到的模块 id。
    pub modules: Vec<String>,
    /// 目的：这一席的生效网络档位。
    pub permissions: PermissionSet,
    /// 目的：根在前的策略参数链（共享区归属按它派生；字段由本单元解释）。
    pub params: Vec<serde_json::Value>,
    /// 目的：私有根显式指定（代理发言席的目录名与发言人不一致时才给）；None = 按 agent 名从工作区根取。
    pub private: Option<std::path::PathBuf>,
}

/// 目的：组装工作面清单所需的已取事实。
pub struct SeatFacts<'a> {
    /// 目的：顶层工作名（整棵树只有一个 work/）。
    pub work: &'a str,
    /// 目的：会话名（写进每份工作面）。
    pub session: &'a str,
    /// 目的：工作区端口给出的根（共享区、会话根、每个 agent 的私有根）。
    pub roots: &'a WorkRoots,
    /// 目的：虚拟机档给 guest↔宿主的路径翻译器。
    pub codec: Option<Arc<dyn PathCodec>>,
    /// 目的：全局默认共享区范围（会话声明的根可在它之上覆盖）。
    pub base_scope: &'a crate::units::pure::permission::api::SharedScope,
    /// 目的：全局硬黑名单（任何一层都不得放开）。
    pub hard_deny: &'a [String],
    /// 目的：清单里可用的模块目录。
    pub modules: &'a [ModuleDir],
    /// 目的：每席的计划（顺序即工作面清单的顺序）。
    pub seats: &'a [SeatPlan],
}

/// 目的：按席位计划组装工作面清单；模块目录只对使用它的 agent 可达。
/// 参数：`facts` = 已取好的工作名、工作区根、模块目录与每席计划。
/// 返回：一次工作的 `Benches`（共享区根、会话根与每席一份 `Bench`）。
/// 错误：工作区没给出某席的私有根时逐字报「工作区没有给出 agent {名} 的工作面路径」；
///   声明链算不出共享区归属时把错误原样透出。
pub fn seat_benches(facts: &SeatFacts<'_>) -> Result<Benches, String> {
    let mut list: Vec<Bench> = Vec::new();
    for seat in facts.seats {
        let private = match &seat.private {
            Some(p) => p.clone(),
            None => facts
                .roots
                .agents
                .get(&seat.agent)
                .cloned()
                .ok_or_else(|| format!("工作区没有给出 agent {} 的工作面路径", seat.agent))?,
        };
        let mut modules = BTreeMap::new();
        let mut module_data = BTreeMap::new();
        for id in &seat.modules {
            if let Some(m) = facts.modules.iter().find(|m| &m.id == id) {
                modules.insert(id.clone(), m.root.clone());
                if let Some(private) = &m.private {
                    module_data.insert(id.clone(), private.clone());
                }
            }
        }
        let scope = scope_from(&seat.params, facts.base_scope, facts.hard_deny)?;
        list.push(Bench {
            work_name: facts.work.to_string(),
            session: facts.session.to_string(),
            agent: seat.agent.clone(),
            shared: facts.roots.shared.clone(),
            private,
            modules,
            module_data,
            permissions: seat.permissions.clone(),
            scope,
            path_codec: facts.codec.clone(),
            filter_rules: facts.roots.store.join(WORKIGNORE),
        });
    }
    Ok(Benches {
        shared: facts.roots.shared.clone(),
        session_root: facts.roots.session_root.clone(),
        list,
        codec: facts.codec.clone(),
    })
}
