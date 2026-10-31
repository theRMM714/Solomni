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

/// 自检：本机事实（平台 + 围栏能力 + 外部解释器）。只报事实，不猜、不改任何东西（围栏自检那个临时目录除外）。
/// 参数：root = 产品根——虚拟机档要按它读"项目内已装了什么"（与会话判据同一份来源，不另编一套）。
pub fn doctor(root: &std::path::Path) -> i32 {
    println!("{}", doctor_json(root));
    0
}

/// 目的：自检的事实本体（`--doctor` 打印它；用例也读它——不必抓 stdout）。
pub fn doctor_json(root: &std::path::Path) -> serde_json::Value {
    doctor_json_with(root, crate::kernel::detail::confine::capability())
}

/// 目的：自检事实本体；本机能力由调用方传入——生产取一次 `capability()`，用例复用同一份值，
///   不必再触发一次围栏自检（Windows 的自检会写临时目录 ACL）。
pub(crate) fn doctor_json_with(
    root: &std::path::Path,
    cap: crate::kernel::detail::confine::Capability,
) -> serde_json::Value {
    // 虚拟机档的逐项前置（**只读事实**）：项目内补齐过就按那一份问，否则按 PATH 问（与核心同一把尺子）。
    let installed = crate::capabilities::provision::api::Provision::status(
        &crate::capabilities::provision::ProvisionService::new(root),
    )
    .unwrap_or_default();
    let installed_at = |prefix: &str| {
        installed
            .iter()
            .find(|d| d.present && d.id.starts_with(prefix))
            .map(|d| root.join(&d.probe).to_string_lossy().into_owned())
    };
    let qemu = installed_at("vm.qemu.");
    let share_tool = installed_at(if cfg!(windows) {
        "vm.smbd."
    } else {
        "vm.virtiofsd."
    });
    let vm =
        crate::capabilities::vm::api::vm_requirements(&crate::capabilities::vm::api::VmInputs {
            qemu: qemu.as_deref(),
            share_tool: share_tool.as_deref(),
            probe: &crate::kernel::detail::HostProbeAdapter,
        });
    // 长驻子进程：账本里还活着的那几条（`--doctor` 只报事实，不动手收）。
    let book = crate::kernel::detail::procs::ProcessBook::open(&root.join(".home"));
    let procs: Vec<serde_json::Value> = book
        .list()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "pid": e.pid,
                "alive": crate::kernel::detail::procs::alive(e.pid),
                "purpose": e.purpose,
                "owner": e.owner,
                "since": e.since,
                "orphanRisk": e.guard_note.is_some(),
                "guardNote": e.guard_note,
            })
        })
        .collect();
    let base = crate::entry::toolchains::base_facts(root);
    let doc = serde_json::json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "fence": { "fs": cap.fs, "net": cap.net, "tree": cap.tree, "note": cap.note },
        "externals": {
            "python": crate::kernel::detail::host_probe::find_exe("python").map(|p| p.to_string_lossy().into_owned()),
            "node": crate::kernel::detail::host_probe::find_exe("node").map(|p| p.to_string_lossy().into_owned()),
            "curl": crate::kernel::detail::host_probe::find_exe("curl").map(|p| p.to_string_lossy().into_owned()),
        },
        "toolchains": crate::entry::toolchains::json(&base),
        "processes": procs,
        "vm_tier": {
            "available": vm.iter().all(|r| r.met),
            "requirements": vm.iter().map(|r| serde_json::json!({
                "id": r.id, "met": r.met, "detail": r.detail, "how": r.how,
            })).collect::<Vec<_>>(),
        },
    });
    doc
}

/// 目的：虚拟机档构建用的 vm 能力面：**只持挂载式**共享档（virtiofs / 9p / smb）。
/// 参数：root = 产品根。
/// 约束：模块目录是读写挂进去的，产物才能落回宿主那份——构建只持挂载式用户档；
///   `copy` 是开发/应急通道，不装配；`none` 只给供给编排。
pub(crate) fn build_vm(
    root: &std::path::Path,
) -> std::sync::Arc<dyn crate::capabilities::vm::api::Vm + Send + Sync> {
    let shares: Vec<
        std::sync::Arc<dyn crate::capabilities::vm::ports::ShareBackend + Send + Sync>,
    > = {
        let book = crate::kernel::detail::procs::ProcessBook::open(&root.join(".home"));
        vec![
            std::sync::Arc::new(
                crate::capabilities::vm::detail::VirtioFsShare::new()
                    .with_book(std::sync::Arc::clone(&book)),
            ),
            std::sync::Arc::new(crate::capabilities::vm::detail::NinePShare::new()),
            std::sync::Arc::new(crate::capabilities::vm::detail::SmbShare::new().with_book(book)),
        ]
    };
    std::sync::Arc::new(crate::capabilities::vm::service::VmService::new(
        std::sync::Arc::new(
            crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(
                root.join(".vm").join("overlays"),
            ),
        ),
        shares,
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
        // 与网页补齐同一条装配：只持内部 `none`，不拿会话 vm 顶替。
        let p = crate::capabilities::provision::ProvisionService::with_vm(
            root,
            crate::provision_vm(crate::kernel::detail::procs::ProcessBook::open(
                &root.join(".home"),
            )),
        );
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
/// 返回：退出码（0 成功；1 构建失败或环境不齐；2 用法错、没有该模块、或模块没有 `build` 段）。
/// 约束：用例在 `builds` 能力（本机档用产品围栏、虚拟机档在 guest 里编）；这里只解析参数、装配能力面与打印回执。
pub fn build(root: &std::path::Path, args: &[String], i: usize) -> i32 {
    use crate::capabilities::builds::api::Target;
    let id = match args[i].strip_prefix("--build=") {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => args.get(i + 1).cloned().unwrap_or_default(),
    };
    if id.is_empty() {
        eprintln!("用法：solomni --build=<模块 id> [--guest=<虚拟根>]");
        return 2;
    }
    let guest = args.iter().find_map(|a| a.strip_prefix("--guest="));
    if let Some(g) = guest {
        if g.trim().is_empty() {
            eprintln!("[构建] --guest 要给虚拟根名（`.vm/guest/<名>`）");
            return 2;
        }
    }
    let Some(module) = module_by_id(root, &id) else {
        eprintln!("[构建] 没有模块「{}」（清单被拒收的话见 --doctor）", id);
        return 2;
    };
    let Some(decl) = module.manifest.build.clone() else {
        eprintln!(
            "[构建] 模块「{}」没声明 build 段：在 module.yaml 里写 build.command（见 MODULE_SPEC 的「构建」）",
            id
        );
        return 2;
    };
    let target = match guest {
        Some(g) => Target::Guest(g.trim().to_string()),
        None => Target::Host,
    };
    let builds = match build_service(root) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[构建] {}", e);
            return 1;
        }
    };
    let net = global_network_allowed(root) && decl.net;
    let at = match &target {
        Target::Host => "在模块根".to_string(),
        Target::Guest(g) => format!("在 guest「{}」里", g),
    };
    println!(
        "[构建] {}：{}跑声明的构建命令（出站网络：{}）",
        id,
        at,
        if net { "放行" } else { "不放行" }
    );
    match crate::capabilities::builds::api::Builds::build(&builds, &module, &target, net) {
        Ok(b) if b.skipped => {
            println!("[构建] {} 在目标 {} 上已是最新，跳过", id, b.target);
            0
        }
        Ok(b) => {
            print!("{}", b.output);
            println!("[构建] {} 构建完成（目标 {}）", id, b.target);
            0
        }
        Err(e) => {
            eprintln!("[构建] {}", e);
            1
        }
    }
}

/// 目的：命令行构建用的能力面——包库、虚拟机、补齐、执行器与基础工具链安装根按产品根现场装配。
/// 参数：root = 产品根。
/// 返回：可调用的构建能力面。
/// 错误：执行器起不来。
fn build_service(
    root: &std::path::Path,
) -> Result<crate::capabilities::builds::service::BuildsService, String> {
    use std::sync::Arc;
    let packages: Arc<dyn crate::capabilities::packages::api::Packages + Send + Sync> = Arc::new(
        crate::capabilities::packages::service::PackagesService::with_base(
            Arc::new(crate::capabilities::packages::detail::FsPackages::new(
                root.join("runtimes"),
            )),
            crate::entry::toolchains::base_facts(root),
        ),
    );
    let provision: Arc<dyn crate::capabilities::provision::api::Provision + Send + Sync> =
        Arc::new(crate::capabilities::provision::ProvisionService::new(root));
    let runner: Arc<dyn crate::kernel::ports::ProcessRunner + Send + Sync> =
        Arc::new(build_runner(root)?);
    Ok(crate::capabilities::builds::service::BuildsService::new(
        root.to_path_buf(),
        packages,
        build_vm(root),
        provision,
        runner,
        Arc::new(crate::kernel::detail::confine::FenceHostAdapter::new(
            root.join(".home"),
        )),
        Arc::new(crate::kernel::detail::host_probe::HostProbeAdapter),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::entry::toolchains::active_roots(root),
    ))
}

/// 目的：按模块 id 扫出模块（扫盘；清单被拒收的在这里就是"找不到"，细节看 `--doctor`）。
fn module_by_id(
    root: &std::path::Path,
    id: &str,
) -> Option<crate::capabilities::modules::api::Module> {
    use crate::capabilities::modules::ports::ModuleSource;
    crate::capabilities::modules::detail::FsModules::new(
        root.join("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan()
    .modules
    .into_iter()
    .find(|m| m.manifest.id == id)
}

/// 目的：构建期的全局网络开关（与主程序同一份 settings.yaml；读不到/非法按拒绝）。
fn global_network_allowed(root: &std::path::Path) -> bool {
    std::fs::read_to_string(root.join(".home").join("settings.yaml"))
        .ok()
        .and_then(|t| {
            yaml_serde::from_str::<crate::capabilities::registry::api::AppSettings>(&t).ok()
        })
        .map(|a| a.sandbox.policy.network)
        .unwrap_or(false)
}

/// 目的：构建用的执行器——与外层工具执行同一套守门进程机制（围栏在它里面装）。
fn build_runner(
    root: &std::path::Path,
) -> Result<crate::kernel::detail::process::ProcTools, String> {
    let texts: std::sync::Arc<dyn crate::kernel::ports::ProcessTexts> =
        std::sync::Arc::new(crate::kernel::detail::process::CodeProcessTexts);
    Ok(crate::kernel::detail::process::ProcTools::new(
        std::env::current_exe().unwrap_or_default(),
        texts,
        root.join(".home"),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ))
}
