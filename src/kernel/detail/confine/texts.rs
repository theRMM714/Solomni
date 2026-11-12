//! 目的：围栏内失败时给回执的边界说明与可达范围摘要。
//! 管：把 FenceSpec 的可达根压成一行、只在围栏真强制且工具非零退出时给说明。
//! 不管：围栏判定、授权与平台差异。
//! 联动：由 `process.rs` 的回执装配与 `confine.rs` 门面消费；边界标记来自 `contracts/fence.rs`。

use crate::kernel::api::FenceSpec;
use std::path::Path;

/// 目的：围栏里工具失败时给回执的边界说明（可达范围 + 范围外被拒）；不需要说明时返回 None。
/// 约束：只在围栏真强制生效且工具自己非零退出时给；不含任何语言知识，也不猜被拒的是哪条路径。
pub(super) fn fence_boundary_note(
    spec: &FenceSpec,
    fs_enforced: bool,
    net_enforced: bool,
    code: i32,
) -> Option<String> {
    if !fs_enforced || code == 0 {
        return None;
    }
    let net = if net_enforced {
        if spec.net {
            "开"
        } else {
            "关"
        }
    } else {
        "未生效"
    };
    Some(format!(
        "[围栏] 工具进程只可达 {}（网：{}）；{}。",
        reachable_roots(spec),
        net,
        crate::kernel::api::FENCE_BOUNDARY_MARK
    ))
}

/// 目的：可达范围的一行摘要（工作目录 + 读写/只读根，去重；超过三处折叠成一处计数）。
pub(super) fn reachable_roots(spec: &FenceSpec) -> String {
    let mut all: Vec<String> = Vec::new();
    push_root(&mut all, &spec.cwd);
    for p in spec
        .rw
        .iter()
        .chain(spec.ro.iter())
        .chain(spec.ro_tree.iter())
    {
        push_root(&mut all, p);
    }
    if all.is_empty() {
        return "（无）".to_string();
    }
    if all.len() <= 3 {
        return all.join("、");
    }
    format!("{} 等 {} 处", all[..3].join("、"), all.len())
}

/// 目的：把一个非空的根收进摘要，重复的丢掉。
fn push_root(all: &mut Vec<String>, p: &Path) {
    if p.as_os_str().is_empty() {
        return;
    }
    let s = p.display().to_string();
    if !all.contains(&s) {
        all.push(s);
    }
}

/// 目的：把边界说明打到 stderr（守门进程的 stderr 由外层拼进工具回执）。
pub(super) fn note_fence_boundary(
    spec: &FenceSpec,
    fs_enforced: bool,
    net_enforced: bool,
    code: i32,
) {
    if let Some(line) = fence_boundary_note(spec, fs_enforced, net_enforced, code) {
        eprintln!("{}", line);
    }
}
