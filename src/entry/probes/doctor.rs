//! 目的：机器可读的自检事实（隐藏模式 --doctor）。
//! 管：平台、围栏、外部解释器、工具链、进程台账、虚拟机档前置与存储盘面的事实汇总。
//! 不管：围栏机制本身（在 kernel）、进程身份（在 kernel::detail::procs）、能力内部规则。
//! 联动：由 main 在 --doctor 分支调用；装配在 entry::assembly；用例读同一份事实。

use crate::capabilities::storage::api::Storage as _;

/// 目的：自检本机事实（平台 + 围栏能力 + 外部解释器）。只报事实，不猜、不改任何东西（围栏自检那个临时目录除外）。
/// 参数：root = 产品根——虚拟机档要按它读"项目内已装了什么"（与会话判据同一份来源，不另编一套）。
pub fn doctor(root: &std::path::Path) -> i32 {
    println!("{}", doctor_json(root));
    0
}

/// 目的：自检的事实本体（`--doctor` 打印它；用例也读它——不必抓 stdout）。
pub fn doctor_json(root: &std::path::Path) -> serde_json::Value {
    doctor_json_with(root, crate::kernel::detail::confine::capability())
}

/// 目的：自检事实本体；本机能力由调用方传入——生产取一次 `capability()`，用例复用同一份值，
///   不必再触发一次围栏自检（Windows 的自检会写临时目录 ACL）。
pub(crate) fn doctor_json_with(
    root: &std::path::Path,
    cap: crate::kernel::detail::confine::Capability,
) -> serde_json::Value {
    // 虚拟机档的逐项前置（**只读事实**）：项目内补齐过就按那一份问，否则按 PATH 问（与核心同一把尺子）。
    let installed = crate::capabilities::provision::api::Provision::status(
        &crate::entry::assembly::provision(root),
    )
    .unwrap_or_default();
    let installed_at = |prefix: &str| {
        installed
            .iter()
            .find(|d| d.present && d.id.starts_with(prefix))
            .map(|d| root.join(&d.probe).to_string_lossy().into_owned())
    };
    let qemu = installed_at("vm.qemu.");
    let share_tool = installed_at(if crate::kernel::api::IS_WINDOWS {
        "vm.smbd."
    } else {
        "vm.virtiofsd."
    });
    let vm =
        crate::capabilities::vm::api::vm_requirements(&crate::capabilities::vm::api::VmInputs {
            qemu: qemu.as_deref(),
            share_tool: share_tool.as_deref(),
            probe: &crate::kernel::detail::HostProbeAdapter,
        });
    // 长驻子进程：账本里还活着的那几条（`--doctor` 只报事实，不动手收）。
    let book = crate::kernel::detail::procs::ProcessBook::open(&root.join(".home"));
    let procs: Vec<serde_json::Value> = book
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
    let base = crate::entry::toolchains::base_facts(root);
    let doc = serde_json::json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "fence": { "fs": cap.fs, "net": cap.net, "tree": cap.tree, "note": cap.note },
        "externals": {
            "python": crate::kernel::detail::host_probe::find_exe("python").map(|p| p.to_string_lossy().into_owned()),
            "node": crate::kernel::detail::host_probe::find_exe("node").map(|p| p.to_string_lossy().into_owned()),
            "curl": crate::kernel::detail::host_probe::find_exe("curl").map(|p| p.to_string_lossy().into_owned()),
        },
        "toolchains": crate::entry::toolchains::json(&base),
        "processes": procs,
        "vm_tier": {
            "available": vm.iter().all(|r| r.met),
            "requirements": vm.iter().map(|r| serde_json::json!({
                "id": r.id, "met": r.met, "detail": r.detail, "how": r.how,
            })).collect::<Vec<_>>(),
        },
        "storage": match crate::entry::assembly::storage_from_settings(root).report() {
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
    });
    doc
}
