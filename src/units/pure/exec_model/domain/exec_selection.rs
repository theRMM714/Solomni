//! 目的：执行档位与执行选型——「在哪跑、挂哪个虚拟根、定哪个版本」这三个共享事实。
//! 管：`Tier`（本机 / 虚拟机）、`ExecSpec`（档位 + 虚拟根名 + 能力定版）与档位的对外解析。
//! 不管：虚拟机怎么起、清单怎么解析（vm）；执行计划怎么排（provision / runtime）；落盘（会话 meta 的读写方）。
//! 联动：登记处、vm、provision、runtime、turn 与呈现层都经 units::pure::exec_model::api 取它；唯一家判据见 R6。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：执行档位——本机 = 直接在宿主上跑；虚拟机 = 整台 guest（不信任 AI 时的可选档）。
/// 约束：共享形状只有这一份（R6）——它被登记处与执行计划共享，且不认识任何业务概念。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Host,
    Vm,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Host => "host",
            Tier::Vm => "vm",
        }
    }

    /// 目的：把对外写法解析成档位；缺省（空串）= 本机档，未知值如实报错。
    /// 约束：呈现层与协调业务共用这一处解析，不再各写一份（对外词汇只有 host / vm）。
    pub fn parse(s: &str) -> Result<Tier, String> {
        match s.trim() {
            "" | "host" => Ok(Tier::Host),
            "vm" => Ok(Tier::Vm),
            other => Err(format!("未知执行档位：{}（只接受 host / vm）", other)),
        }
    }
}

/// 目的：会话的执行选型——档位 + 虚拟根名 + 能力定版。
/// 约束：进 meta.yaml 的 exec 段（缺字段按默认 = 本机档读回）；本机/虚拟机两侧与呈现层共享，按 R6 只此一份。
///   网络与围栏要求是**全局策略**，不在这里。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecSpec {
    #[serde(default)]
    pub tier: Tier,
    /// 目的：虚拟机档用的**虚拟根名**（清单 `guest` 字段，如 debian / windows）；本机档为空。
    /// 约束：存的是名字不是路径——路径由 `vm::resolve_guest` 按当前安装解析，换安装/挪产品根都不用改会话。
    #[serde(default)]
    pub guest: Option<String>,
    /// 目的：能力名 → 精确版本（用户定版；多版本时不猜）。
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
}
