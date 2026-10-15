//! **机器可读的自检与探针**：测试与 CI 的接口，不是用户功能。
//! 退出码按各模式的约定（多数恒 0：**判定归调用方**，这里只报事实）。
//! 隐藏模式：用户经文档知道它们，不列进面向用户的帮助。

/// 目的：把归档解到目录（跨平台纯 Rust，不依赖用户装解压工具；用法 solomni --unpack <archive> <dest>）。
pub fn unpack_cli(args: &[String], i: usize) -> i32 {
    let archive = args.get(i + 1).cloned().unwrap_or_default();
    let dest = args.get(i + 2).cloned().unwrap_or_default();
    if archive.is_empty() || dest.is_empty() {
        eprintln!("用法：solomni --unpack <archive> <dest>");
        return 2;
    }
    match crate::kernel::detail::unpack::extract(
        std::path::Path::new(&archive),
        std::path::Path::new(&dest),
    ) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("[解包] {}", e);
            1
        }
    }
}

/// 自检：本机事实（平台 + 围栏能力 + 外部解释器）。只报事实，不猜、不改任何东西（围栏自检那个临时目录除外）。
pub fn doctor() -> i32 {
    let cap = crate::kernel::detail::confine::capability();
    // 虚拟机档的逐项前置（**只读事实**）：这里按"没登记 QEMU、没指定基础根"问一次，
    // 也就是最朴素的情形——登记过的路径以会话配置界面为准（那里按会话选型问同一份清单）。
    let vm =
        crate::capabilities::vm::api::vm_requirements(&crate::capabilities::vm::api::VmInputs {
            qemu: None,
            share_tool: None,
            probe: &crate::kernel::detail::HostProbeAdapter,
        });
    let doc = serde_json::json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "fence": { "fs": cap.fs, "net": cap.net, "tree": cap.tree, "note": cap.note },
        "externals": {
            "python": crate::kernel::detail::host_probe::find_exe("python").map(|p| p.to_string_lossy().into_owned()),
            "node": crate::kernel::detail::host_probe::find_exe("node").map(|p| p.to_string_lossy().into_owned()),
            "curl": crate::kernel::detail::host_probe::find_exe("curl").map(|p| p.to_string_lossy().into_owned()),
        },
        "vm_tier": {
            "available": vm.iter().all(|r| r.met),
            "requirements": vm.iter().map(|r| serde_json::json!({
                "id": r.id, "met": r.met, "detail": r.detail, "how": r.how,
            })).collect::<Vec<_>>(),
        },
    });
    println!("{}", doc);
    0
}

/// 目的：给 `--provision` 的补齐作业装配 vm 能力面（guest-runtime 的运行期编排要起 guest）。
/// 约束：只用于补齐——共享档用 QGA 复制（与挂载无关），引擎路径由编排条目自己给。
fn provisioning_vm(
    root: &std::path::Path,
) -> std::sync::Arc<dyn crate::capabilities::vm::api::Vm + Send + Sync> {
    std::sync::Arc::new(crate::capabilities::vm::service::VmService::new(
        std::sync::Arc::new(crate::capabilities::vm::detail::FsPackages::new(
            root.join("runtimes"),
        )),
        std::sync::Arc::new(crate::capabilities::vm::detail::QemuEngine::new()),
        std::sync::Arc::new(crate::capabilities::vm::detail::CopyShare::new()),
    ))
}

/// 目的：隐藏模式——依赖补齐：不带 `--ensure` 报状态（机器可读）；带 `--ensure[=id,…] --yes` 真下载安装。
pub fn provision(root: &std::path::Path, args: &[String]) -> i32 {
    let ids: Vec<String> = args
        .iter()
        .find_map(|a| a.strip_prefix("--ensure="))
        .map(|s| {
            s.split(',')
                .filter(|x| !x.is_empty())
                .map(|x| x.to_string())
                .collect()
        })
        .unwrap_or_default();
    if args.iter().any(|a| a == "--ensure") || !ids.is_empty() {
        if !args.iter().any(|a| a == "--yes") {
            eprintln!(
                "[依赖补齐] 会从网络下载并写进项目内。确认后加 --yes（先用 --provision 看清单）。"
            );
            return 2;
        }
        let p =
            crate::capabilities::provision::ProvisionService::with_vm(root, provisioning_vm(root));
        return match p.ensure(&ids, &mut |line| eprintln!("[依赖补齐] {}", line)) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("[依赖补齐] {}", e);
                1
            }
        };
    }
    use crate::capabilities::provision::api::Provision;
    let svc = crate::capabilities::provision::ProvisionService::new(root);
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

/// 隐藏模式：用**产品自己的出站代理**（含按平台装配的 TLS）打一次最小 HTTPS 请求，如实报结论。
/// 四态机器可读：ok（通）/ no-net（环境连不上外网）/ env-tls（本进程取不到系统 TLS 凭证，如沙箱挡住凭证存储）/
/// tls-fail|fail（我们链路坏了）。
/// 退出码恒 0：判定归调用方（测试按性质决定 env-skip 还是失败），这里只报事实。
pub fn https_check(args: &[String], i: usize) -> i32 {
    let url = args.get(i + 1).cloned().unwrap_or_default();
    if url.is_empty() {
        eprintln!("用法：solomni --https-check <https url>");
        return 2;
    }
    let backend = crate::capabilities::llm::detail::http_agent::tls_backend();
    let agent = crate::capabilities::llm::detail::http_agent::agent(10, 20);
    match agent.get(&url).call() {
        Ok(resp) => {
            println!("[HTTPS] ok {} {} {}", resp.status().as_u16(), backend, url);
            0
        }
        Err(e) => {
            println!(
                "[HTTPS] {} {} {}",
                crate::capabilities::llm::detail::http_agent::classify(&e),
                backend,
                e
            );
            0
        }
    }
}

/// 隐藏模式：按 KEY=VALUE 逐行打出运行期给工具进程的环境白名单（入参 = 守门进程那份 JSON，`--` 之后是命令）。
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

/// 命令行里 `--` 之后的命令（守门进程与两个探针入口共用同一份取法：少一处就不一致）。
fn command_after_separator(args: &[String]) -> String {
    args.iter()
        .position(|a| a == "--")
        .and_then(|j| args.get(j + 1))
        .cloned()
        .unwrap_or_default()
}

/// 隐藏模式：只做机制验证，如实报三态（enforced / env-unavailable / broken），恒退出 0——
/// 判定归调用方（探针按性质决定 env-skip 还是失败）。入参 = 守门进程那份 JSON，`--` 之后是命令。
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

/// 入站契约（机器可读）：HTTP 路由目录的唯一定义（见 docs/presentation/contracts.md）。
pub fn print_routes() {
    println!("{}", crate::presentation::web::routes::catalog_json());
}
