//! 目的：存储回收（隐藏模式 --gc）；默认 dry-run，--yes 才删。
//! 管：参数解析、预览、调用存储能力并打印回收报告。
//! 不管：盘面盘点、回收策略与候选判定（在 storage 能力）。
//! 联动：由 main 在 --gc 分支调用；装配在 entry::assembly。

use super::print_gc;

/// 目的：存储回收（--gc [--yes] [--category=…] [--guest=…] [--module=…]）；默认 dry-run。
pub fn gc(root: &std::path::Path, args: &[String], i: usize) -> i32 {
    let apply = args.iter().any(|a| a == "--yes");
    let category = args
        .iter()
        .find_map(|a| a.strip_prefix("--category="))
        .map(str::to_string)
        .or_else(|| {
            args.get(i)
                .and_then(|a| a.strip_prefix("--gc="))
                .map(str::to_string)
        });
    let guest = args
        .iter()
        .find_map(|a| a.strip_prefix("--guest="))
        .map(str::to_string);
    let module = args
        .iter()
        .find_map(|a| a.strip_prefix("--module="))
        .map(str::to_string);
    let svc = crate::entry::assembly::storage_from_settings(root);
    use crate::capabilities::storage::api::Storage;
    if let Some(cat) = category {
        let kind = match crate::capabilities::storage::api::CleanKind::parse(&cat) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("[存储] {}", e);
                return 2;
            }
        };
        let req = crate::capabilities::storage::api::CleanRequest {
            kind,
            id: guest,
            module,
            confirm: apply,
        };
        if !apply {
            println!("[存储] 预览模式：确认后加 --yes 才会清理 {}", cat);
            match svc.report() {
                Ok(r) => {
                    let c = r
                        .categories
                        .iter()
                        .find(|c| c.id == cat || c.id == "guest-bases");
                    if let Some(c) = c {
                        println!(
                            "[存储] {}：{} 字节 / {} 项",
                            c.label,
                            c.bytes,
                            c.items.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("[存储] {}", e);
                    return 1;
                }
            }
            return 0;
        }
        match svc.clean(&req) {
            Ok(crate::capabilities::storage::api::CleanOutcome::Report(r)) => {
                print_gc(&r);
                0
            }
            Ok(crate::capabilities::storage::api::CleanOutcome::Started(job)) => {
                println!("[存储] 已启动作业：{}", job.lines.join(" / "));
                while svc.job().running {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                let j = svc.job();
                for line in &j.lines {
                    println!("[存储] {}", line);
                }
                if let Some(e) = j.error {
                    eprintln!("[存储] {}", e);
                    1
                } else {
                    0
                }
            }
            Err(e) => {
                eprintln!("[存储] {}", e);
                1
            }
        }
    } else {
        match svc.gc(apply) {
            Ok(r) => {
                print_gc(&r);
                0
            }
            Err(e) => {
                eprintln!("[存储] {}", e);
                1
            }
        }
    }
}
