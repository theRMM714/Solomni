//! **机器可读的自检与探针**：测试与 CI 的接口，不是用户功能。
//! 退出码按各模式的约定（多数恒 0：**判定归调用方**，这里只报事实）。
//! 隐藏模式：用户经文档知道它们，不列进面向用户的帮助。

/// 自检：本机事实（平台 + 围栏能力 + 外部解释器）。只报事实，不猜、不改任何东西（围栏自检那个临时目录除外）。
pub fn doctor() -> i32 {
    let cap = crate::capabilities::tools::detail::confine::capability();
    // 虚拟机档的逐项前置（**只读事实**）：这里按"没登记 QEMU、没指定基础根"问一次，
    // 也就是最朴素的情形——登记过的路径以会话配置界面为准（那里按会话选型问同一份清单）。
    let vm = crate::capabilities::workspace::api::vm_requirements(
        &crate::capabilities::workspace::api::VmInputs {
            base: None,
            qemu: None,
            probe: &crate::adapters::HostProbeAdapter,
        },
    );
    let doc = serde_json::json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "fence": { "fs": cap.fs, "net": cap.net, "tree": cap.tree, "note": cap.note },
        "externals": {
            "python": crate::adapters::host_probe::find_exe("python").map(|p| p.to_string_lossy().into_owned()),
            "node": crate::adapters::host_probe::find_exe("node").map(|p| p.to_string_lossy().into_owned()),
            "curl": crate::adapters::host_probe::find_exe("curl").map(|p| p.to_string_lossy().into_owned()),
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

/// 隐藏模式：按 KEY=VALUE 逐行打出运行期给工具进程的环境白名单（入参 = 守门进程那份 JSON）。
pub fn print_fence_env(args: &[String], flag: usize) -> i32 {
    let raw = args.get(flag + 1).cloned().unwrap_or_default();
    match crate::capabilities::tools::detail::confine::FenceJob::from_json(&raw) {
        Ok(job) => {
            for (k, v) in crate::capabilities::tools::detail::confine::fence_env(&job.spec) {
                println!("{}={}", k.to_string_lossy(), v.to_string_lossy());
            }
            0
        }
        Err(e) => {
            eprintln!("[围栏] {}", e);
            crate::capabilities::tools::detail::confine::FENCE_FAILED
        }
    }
}

/// 隐藏模式：只做机制验证，如实报三态（enforced / env-unavailable / broken），恒退出 0——
/// 判定归调用方（探针按性质决定 env-skip 还是失败）。入参 = 守门进程那份 JSON，`--` 之后是命令。
pub fn fence_verify(args: &[String], flag: usize) -> i32 {
    let raw = args.get(flag + 1).cloned().unwrap_or_default();
    let command = match args.iter().position(|a| a == "--") {
        Some(j) => args.get(j + 1).cloned().unwrap_or_default(),
        None => String::new(),
    };
    let job = match crate::capabilities::tools::detail::confine::FenceJob::from_json(&raw) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[围栏] {}", e);
            return crate::capabilities::tools::detail::confine::FENCE_FAILED;
        }
    };
    match crate::capabilities::tools::detail::confine::verify(&job.spec, &command) {
        crate::capabilities::tools::detail::confine::FenceVerdict::Enforced => {
            println!("enforced");
            0
        }
        crate::capabilities::tools::detail::confine::FenceVerdict::EnvUnavailable(why) => {
            println!("env-unavailable {}", why);
            0
        }
        crate::capabilities::tools::detail::confine::FenceVerdict::Broken(why) => {
            println!("broken {}", why);
            0
        }
    }
}

/// 入站契约（机器可读）：HTTP 路由目录的唯一定义（见 docs/architecture/contracts.md）。
pub fn print_routes() {
    println!("{}", crate::web::routes::catalog_json());
}
