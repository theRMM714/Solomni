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
    // 虚拟机档的逐项前置（**只读事实**）：这里按"没登记 QEMU、没选虚拟根"问一次，
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

/// 目的：隐藏模式——**构建一个模块声明的产物**（系统动作：在目标环境里跑 `module.yaml` 的 `build.command`）。
/// 参数：root = 产品根；args = 原始参数；i = `--build` 的位置（`--build=<模块 id>`；可选 `--guest=<虚拟根>`）。
/// 约束：只写项目内；本机档用**产品自己写的**受限围栏跑（只写模块目录与状态目录、按声明的 net），
///   工具链只是宿主 PATH 上的东西——**不进 agent 的工具围栏**；
///   模块契约与状态落点见 MODULE_SPEC 的「构建」与 `.vm/build-state/<模块>.json`。
pub fn build(root: &std::path::Path, args: &[String], i: usize) -> i32 {
    let id = match args[i].strip_prefix("--build=") {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => args.get(i + 1).cloned().unwrap_or_default(),
    };
    if id.is_empty() {
        eprintln!("用法：solomni --build=<模块 id> [--guest=<虚拟根>]");
        return 2;
    }
    if let Some(guest) = args.iter().find_map(|a| a.strip_prefix("--guest=")) {
        eprintln!(
            "[构建] 在虚拟根（{}）里构建还没接线：本机档编出来的是**宿主 ABI**，guest 里不一定能跑——\n        \
             先按模块 README 在**目标环境**里编一次，或等缺口 module.build-in-guest 接线（tests/gaps.yaml）",
            guest
        );
        return 2;
    }
    let runner = match build_runner(root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[构建] 执行器起不来：{}", e);
            return 1;
        }
    };
    build_module(root, &id, &runner)
}

/// 目的：构建一个模块（CLI 与用例共用）：查模块、查构建期能力、看缓存、跑命令、复验入口、落状态。
/// 参数：root = 产品根；id = 模块 id；runner = 执行器（生产是守门进程，用例可注入替身）——
///   构建命令经它跑在**产品给的**围栏里（只写模块目录与状态目录、按声明的 net）。
pub fn build_module(
    root: &std::path::Path,
    id: &str,
    runner: &dyn crate::kernel::ports::ProcessRunner,
) -> i32 {
    use crate::capabilities::modules::ports::ModuleSource;
    let roster = crate::capabilities::modules::detail::FsModules::new(
        root.join("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan();
    let Some(module) = roster.modules.iter().find(|m| m.manifest.id == id) else {
        eprintln!("[构建] 没有模块「{}」（清单被拒收的话见 --doctor）", id);
        return 2;
    };
    let Some(decl) = module.manifest.build.clone() else {
        eprintln!("[构建] 模块「{}」没声明 build 段：在 module.yaml 里写 build.command（见 MODULE_SPEC 的「构建」）", id);
        return 2;
    };
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    use crate::kernel::ports::HostProbe;
    // 构建期能力：本机档就是宿主 PATH 上有没有那个工具（包库只服务虚拟机档）。
    let missing: Vec<&str> = decl
        .needs
        .iter()
        .filter(|c| !probe.has_exe(c))
        .map(|c| c.as_str())
        .collect();
    if !missing.is_empty() {
        eprintln!(
            "[构建] 本机没有这些构建期工具：{}（装工具链、自己编好放进来，或让作者发产物）",
            missing.join("、")
        );
        return 1;
    }
    let state_dir = root.join(".vm").join("build-state");
    let state_path = state_dir.join(format!("{}.json", id));
    let fingerprint = match source_fingerprint(&module.root, &decl) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[构建] 算源指纹失败：{}", e);
            return 1;
        }
    };
    let entries = || {
        crate::capabilities::vm::api::missing_entries(std::slice::from_ref(module), &|p| {
            probe.is_executable(p)
        })
    };
    if let Some(prev) = read_build_state(&state_path) {
        if prev.fingerprint == fingerprint && entries().is_empty() {
            println!(
                "[构建] {} 已是最新，跳过（状态 {}）",
                id,
                state_path.display()
            );
            return 0;
        }
    }
    // 跑构建：围栏由产品给（只写模块目录与状态目录；不写本机任何权限项）。
    let home = state_dir.join(id).join("home");
    if let Err(e) = std::fs::create_dir_all(&home) {
        eprintln!("[构建] 建状态目录失败：{}", e);
        return 1;
    }
    let fence = crate::kernel::api::FenceSpec {
        agent: format!("build:{}", id),
        lease: String::new(),
        rw: vec![module.root.clone(), state_dir.join(id)],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: home,
        cwd: module.root.clone(),
        net: decl.net,
    };
    println!(
        "[构建] {}：在模块根跑声明的构建命令（出站网络：{}）",
        id,
        if decl.net { "放行" } else { "不放行" }
    );
    let out =
        crate::kernel::ports::ProcessRunner::run(runner, &fence, &decl.command, "{}", &[], None);
    print!("{}", out.output);
    if !out.ok {
        eprintln!("[构建] {} 的构建命令失败（不落状态，下次重试）", id);
        return 1;
    }
    // 构建完了入口必须真在：否则等于白编——入口判据不认，会话照样起不来。
    let left = entries();
    if !left.is_empty() {
        eprintln!(
            "[构建] 命令跑通了，但入口仍不在场：{}——检查 build.command 把产物写到哪了",
            left.iter()
                .map(|e| format!("{}（工具 {}）", e.command, e.tool))
                .collect::<Vec<_>>()
                .join("、")
        );
        return 1;
    }
    let state = BuildState {
        module: id.to_string(),
        target: "host".to_string(),
        fingerprint,
    };
    let text = match serde_json::to_string_pretty(&state) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[构建] 状态序列化失败：{}", e);
            return 1;
        }
    };
    if let Err(e) = std::fs::write(&state_path, format!("{}\n", text)) {
        eprintln!("[构建] 写状态失败（{}）：{}", state_path.display(), e);
        return 1;
    }
    println!("[构建] {} 构建完成（状态 {}）", id, state_path.display());
    0
}

/// 目的：构建状态（`.vm/build-state/<模块>.json`）——指纹一致且入口在场就跳过重编。
#[derive(serde::Serialize, serde::Deserialize)]
struct BuildState {
    module: String,
    target: String,
    fingerprint: String,
}

/// 目的：读构建状态；读不到（首次）或格式不认识都给 None（那就重编）。
fn read_build_state(path: &std::path::Path) -> Option<BuildState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 目的：模块的**源指纹**（构建命令 + 能力 + 网络 + 全部源文件的相对路径与内容）——变了才重编。
/// 约束：跳过 `build/`（产物本身）与 `userdata/`（跨任务私有状态，不是源）。
fn source_fingerprint(
    module_root: &std::path::Path,
    decl: &crate::capabilities::modules::api::BuildDecl,
) -> Result<String, String> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(decl.command.as_bytes());
    buf.push(0);
    buf.extend_from_slice(decl.needs.join(",").as_bytes());
    buf.push(0);
    buf.push(if decl.net { b'1' } else { b'0' });
    buf.push(0);
    collect_sources(module_root, module_root, &mut buf)?;
    Ok(crate::capabilities::workspace::api::content_hash(&buf))
}

/// 目的：把模块根下的源文件按相对路径 + 内容收进指纹缓冲；`build/` 与 `userdata/` 跳过。
fn collect_sources(
    root: &std::path::Path,
    dir: &std::path::Path,
    buf: &mut Vec<u8>,
) -> Result<(), String> {
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("读 {} 失败：{}", dir.display(), e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let top = rel.split(['/', '\\']).next().unwrap_or("");
        if top == "build" || top == "userdata" {
            continue;
        }
        if path.is_dir() {
            collect_sources(root, &path, buf)?;
            continue;
        }
        buf.extend_from_slice(rel.as_bytes());
        buf.push(0);
        buf.extend_from_slice(
            &std::fs::read(&path).map_err(|e| format!("读 {} 失败：{}", path.display(), e))?,
        );
        buf.push(0);
    }
    Ok(())
}

/// 目的：构建用的执行器——与外层工具执行同一套守门进程机制（围栏在它里面装）。
fn build_runner(
    root: &std::path::Path,
) -> Result<crate::kernel::detail::process::ProcTools, String> {
    let source =
        crate::capabilities::prompt::detail::yaml_prompts::YamlPrompts::new(root.join("prompts"));
    let prompt = crate::capabilities::prompt::service::load(&source)?;
    let texts: std::sync::Arc<dyn crate::kernel::ports::ProcessTexts> = std::sync::Arc::new(
        crate::capabilities::tools::detail::PromptProcessTexts::new(prompt.tools()),
    );
    Ok(crate::kernel::detail::process::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        texts,
        root.join(".home"),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ))
}
