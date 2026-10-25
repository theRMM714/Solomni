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
        "toolchains": {
            "profile": crate::entry::toolchains::PROFILE,
            "active": base.active.iter().map(|(id, version)| serde_json::json!({
                "id": id,
                "version": version,
            })).collect::<Vec<_>>(),
            "fallback": base.fallback.iter().map(|(id, f)| (id.clone(), serde_json::json!({
                "path": f.path.to_string_lossy(),
                "fenced": f.fenced,
                "why": f.why,
            }))).collect::<std::collections::BTreeMap<_, _>>(),
        },
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
fn build_vm(
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

/// 目的：隐藏模式——**构建一个模块声明的产物**（系统动作：在**目标环境**里跑 `module.yaml` 的 `build.command`）。
/// 参数：root = 产品根；args = 原始参数；i = `--build` 的位置（`--build=<模块 id>`；可选 `--guest=<虚拟根>`）。
/// 约束：只写项目内；本机档用**产品自己写的**受限围栏跑，虚拟机档在 guest 里跑（工具链是 guest 的编译包）——
///   两者都**不进 agent 的工具围栏**；模块契约与状态落点见 MODULE_SPEC 的「构建」与 `.vm/build-state/<模块>.json`。
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
        if guest.trim().is_empty() {
            eprintln!("[构建] --guest 要给虚拟根名（`.vm/guest/<名>`）");
            return 2;
        }
        return build_module_in_guest(root, &id, guest.trim());
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

/// 目的：构建一个模块（**本机档**：在宿主上编，产物给宿主用）——CLI 与用例共用。
/// 参数：root = 产品根；id = 模块 id；runner = 执行器（生产是守门进程，用例可注入替身）——
///   构建命令经它跑在**产品给的**围栏里（只写模块目录与状态目录、按声明的 net）。
pub fn build_module(
    root: &std::path::Path,
    id: &str,
    runner: &dyn crate::kernel::ports::ProcessRunner,
) -> i32 {
    let Some(module) = module_by_id(root, id) else {
        eprintln!("[构建] 没有模块「{}」（清单被拒收的话见 --doctor）", id);
        return 2;
    };
    let Some(decl) = module.manifest.build.clone() else {
        eprintln!("[构建] 模块「{}」没声明 build 段：在 module.yaml 里写 build.command（见 MODULE_SPEC 的「构建」）", id);
        return 2;
    };
    use crate::capabilities::packages::api::Packages as _;
    use crate::kernel::ports::HostProbe;
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    // 构建期能力：运行包优先，基础工具链 / 宿主 PATH 回退；没有会话定版，多版本取稳定序第一个。
    let base = crate::entry::toolchains::base_facts(root);
    let packages_lib = crate::capabilities::packages::service::PackagesService::with_base(
        std::sync::Arc::new(crate::capabilities::packages::detail::FsPackages::new(
            root.join("runtimes"),
        )),
        base.clone(),
    );
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let has_exe = |name: &str| probe.has_exe(name);
    let bin_ok = |path: &std::path::Path| probe.is_dir(path);
    let facts = crate::capabilities::packages::api::Facts {
        has_exe: &has_exe,
        bin_ok: &bin_ok,
        current_path: &current_path,
        base: &base.active,
        host_fallback: &base.fallback,
    };
    let requests =
        crate::capabilities::packages::api::requests_for(&decl.needs, &format!("build:{}", id));
    let plan = crate::capabilities::packages::api::plan_host_requests(
        &requests,
        &std::collections::BTreeMap::new(),
        &packages_lib.library(),
        crate::capabilities::packages::api::PinPolicy::First,
        &facts,
    );
    if !plan.problems.is_empty() {
        eprintln!(
            "[构建] 本机没有这些构建期能力：{}（装工具链、把适用于本机目标的包放进 runtimes/、自己编好放进来，或让作者发产物）",
            crate::capabilities::packages::api::diagnose_text(&plan.problems)
        );
        return 1;
    }
    let providers: Vec<String> = plan
        .providers
        .iter()
        .map(|(cap, p)| match p {
            crate::capabilities::packages::api::Provider::Base { id, version } => {
                format!("{}={}@{}", cap, id, version)
            }
            crate::capabilities::packages::api::Provider::Host => format!("{}=host", cap),
            crate::capabilities::packages::api::Provider::Package { id, version } => {
                format!("{}={}@{}", cap, id, version)
            }
        })
        .collect();
    let fingerprint = match source_fingerprint(&module.root, &decl, &providers) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[构建] 算源指纹失败：{}", e);
            return 1;
        }
    };
    let state_path = build_state_path(root, id);
    if build_is_fresh(
        &state_path,
        "host",
        std::env::consts::OS,
        &fingerprint,
        &module,
    ) {
        println!(
            "[构建] {} 已是最新，跳过（状态 {}）",
            id,
            state_path.display()
        );
        return 0;
    }
    // 跑构建：围栏由产品给（只写模块目录与状态目录；不写本机任何权限项）。
    // 网络是全局开关，模块的 build.net 只是“这次构建需要网络”的声明；两者取交集。
    let network = global_network_allowed(root) && decl.net;
    let state_dir = root.join(".vm").join("build-state");
    let home = state_dir.join(id).join("home");
    if let Err(e) = std::fs::create_dir_all(&home) {
        eprintln!("[构建] 建状态目录失败：{}", e);
        return 1;
    }
    let fence = crate::kernel::api::FenceSpec {
        agent: format!("build:{}", id),
        lease: String::new(),
        rw: vec![module.root.clone(), state_dir.join(id)],
        // 构建脚本可能调用项目内编译器（gcc/g++/zig），要放行它的安装根；否则 Landlock 会拒绝。
        ro: crate::entry::toolchains::active_roots(root),
        ro_tree: plan.roots.clone(),
        private: home,
        cwd: module.root.clone(),
        net: network,
    };
    println!(
        "[构建] {}：在模块根跑声明的构建命令（出站网络：{}）",
        id,
        if network { "放行" } else { "不放行" }
    );
    let out = crate::kernel::ports::ProcessRunner::run(
        runner,
        &fence,
        &decl.command,
        "{}",
        &[],
        plan.path.as_deref(),
        None,
    );
    print!("{}", out.output);
    if !out.ok {
        eprintln!("[构建] {} 的构建命令失败（不落状态，下次重试）", id);
        return 1;
    }
    finish_build(
        root,
        id,
        "host",
        std::env::consts::OS,
        &fingerprint,
        &module,
        &state_path,
    )
}

/// 目的：构建一个模块的**虚拟机档**产物——在 guest 里用它的编译包编，产物经挂载落回模块目录。
/// 参数：root = 产品根；id = 模块 id；guest = 虚拟根名（`.vm/guest/<名>`）。
/// 约束：guest 用完即销毁（会话级可写层）；模块目录以读写挂进去（产物落在宿主那份里）；
///   `needs` 由 guest 自带或包库满足（prefix 的 bin 进 PATH、system 的载荷叠写进系统路径）。
pub fn build_module_in_guest(root: &std::path::Path, id: &str, guest: &str) -> i32 {
    let Some(module) = module_by_id(root, id) else {
        eprintln!("[构建] 没有模块「{}」（清单被拒收的话见 --doctor）", id);
        return 2;
    };
    let Some(decl) = module.manifest.build.clone() else {
        eprintln!("[构建] 模块「{}」没声明 build 段：在 module.yaml 里写 build.command（见 MODULE_SPEC 的「构建」）", id);
        return 2;
    };
    use crate::kernel::ports::HostProbe;
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    // 目标环境：虚拟根必须可用（名字 → 一格目录；布局不对如实拒绝）。
    let dir = match crate::capabilities::vm::api::resolve_guest(root, guest) {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!(
                "[构建] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes（或换一个 --guest）",
                guest
            );
            return 2;
        }
    };
    let image = match crate::capabilities::vm::api::classify_base(&dir, &probe) {
        Ok(i) => i,
        Err(why) => {
            eprintln!("[构建] 虚拟根「{}」不可用（{}）", guest, why);
            return 1;
        }
    };
    // 引擎与共享工具从**补齐台账**取（与 provision 同源；缺 QEMU 就如实拒绝）。
    let deps = crate::capabilities::provision::api::Provision::status(
        &crate::capabilities::provision::ProvisionService::new(root),
    )
    .unwrap_or_default();
    let pick = |prefix: &str| {
        deps.iter()
            .find(|d| d.present && d.id.starts_with(prefix))
            .map(|d| d.probe.clone())
    };
    let engine_path = pick("vm.qemu.");
    if engine_path.is_none() {
        eprintln!("[构建] 没有可用的 QEMU：先 solomni --provision --ensure --yes");
        return 1;
    }
    let share_tool = if std::env::consts::OS == "windows" {
        pick("vm.smbd.")
    } else {
        pick("vm.virtiofsd.")
    };
    // 构建期能力：guest 自带，或包库（prefix / system 两种装配都已就位）。
    let vm = build_vm(root);
    let packages_lib =
        crate::capabilities::packages::service::PackagesService::new(std::sync::Arc::new(
            crate::capabilities::packages::detail::FsPackages::new(root.join("runtimes")),
        ));
    let platform = crate::capabilities::vm::api::guest_platform(image.os, image.arch);
    let eligible = crate::capabilities::packages::api::Library::for_target(
        &crate::capabilities::packages::api::Packages::library(&packages_lib),
        &crate::capabilities::packages::api::Target::guest(&platform),
    );
    let (packages, missing) =
        crate::capabilities::packages::api::build_packages(&decl.needs, &eligible, &image.runtimes);
    if !missing.is_empty() {
        eprintln!(
            "[构建] guest「{}」里没有这些构建期能力，包库也没提供：{}（把适用于该 guest 目标的包放进 runtimes/，或给虚拟根装一个）",
            guest,
            missing.join("、")
        );
        return 1;
    }
    let providers: Vec<String> = packages
        .iter()
        .map(|p| format!("{}@{}", p.id, p.version))
        .collect();
    let fingerprint = match source_fingerprint(&module.root, &decl, &providers) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[构建] 算源指纹失败：{}", e);
            return 1;
        }
    };
    let state_path = build_state_path(root, id);
    if build_is_fresh(&state_path, guest, image.os.as_str(), &fingerprint, &module) {
        println!(
            "[构建] {} 在「{}」上已是最新，跳过（状态 {}）",
            id,
            guest,
            state_path.display()
        );
        return 0;
    }
    // 挂载：模块目录（读写，产物经它落回宿主）+ 已挑中的包（prefix 挂前缀、system 挂专用只读根）。
    let network = global_network_allowed(root) && decl.net;
    let lease = format!("build-{}", id);
    let plan = crate::capabilities::vm::api::ExecPlan {
        tier: crate::kernel::api::Tier::Vm,
        net: network,
        guest: Some(guest.to_string()),
        packages,
    };
    let fence = crate::kernel::api::FenceSpec {
        agent: format!("build:{}", id),
        lease: lease.clone(),
        rw: vec![module.root.clone()],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: std::path::PathBuf::new(),
        cwd: module.root.clone(),
        net: network,
    };
    let mounts = crate::capabilities::vm::api::mounts_from(&plan, &fence, &image.bridge());
    let spec = crate::capabilities::vm::api::GuestSpec {
        lease: lease.clone(),
        image: image.clone(),
        mounts,
        net: network,
        engine_path,
        share_path: share_tool.clone(),
        // 构建要求**挂载式**共享（产物要落回宿主那份）：这里只用用户链，copy 不在其中。
        shares: crate::capabilities::vm::api::share_chain(
            std::env::consts::OS,
            share_tool.as_deref(),
            &crate::kernel::detail::host_probe::HostProbeAdapter,
        ),
        seed: None,
        isolated: true,
    };
    println!(
        "[构建] {}：在 guest「{}」里跑声明的构建命令（出站网络：{}）",
        id,
        guest,
        if network { "放行" } else { "不放行" }
    );
    if let Err(e) = vm.guest_start(&spec) {
        eprintln!("[构建] 起 guest 失败：{}", e);
        return 1;
    }
    if !vm.guest_wait_agent(&lease, std::time::Duration::from_secs(15 * 60)) {
        vm.guest_stop(&lease);
        eprintln!("[构建] 等 guest 的 qemu-guest-agent 超时");
        return 1;
    }
    // 先把编译包接进 guest（prefix 的 bin 进 PATH、system 的载荷叠写进系统路径），再跑构建。
    for script in crate::capabilities::vm::api::package_setup_scripts(&plan) {
        let _ = vm.guest_run(
            &lease,
            &crate::capabilities::vm::api::GuestRun {
                command: script,
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
            },
        );
    }
    let out = vm.guest_run(
        &lease,
        &crate::capabilities::vm::api::GuestRun {
            command: decl.command.clone(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: module.root.display().to_string(),
        },
    );
    vm.guest_stop(&lease);
    print!("{}", out.output);
    if !out.ok {
        eprintln!(
            "[构建] {} 在 guest「{}」里的构建命令失败（不落状态，下次重试）",
            id, guest
        );
        return 1;
    }
    finish_build(
        root,
        id,
        guest,
        image.os.as_str(),
        &fingerprint,
        &module,
        &state_path,
    )
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

/// 目的：构建状态的落点（`.vm/build-state/<模块>.json`）——本机档与各虚拟根**共用一份**，靠 `target` 区分。
fn build_state_path(root: &std::path::Path, id: &str) -> std::path::PathBuf {
    root.join(".vm")
        .join("build-state")
        .join(format!("{}.json", id))
}

/// 目的：缓存命中吗——目标与源指纹都得一致，且入口都在场（否则等于没编）。
fn build_is_fresh(
    state_path: &std::path::Path,
    target: &str,
    target_os: &str,
    fingerprint: &str,
    module: &crate::capabilities::modules::api::Module,
) -> bool {
    let Some(prev) = read_build_state(state_path) else {
        return false;
    };
    prev.target == target
        && prev.fingerprint == fingerprint
        && entries_left(module, target_os).is_empty()
}

/// 目的：入口复验（构建完必须真在、真能跑）；返回还缺哪些入口。
/// 参数：target_os = 这次构建的**目标平台**（宿主构建 = 宿主；在虚拟根里构建 = 那个 guest 的 os）——
///   与建会话时的入口判据同一把尺子，否则"宿主编的产物"会被当成"guest 里能跑"。
fn entries_left(
    module: &crate::capabilities::modules::api::Module,
    target_os: &str,
) -> Vec<crate::capabilities::vm::api::MissingEntry> {
    use crate::kernel::ports::HostProbe;
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    crate::capabilities::vm::api::missing_entries_for(
        std::slice::from_ref(module),
        target_os,
        &|p| {
            probe.is_executable(p)
                && (target_os == std::env::consts::OS
                    || crate::capabilities::vm::api::runs_on_target(p, target_os))
        },
    )
}

/// 目的：构建的收尾——复验入口、落状态、如实报成败（本机档与虚拟机档共用）。
fn finish_build(
    _root: &std::path::Path,
    id: &str,
    target: &str,
    target_os: &str,
    fingerprint: &str,
    module: &crate::capabilities::modules::api::Module,
    state_path: &std::path::Path,
) -> i32 {
    let left = entries_left(module, target_os);
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
        target: target.to_string(),
        fingerprint: fingerprint.to_string(),
    };
    let text = match serde_json::to_string_pretty(&state) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[构建] 状态序列化失败：{}", e);
            return 1;
        }
    };
    if let Some(dir) = state_path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("[构建] 建状态目录失败（{}）：{}", dir.display(), e);
            return 1;
        }
    }
    if let Err(e) = std::fs::write(
        state_path,
        format!(
            "{}
",
            text
        ),
    ) {
        eprintln!("[构建] 写状态失败（{}）：{}", state_path.display(), e);
        return 1;
    }
    println!(
        "[构建] {} 构建完成（目标 {}；状态 {}）",
        id,
        target,
        state_path.display()
    );
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
    providers: &[String],
) -> Result<String, String> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(decl.command.as_bytes());
    buf.push(0);
    buf.extend_from_slice(decl.needs.join(",").as_bytes());
    buf.push(0);
    buf.extend_from_slice(providers.join(",").as_bytes());
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
