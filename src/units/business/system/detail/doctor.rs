//! 目的：自检事实本体——平台、围栏、外部解释器、工具链、进程台账、虚拟机档前置与存储盘面拼成一份 JSON。
//! 管：只读事实的汇总与字段形状（`--doctor` 与用例读同一份）。
//! 不管：围栏机制（kernel）、进程身份（kernel::detail::procs）、各单元内部规则。
//! 联动：由 SystemService::doctor 调用；字段口径见 docs/system/README.md。

use crate::kernel::api::layout;
use std::path::Path;

use serde_json::Value;

use crate::units::business::catalog::packages::api::BaseFacts;
use crate::units::business::maintenance::storage::api::Storage;
use crate::units::business::system::api::{FenceFacts, toolchain_json};

/// 目的：自检事实本体。
/// 参数：root = 产品根；tools = 虚拟机档外部工具的位置事实（与界面同口径）；storage = 空间盘面；
///   facts = 工具链事实；fence = 本机围栏能力。
pub(crate) fn build(
    root: &Path,
    tools: &crate::units::business::vm::api::VmTools,
    storage: &dyn Storage,
    facts: &BaseFacts,
    fence: FenceFacts,
) -> Value {
    // 虚拟机档的逐项前置（只读事实）：登记值 / 项目内补齐 / PATH 三条路与界面同一把尺子。
    let vm = crate::units::business::vm::api::vm_requirements(
        &crate::units::business::vm::api::VmInputs {
            tools,
            probe: &crate::kernel::detail::HostProbeAdapter,
        },
    );
    // 长驻子进程：账本里还活着的那几条（自检只报事实，不动手收）。
    let book = crate::kernel::detail::procs::ProcessBook::open(&layout::state(root));
    let procs: Vec<Value> = book
        .list()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "pid": e.pid,
                "alive": crate::kernel::detail::procs::alive(e.pid),
                "purpose": e.purpose,
                "owner": e.owner,
                "since": e.since,
                "orphanRisk": e.guard_note.is_some(),
                "guardNote": e.guard_note,
            })
        })
        .collect();
    serde_json::json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "fence": { "fs": fence.fs, "net": fence.net, "tree": fence.tree, "note": fence.note },
        "externals": {
            "python": crate::kernel::detail::host_probe::find_exe("python").map(|p| p.to_string_lossy().into_owned()),
            "node": crate::kernel::detail::host_probe::find_exe("node").map(|p| p.to_string_lossy().into_owned()),
            "curl": crate::kernel::detail::host_probe::find_exe("curl").map(|p| p.to_string_lossy().into_owned()),
        },
        "toolchains": toolchain_json(facts),
        "processes": procs,
        "vm_tier": {
            "available": vm.iter().all(|r| r.met),
            "requirements": vm.iter().map(|r| serde_json::json!({
                "id": r.id, "label": r.label, "met": r.met, "detail": r.detail,
                "fix": r.fix.summary, "fixKind": r.fix.kind, "deps": r.fix.deps,
            })).collect::<Vec<_>>(),
        },
        "storage": match storage.report() {
            Ok(s) => serde_json::json!({
                "managed_bytes": s.managed_bytes,
                "cap_bytes": s.cap_bytes,
                "free_bytes": s.free_bytes,
                "over_cap": s.over_cap,
                "categories": s.categories.iter().map(|c| serde_json::json!({
                    "id": c.id, "bytes": c.bytes, "items": c.items.len(), "auto": c.auto_evictable,
                })).collect::<Vec<_>>(),
                "last_gc": s.last_gc,
            }),
            Err(e) => serde_json::json!({ "error": e }),
        },
    })
}
