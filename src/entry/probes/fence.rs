//! 目的：围栏机制验证与运行期环境白名单探针（隐藏模式 --fence-env / --fence-verify）。
//! 管：守门协议 JSON 的解析、命令取法与三态结论打印。
//! 不管：围栏安装与平台差异（在 kernel::detail::confine）；用户帮助。
//! 联动：由 main 在对应分支调用；入参是守门进程那份 JSON。

/// 目的：按 KEY=VALUE 逐行打出运行期给工具进程的环境白名单（入参 = 守门进程那份 JSON，`--` 之后是命令）。
pub fn print_fence_env(args: &[String], flag: usize) -> i32 {
    let raw = args.get(flag + 1).cloned().unwrap_or_default();
    let command = command_after_separator(args);
    match crate::kernel::detail::confine::FenceJob::from_json(&raw) {
        Ok(job) => {
            for (k, v) in crate::kernel::detail::confine::fence_env(&job.spec, &command) {
                println!("{}={}", k.to_string_lossy(), v.to_string_lossy());
            }
            0
        }
        Err(e) => {
            eprintln!("[围栏] {}", e);
            crate::kernel::detail::confine::FENCE_FAILED
        }
    }
}

/// 目的：命令行里 `--` 之后的命令（守门进程与两个探针入口共用同一份取法：少一处就不一致）。
fn command_after_separator(args: &[String]) -> String {
    args.iter()
        .position(|a| a == "--")
        .and_then(|j| args.get(j + 1))
        .cloned()
        .unwrap_or_default()
}

/// 目的：只做机制验证，如实报三态（enforced / env-unavailable / broken），恒退出 0——
///   判定归调用方（探针按性质决定 env-skip 还是失败）。入参 = 守门进程那份 JSON，`--` 之后是命令。
pub fn fence_verify(args: &[String], flag: usize) -> i32 {
    let raw = args.get(flag + 1).cloned().unwrap_or_default();
    let command = command_after_separator(args);
    let job = match crate::kernel::detail::confine::FenceJob::from_json(&raw) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[围栏] {}", e);
            return crate::kernel::detail::confine::FENCE_FAILED;
        }
    };
    match crate::kernel::detail::confine::verify(&job.spec, &command) {
        crate::kernel::detail::confine::FenceVerdict::Enforced => {
            println!("enforced");
            0
        }
        crate::kernel::detail::confine::FenceVerdict::EnvUnavailable(why) => {
            println!("env-unavailable {}", why);
            0
        }
        crate::kernel::detail::confine::FenceVerdict::Broken(why) => {
            println!("broken {}", why);
            0
        }
    }
}
