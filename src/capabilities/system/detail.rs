//! 目的：系统维护与自检能力的机制实现——自检里要直读宿主的那几项，以及便携维护（PATH / 围栏预授）。
//! 管：doctor 的事实汇总、围栏能力取值、`--setup` 的落点与读写。
//! 不管：用例编排与确认门槛（在 service）；各能力的内部规则。
//! 联动：只被 SystemService（与入口的启动对账）调用；事实来源见 kernel 与各能力 api。

pub(crate) mod doctor;
pub(crate) mod setup;

use serde_json::Value;

use crate::capabilities::catalog::packages::api::BaseFacts;
use crate::capabilities::provision::api::Provision;
use crate::capabilities::maintenance::storage::api::Storage;
use crate::capabilities::system::api::{Facts, FenceFacts};

/// 目的：本机围栏能力（自检用；生产路径只取一次）。
pub(crate) fn fence() -> FenceFacts {
    let cap = crate::kernel::detail::confine::capability();
    FenceFacts {
        fs: cap.fs,
        net: cap.net,
        tree: cap.tree,
        note: cap.note,
    }
}
/// 目的：宿主事实的端口实现——围栏能力取自 kernel，自检报告由 doctor 子模块汇总。
pub(crate) struct HostFacts;

impl Facts for HostFacts {
    fn fence(&self) -> FenceFacts {
        fence()
    }

    fn doctor(
        &self,
        root: &std::path::Path,
        provision: &dyn Provision,
        storage: &dyn Storage,
        toolchains: &BaseFacts,
        fence: FenceFacts,
    ) -> Value {
        doctor::build(root, provision, storage, toolchains, fence)
    }
}
