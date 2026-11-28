//! 目的：执行环境的**派生选型**——会话里存的是执行环境声明（策略 id + 参数），这里放解析出来的三件事实：
//!   「在哪一侧跑、哪个虚拟根、能力定哪个版本」，供执行计划与就绪判定使用。
//! 管：`Side`（宿主侧 / guest 侧）与 `TargetSpec`（侧 + 虚拟根名 + 定版）。
//! 不管：策略清单、策略参数与策略实现（sandbox）；虚拟机怎么起、计划怎么排（vm / packages）。
//! 联动：sandbox 把声明解析成 `TargetSpec`；vm、packages、provision、tools、runtime 与呈现层经 api 取用。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 目的：执行在哪一侧——宿主侧 = 直接在宿主上跑；guest 侧 = 整台虚拟机里（不信任 AI 时的可选档）。
/// 约束：这是**派生事实**，不是策略清单：策略有哪些、叫什么由 sandbox 注册表回答（`SandboxFacts.policy`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Host,
    Guest,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Host => "host",
            Side::Guest => "guest",
        }
    }

    pub fn on_guest(&self) -> bool {
        *self == Side::Guest
    }
}

/// 目的：一次执行的选型事实——哪一侧 + 虚拟根名 + 能力定版。
/// 约束：由 `SandboxDecl`（策略 id + 参数 + 定版）解析而来；**不落盘**，落盘的是声明本身（会话 meta 的 sandbox 段）。
///   网络与围栏要求是全局策略，不在这里。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    #[serde(default)]
    pub side: Side,
    /// 目的：guest 侧用的**虚拟根名**（策略参数 `guest`，如 debian / windows）；宿主侧为空。
    /// 约束：存的是名字不是路径——路径由 `vmstore::api::resolve_guest` 按当前安装解析，换安装/挪产品根都不用改会话。
    #[serde(default)]
    pub guest: Option<String>,
    /// 目的：能力名 → 精确版本（用户定版；多版本时不猜）。
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
}
