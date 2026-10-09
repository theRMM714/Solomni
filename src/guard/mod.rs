//! **围栏守门进程**：它是**第二个程序入口**，由工具进程按 `--fence-run` 拉起。
//! 与「启动应用」分开的理由：它跑的是**模块作者写的命令**，生命周期与退出码都属于那次工具调用。
//! 机制全在 `crate::capabilities::tools::detail::confine`；这里只做 argv → 机制的分发。

/// 守门模式：读回围栏参数与命令，装围栏 → 跑命令 → 以工具退出码收场（失败如实报错，不静默）。
pub fn fence_run(args: &[String], flag: usize) -> i32 {
    let raw_job = args.get(flag + 1).cloned().unwrap_or_default();
    let command = match args.iter().position(|a| a == "--") {
        Some(j) => args.get(j + 1).cloned().unwrap_or_default(),
        None => String::new(),
    };
    match crate::capabilities::tools::detail::confine::FenceJob::from_json(&raw_job) {
        Ok(job) => crate::capabilities::tools::detail::confine::run_fenced(&job, &command),
        Err(e) => {
            eprintln!("[围栏] {}", e);
            crate::capabilities::tools::detail::confine::FENCE_FAILED
        }
    }
}

/// 目的：**按条处置**围栏台账（与 --fence-clean 的整体收尾并列）：列清单 / 还原一条 / 撤一条 / 删一个 profile。
/// 返回：Some(退出码) = 这组开关出现了并已处置（0 成功 / 1 失败 / 2 用法错）；None = 没出现（交给别的模式）。
/// 约束：与 --fence-clean 共用同一份台账与同一套 ACE 读法（confine 的 catalog / restore_one / revoke_grant /
///   remove_profile_one）；**未指定的条目一概不动**。
pub fn fence_grant(args: &[String], root: &std::path::Path) -> Option<i32> {
    use crate::capabilities::tools::detail::confine;
    let home = root.join(".home");
    let after = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    // 启动对账的手动入口（与启动期同一份实现）：按台账回收归属明确已死的陈旧授权。
    if args.iter().any(|a| a == "--fence-reconcile") {
        let rep = confine::reconcile(&home);
        if rep.errors.is_empty() {
            println!("[围栏] {}", rep.summary());
            return Some(0);
        }
        eprintln!("[围栏] {}", rep.summary());
        return Some(1);
    }
    // 列清单：机器可读优先（默认 JSON）；--human 给一行一句的可读版。恒退出 0，判定归调用方。
    if args.iter().any(|a| a == "--fence-ledger") {
        let view = confine::ledger(&home);
        if args.iter().any(|a| a == "--human") {
            for line in ledger_lines(&view) {
                println!("[围栏] {}", line);
            }
        } else {
            match serde_json::to_string_pretty(&view) {
                Ok(text) => println!("{}", text),
                Err(e) => {
                    eprintln!("[围栏] 台账序列化失败：{}", e);
                    return Some(1);
                }
            }
        }
        return Some(0);
    }
    // 按路径只还原一条快照（其余条目与整份 DACL 不受影响）。
    if args.iter().any(|a| a == "--fence-restore") {
        let Some(path) = after("--fence-restore") else {
            eprintln!("用法：solomni --fence-restore 路径（要整体收尾用 --fence-clean）");
            return Some(2);
        };
        return Some(report(confine::restore_one(
            &home,
            &std::path::PathBuf::from(path),
        )));
    }
    // 按 SID + 路径只撤一条授权；不在台账里的台账外残留也走这一条。
    if args.iter().any(|a| a == "--fence-revoke") {
        let (Some(sid), Some(path)) = (after("--fence-revoke"), {
            args.iter()
                .position(|a| a == "--fence-revoke")
                .and_then(|i| args.get(i + 2))
                .cloned()
        }) else {
            eprintln!("用法：solomni --fence-revoke <SID> 路径（不在台账里的残留也照撤）");
            return Some(2);
        };
        return Some(report(confine::revoke_grant(
            &home,
            &sid,
            &std::path::PathBuf::from(path),
        )));
    }
    // 按名只删一个容器 profile。
    if args.iter().any(|a| a == "--fence-profile-rm") {
        let Some(name) = after("--fence-profile-rm") else {
            eprintln!("用法：solomni --fence-profile-rm <profile 名>");
            return Some(2);
        };
        return Some(report(confine::remove_profile_one(&home, &name)));
    }
    None
}

/// 一行一句的可读清单（时间记 Unix 秒：不引时区与本地化）。
fn ledger_lines(view: &crate::capabilities::tools::detail::confine::Ledger) -> Vec<String> {
    use crate::capabilities::tools::detail::confine::LedgerEntry;
    let mut out: Vec<String> = Vec::new();
    if !view.note.is_empty() {
        out.push(view.note.clone());
    }
    for e in &view.entries {
        let LedgerEntry {
            kind,
            sid,
            path,
            rights,
            at,
            present,
            owners,
            ace_sids,
            notes,
        } = e;
        let state = if *present {
            "现在还在"
        } else {
            "现在不在了"
        };
        let mut line = match *kind {
            "snapshot" => format!("[快照] {}（记于 {} 秒；{}）", path, at, state),
            "grant" => format!(
                "[授权] {} → {}（权限位 0x{:X}，记于 {} 秒；{}）",
                sid,
                path,
                rights.unwrap_or(0),
                at,
                state
            ),
            _ => format!("[profile] {}（记于 {} 秒；{}）", path, at, state),
        };
        if !owners.is_empty() {
            let who: Vec<String> = owners
                .iter()
                .map(|o| {
                    if o.lease.is_empty() {
                        format!("pid {}（创建于 {}）", o.pid, o.start)
                    } else {
                        format!("pid {}（创建于 {}，会话 {}）", o.pid, o.start, o.lease)
                    }
                })
                .collect();
            line.push_str(&format!("；归属：{}", who.join("、")));
        }
        if !ace_sids.is_empty() {
            line.push_str(&format!("；盘上显式包授权：{}", ace_sids.join("、")));
        }
        for n in notes {
            line.push_str(&format!("；{}", n));
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push("台账里没有条目".to_string());
    }
    out
}

/// 把一次按条处置的结果如实打到用户面：成功一句话，失败写清为什么。
fn report(done: Result<String, String>) -> i32 {
    match done {
        Ok(msg) => {
            println!("[围栏] {}", msg);
            0
        }
        Err(e) => {
            eprintln!("[围栏] {}", e);
            1
        }
    }
}

/// （台账可能不存在：探针、夹具的台账被删、旧版本建的）——隐藏模式，用户经文档知道它。
pub fn fence_clean(root: &std::path::Path) -> i32 {
    let home = root.join(".home");
    let mut lines: Vec<String> = Vec::new();
    let mut failed = false;
    match crate::capabilities::tools::detail::confine::clean(&home) {
        Ok(msg) => lines.push(msg),
        Err(e) => {
            lines.push(format!("台账回收未完成：{}", e));
            failed = true;
        }
    }
    // 孤儿授权清扫必须排在 profile 清扫**之前**：profile 删了就派生不出 SID，没法按 SID 找残留。
    #[cfg(windows)]
    match crate::capabilities::tools::detail::confine::sweep_orphan_aces(root) {
        Ok(0) => lines.push("产品根内没有台账外的孤儿授权".to_string()),
        Ok(n) => lines.push(format!("产品根内孤儿授权清扫：连树撤掉 {} 处", n)),
        Err(e) => {
            lines.push(format!("孤儿授权清扫未完成：{}", e));
            failed = true;
        }
    }
    match crate::capabilities::tools::detail::confine::sweep_profiles() {
        Ok(n) => lines.push(format!("扫掉 {} 个本程序建过的容器 profile", n)),
        Err(e) => {
            lines.push(format!("容器 profile 清扫未完成：{}", e));
            failed = true;
        }
    }
    for line in &lines {
        if failed {
            eprintln!("[围栏] 清理未完成：{}", line);
        } else {
            println!("[围栏] 清理完成：{}", line);
        }
    }
    if failed {
        1
    } else {
        0
    }
}
