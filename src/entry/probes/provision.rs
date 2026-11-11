//! 目的：依赖补齐的隐藏入口（--provision / --ensure / --reset）。
//! 管：参数解析、确认门槛、调用补齐能力并打印机器可读状态或进度。
//! 不管：清单语义、下载、安装与作业状态（在 provision 能力）。
//! 联动：由 main 在 --provision 分支调用；装配在 entry::assembly。

use crate::capabilities::storage::api::Storage as _;

/// 目的：隐藏模式——依赖补齐：不带 `--ensure` 报状态（机器可读）；带 `--ensure[=id,…|kind,…] --yes` 真下载安装。
/// 约束：`--ensure=<kind>` 是组别名（只补该 kind 里不在场的条目，如 `toolchain`）；
pub fn provision(root: &std::path::Path, args: &[String]) -> i32 {
    if let Some(guest) = args.iter().find_map(|a| a.strip_prefix("--reset=")) {
        if !args.iter().any(|a| a == "--yes") {
            eprintln!("[依赖补齐] 还原虚拟根会删除 guest 内状态并重建；确认后加 --yes");
            return 2;
        }
        let svc = crate::entry::assembly::provision_with_guest_vm(root);
        use crate::capabilities::provision::api::Provision;
        return match svc.reset_guest(guest, &mut |line| eprintln!("[依赖补齐] {}", line)) {
            Ok(()) => {
                let req = crate::capabilities::storage::api::CleanRequest {
                    kind: crate::capabilities::storage::api::CleanKind::BuildCache,
                    id: Some(guest.to_string()),
                    module: None,
                    confirm: true,
                };
                if let Err(e) = crate::entry::assembly::storage_from_settings(root).clean(&req) {
                    eprintln!("[依赖补齐] 清构建缓存失败：{}", e);
                }
                0
            }
            Err(e) => {
                eprintln!("[依赖补齐] {}", e);
                1
            }
        };
    }
    let spec = args
        .iter()
        .find_map(|a| a.strip_prefix("--ensure="))
        .map(|s| s.to_string());
    if args.iter().any(|a| a == "--ensure") || spec.is_some() {
        if !args.iter().any(|a| a == "--yes") {
            eprintln!(
                "[依赖补齐] 会从网络下载并写进项目内。确认后加 --yes（先用 --provision 看清单）。"
            );
            return 2;
        }
        // 与网页补齐同一条装配：只持内部 `none`，不拿会话 vm 顶替。
        let p = crate::entry::assembly::provision_with_guest_vm(root);
        let outcome = match spec {
            // 组别名（如 toolchain）只补该 kind 的缺项；裸 --ensure 仍是清单全量。
            Some(spec) => p.ensure_spec(&spec, &mut |line| eprintln!("[依赖补齐] {}", line)),
            None => p.ensure(&[], &mut |line| eprintln!("[依赖补齐] {}", line)),
        };
        return match outcome {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("[依赖补齐] {}", e);
                1
            }
        };
    }
    use crate::capabilities::provision::api::Provision;
    let svc = crate::entry::assembly::provision(root);
    match svc.status() {
        Ok(items) => {
            let missing = items.iter().filter(|s| !s.present).count();
            let doc = serde_json::json!({
                "platform": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "missing": missing,
                "deps": items.iter().map(|s| serde_json::json!({
                    "id": s.id,
                    "present": s.present,
                    "stale": s.stale,
                    "note": s.note,
                    "install": s.install,
                    "probe": s.probe,
                    "source": s.source,
                    "url": s.url,
                    "size_mb": s.size_mb,
                })).collect::<Vec<_>>(),
            });
            println!("{}", doc);
            0
        }
        Err(e) => {
            eprintln!("[依赖检查] {}", e);
            1
        }
    }
}
