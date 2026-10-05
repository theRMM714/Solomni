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
