//! 目的：回收一个工作里不再可达的内容对象（隐藏模式 --work-gc）。
//! 管：参数解析、dry-run 与调用工作区能力。
//! 不管：工作区对象格式与可达性判定（在 workspace 能力）。
//! 联动：由 main 在 --work-gc 分支调用。

/// 目的：回收一个工作里不再可达的内容对象（隐藏维护命令）。
/// 约束：用法 solomni --work-gc <工作名> [--yes]；默认 dry-run，加 --yes 才删。
pub fn work_gc(root: &std::path::Path, args: &[String], i: usize) -> i32 {
    let work = args
        .get(i)
        .and_then(|a| a.strip_prefix("--work-gc="))
        .map(str::to_string)
        .or_else(|| args.get(i + 1).filter(|a| !a.starts_with('-')).cloned())
        .unwrap_or_default();
    if work.is_empty() {
        eprintln!("用法：solomni --work-gc <工作名> [--yes]");
        return 2;
    }
    let work = match crate::capabilities::workspace::api::safe_file_name(&work) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[回收] {}", e);
            return 2;
        }
    };
    let apply = args.iter().any(|a| a == "--yes");
    let session = root.join("session").join(&work);
    if !session.is_dir() {
        eprintln!("[回收] 没有这个工作：{}", work);
        return 2;
    }
    let dirs = std::sync::Arc::new(
        crate::capabilities::workspace::detail::fs_workspace::FsWorkspace::new(
            root.join("session"),
        ),
    );
    let store = std::sync::Arc::new(
        crate::capabilities::workspace::detail::fs_workstore::FsWorkStore::new(),
    );
    let workspace = crate::capabilities::workspace::service::WorkspaceService::new(dirs, store);
    match crate::capabilities::workspace::api::Workspace::work_gc(&workspace, &work, apply) {
        Ok(r) => {
            println!(
                "[回收] 工作 {}：对象 {} 个 / {} 字节；可达 {} / {}；不可达 {} / {}；杂项 {} 个",
                work,
                r.objects,
                r.bytes,
                r.reachable_objects,
                r.reachable_bytes,
                r.unreachable_objects,
                r.unreachable_bytes,
                r.ignored_files
            );
            if r.applied {
                println!(
                    "[回收] 已删除 {} 个 / {} 字节；清理杂项 {} 个",
                    r.removed_objects, r.removed_bytes, r.removed_ignored
                );
            } else {
                println!("[回收] 未删除（dry-run）；确认后加 --yes");
            }
            0
        }
        Err(e) => {
            eprintln!("[回收] {}", e);
            1
        }
    }
}
