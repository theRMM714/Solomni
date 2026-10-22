//! 目的：入口层的可观察契约——隐藏模式（自检 / 探针）与启动报告的围栏判据。
//! 管：构建的产品路径（查模块 / 查构建期能力 / 缓存 / 跑命令 / 入口复验 / 状态落盘）、
//!   `--doctor` 的事实本体、启动报告与 `--doctor` 共用同一份本机能力与本次实际。
//! 不管：CLI 参数解析（在 main.rs）、模块清单校验（在 `modules.rs`）、围栏机制本身（在 kernel）。
//! 联动：模块契约见 MODULE_SPEC 的「构建」；状态落 `.vm/build-state/<模块>.json`。

/// 目的：--work-gc 默认 dry-run、--yes 才删；只删不可达对象。
#[test]
fn work_gc_cli_dry_run_then_applies() {
    use crate::capabilities::workspace::api::{content_hash, Workspace};
    use crate::capabilities::workspace::detail::fs_workspace::FsWorkspace;
    use crate::capabilities::workspace::detail::fs_workstore::FsWorkStore;
    use crate::capabilities::workspace::ports::WorkStore;
    use crate::capabilities::workspace::service::WorkspaceService;
    use std::sync::Arc;

    let root = crate::tests::scratch("work-gc-cli");
    let dirs = Arc::new(FsWorkspace::new(root.join("session")));
    let store = Arc::new(FsWorkStore::new());
    let ws = WorkspaceService::new(dirs, store.clone());
    ws.prepare("w", &[]).unwrap();
    ws.work_commit_user("w", "keep.txt", b"keep", 1, 0).unwrap();
    let store_path = root.join("session").join("w").join(".work");
    let orphan = content_hash(b"orphan");
    store.write_object(&store_path, &orphan, b"orphan").unwrap();
    let keep = content_hash(b"keep");
    assert!(store_path.join("objects").join(&orphan).exists());

    let dry = crate::diagnostics::work_gc(&root, &["--work-gc".to_string(), "w".to_string()], 0);
    assert_eq!(dry, 0);
    assert!(
        store_path.join("objects").join(&orphan).exists(),
        "dry-run 不能删"
    );

    let applied = crate::diagnostics::work_gc(
        &root,
        &[
            "--work-gc".to_string(),
            "w".to_string(),
            "--yes".to_string(),
        ],
        0,
    );
    assert_eq!(applied, 0);
    assert!(
        !store_path.join("objects").join(&orphan).exists(),
        "apply 要删不可达对象"
    );
    assert!(
        store_path.join("objects").join(&keep).exists(),
        "可达对象必须保留"
    );
}

/// 目的：构建契约用的夹具模块——清单只声明 `build.command: tools/build`，**平台各一份脚本**并排放
///   （与 MODULE_SPEC「构建」里模块作者的跨平台写法、随仓 indexer 一致：unix 由 sh 直接跑 `tools/build`，
///   Windows 由 cmd 按 PATHEXT 落到 `tools/build.cmd`）。
/// 返回：模块工具声明的入口在**本平台**落成的产物路径（入口判据按壳的候选认，Windows 落 `.cmd`）。
/// 约束：脚本只写 **ASCII**——cmd 按控制台代码页解释批处理字节，非 ASCII 会吞掉下一行（见 MODULE_SPEC 的「跨平台」）。
fn host_build_module(module: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(module.join("tools")).expect("建模块目录");
    std::fs::write(
        module.join("module.yaml"),
        "id: hello\nbrief: 构建契约用例\nsystem: 你负责打招呼\nbuild:\n  command: tools/build\ntools:\n  hi:\n    command: build/hello\n",
    )
    .expect("写模块清单");
    // 构建命令每跑一次就往 `build/runs.txt` 记一行：`build/` 不进源指纹，所以它是"命令真的又跑了一次"的
    // 硬证据（比产物 mtime 可靠——Windows 的 `copy` 保留源文件时间戳，mtime 不前进并不等于没重编）。
    std::fs::write(
        module.join("tools").join("build"),
        "#!/bin/sh\nset -e\nmkdir -p build\nprintf '#!/bin/sh\\necho HELLO-OK\\n' > build/hello\nchmod +x build/hello\necho run >> build/runs.txt\n",
    )
    .expect("写 unix 构建脚本");
    // Windows 那份：cmd 按 PATHEXT 找到同名 `.cmd`（unix 那份它不看，反之亦然——两个文件并排就是跨平台写法）。
    std::fs::write(
        module.join("tools").join("build.cmd"),
        "@echo off\r\nif not exist build mkdir build\r\n> build\\hello.cmd echo @echo off\r\n>> build\\hello.cmd echo echo HELLO-OK\r\n>> build\\runs.txt echo run\r\n",
    )
    .expect("写 Windows 构建脚本");
    // unix 的脚本要能直接跑（产品在入口判据与执行时都按它判）。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = module.join("tools").join("build");
        let mut perm = std::fs::metadata(&script).expect("读权限位").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&script, perm).expect("加执行位");
    }
    let name = if cfg!(windows) { "hello.cmd" } else { "hello" };
    module.join("build").join(name)
}

/// 目的：构建命令真跑了几次——夹具脚本每跑一次就往 `build/runs.txt` 记一行。
fn build_runs(module: &std::path::Path) -> usize {
    std::fs::read_to_string(module.join("build").join("runs.txt"))
        .map(|text| text.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// 目的：构建期望失败的命令——两个平台各自的壳语法，都只写 ASCII。
#[cfg(unix)]
const FAILING_BUILD: &str = "echo boom >&2; exit 7";
#[cfg(windows)]
const FAILING_BUILD: &str = "echo boom 1>&2 & exit /b 7";

#[test]
fn host_build_makes_the_entry_and_hits_the_cache() {
    use crate::capabilities::modules::ports::ModuleSource;
    use crate::kernel::detail::process::ProcTools;
    use crate::kernel::ports::HostProbe;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过 --build 契约");
        return;
    };
    let root = crate::tests::scratch("host-build");
    let module = root.join("modules").join("hello");
    // 一个不需要编译器的"构建"：脚本写一个能直接跑的产物（两平台都跑得动，不赌宿主工具链）。
    let artifact = host_build_module(&module);
    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    // 第一次：真编出来（产物在场 + 能直接跑 + 状态落盘）。
    assert_eq!(
        crate::diagnostics::build_module(&root, "hello", &runner),
        0,
        "第一次构建要成功"
    );
    assert!(
        artifact.is_file(),
        "产物要落进模块目录：{}",
        artifact.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&artifact)
            .expect("读权限")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "产物要能直接跑（拿到 {:o}）", mode);
    }
    let state = root.join(".vm").join("build-state").join("hello.json");
    assert!(state.is_file(), "状态要落盘：{}", state.display());
    // 会话语义：这条入口此时应当放行（入口判据真在、真能跑）。
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    let scanned = crate::capabilities::modules::detail::FsModules::new(
        root.join("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan();
    assert!(
        crate::capabilities::vm::api::missing_entries_for(
            &scanned.modules,
            std::env::consts::OS,
            &|p| probe.is_executable(p)
        )
        .is_empty(),
        "构建完入口判据要放行"
    );
    // 第二次：指纹没变 → 跳过（产物不被重写，构建命令也不再跑）。
    let before = std::fs::metadata(&artifact)
        .expect("读 mtime")
        .modified()
        .expect("mtime");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(
        crate::diagnostics::build_module(&root, "hello", &runner),
        0,
        "第二次要跳过（仍算成功）"
    );
    let after = std::fs::metadata(&artifact)
        .expect("读 mtime")
        .modified()
        .expect("mtime");
    assert_eq!(before, after, "命中缓存时不该重编（产物不该被重写）");
    assert_eq!(build_runs(&module), 1, "命中缓存时构建命令不该再跑");
    // 源变了 → 真重编（构建命令又跑了一次）。
    std::fs::write(module.join("tools").join("src.txt"), "changed\n").expect("改源文件");
    assert_eq!(
        crate::diagnostics::build_module(&root, "hello", &runner),
        0,
        "改源后要重编"
    );
    assert_eq!(build_runs(&module), 2, "源变了构建命令要真重跑");
}

#[test]
fn host_build_failure_is_honest_and_leaves_no_state() {
    use crate::kernel::detail::process::ProcTools;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过 --build 失败契约");
        return;
    };
    let root = crate::tests::scratch("host-build-fail");
    let module = root.join("modules").join("bad");
    std::fs::create_dir_all(&module).expect("建模块目录");
    std::fs::write(
        module.join("module.yaml"),
        format!(
            "id: bad\nbrief: 构建失败用例\nsystem: 你负责失败\nbuild:\n  command: \"{}\"\ntools:\n  hi:\n    command: build/bad\n",
            FAILING_BUILD
        ),
    )
    .expect("写模块清单");
    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(
        crate::diagnostics::build_module(&root, "bad", &runner),
        1,
        "构建失败要如实报失败"
    );
    assert!(
        !root
            .join(".vm")
            .join("build-state")
            .join("bad.json")
            .is_file(),
        "失败不落状态（下次重试）"
    );
    // 构建期能力不在场：如实拒绝（点出缺哪个，不猜、不降级）。
    let needy = root.join("modules").join("needy");
    std::fs::create_dir_all(&needy).expect("建模块目录");
    std::fs::write(
        needy.join("module.yaml"),
        "id: needy\nbrief: 构建期能力不在场\nsystem: 你负责构建\nbuild:\n  command: tools/build\n  needs: [no-such-tool-xyz]\ntools:\n  hi:\n    command: build/hello\n",
    )
    .expect("写模块清单");
    assert_eq!(
        crate::diagnostics::build_module(&root, "needy", &runner),
        1,
        "构建期工具不在场要如实拒绝"
    );
    // 没声明 build 的模块：明确拒绝（不是静默成功）。
    let plain = root.join("modules").join("plain");
    std::fs::create_dir_all(&plain).expect("建模块目录");
    std::fs::write(
        plain.join("module.yaml"),
        "id: plain\nbrief: 没有构建段\nsystem: 你负责单干\ntools:\n  hi:\n    command: python tools/hi.py\n",
    )
    .expect("写模块清单");
    assert_eq!(
        crate::diagnostics::build_module(&root, "plain", &runner),
        2,
        "没声明 build 的模块要明确拒绝"
    );
}

#[test]
pub(crate) fn build_packages_take_guest_or_library() {
    use crate::capabilities::vm::api::Library;
    use crate::tests::doubles::{pkg, pkg_yaml};
    let lib = Library::build(
        vec![
            pkg_yaml(
                "id: cc\nversion: 13.2.0\nkind: system\nprovides_paths: [usr/bin, usr/lib]\nrequires: [binutils]",
            ),
            pkg("binutils", "2.42"),
            pkg("python", "3.12.4"),
        ],
        Vec::new(),
    );
    // guest 自带的跳过；库里有的一并带上传递 requires；都没有的如实报缺。
    let (pkgs, missing) = crate::diagnostics::build_packages(
        &["python".to_string(), "cc".to_string(), "rustc".to_string()],
        &lib,
        &["python".to_string()],
    );
    let ids: Vec<&str> = pkgs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "cc 与它的传递依赖 binutils：{:?}", ids);
    assert!(
        ids.contains(&"cc") && ids.contains(&"binutils"),
        "{:?}",
        ids
    );
    assert!(
        !ids.contains(&"python"),
        "guest 自带的运行能力不装载包：{:?}",
        ids
    );
    assert_eq!(missing, vec!["rustc".to_string()], "缺的能力要如实报");
    let cc = pkgs.iter().find(|p| p.id == "cc").expect("有 cc");
    assert_eq!(cc.kind, "system");
    assert_eq!(
        cc.provides_paths,
        vec!["usr/bin".to_string(), "usr/lib".to_string()]
    );
}

/// 目的：真机验收（默认跳过）：虚拟机档构建——在 guest 里编出产物，产物经挂载落回模块目录。
#[cfg(unix)]
#[test]
fn guest_build_makes_the_entry_in_the_guest() {
    if std::env::var("SOLOMNI_VM_LIVE").as_deref() != Ok("1") {
        eprintln!("[探针] 未开启虚拟机档构建的真机验收：设 SOLOMNI_VM_LIVE=1 后重跑");
        return;
    }
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let deps = crate::capabilities::provision::api::Provision::status(
        &crate::capabilities::provision::ProvisionService::new(&repo),
    )
    .unwrap_or_default();
    // 引擎与虚拟根不在场就如实跳过（真机验收只在装齐的机器上跑）。
    let has_qemu = deps
        .iter()
        .any(|d| d.present && d.id.starts_with("vm.qemu."));
    let has_guest = repo.join(".vm").join("guest").join("debian").is_dir();
    if !has_qemu || !has_guest {
        eprintln!("[探针] 本机没有可用的 QEMU 或虚拟根 debian，跳过虚拟机档构建验收");
        return;
    }
    // 夹具根借仓库的 `.vm`（虚拟根、QEMU、包库都在那儿）；跑完删链本身，不碰仓库那份。
    let root = crate::tests::scratch("guest-build");
    std::os::unix::fs::symlink(repo.join(".vm"), root.join(".vm")).expect("链 .vm");
    let module = root.join("modules").join("greet");
    std::fs::create_dir_all(&module).expect("建模块目录");
    std::fs::write(
        module.join("module.yaml"),
        "id: greet\nbrief: 虚拟机档构建用例\nsystem: 你负责打招呼\nbuild:\n  command: \"mkdir -p build && printf '#!/bin/sh\\necho GUEST-OK\\n' > build/hello && chmod +x build/hello\"\ntools:\n  hi:\n    command: build/hello\n",
    )
    .expect("写模块清单");
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
    let _live = crate::tests::doubles::live_guest_lock();
    let code = crate::diagnostics::build_module_in_guest(&root, "greet", "debian");
    // 先清理：状态文件写在借来的 `.vm` 里（别留下夹具的痕迹）。
    let _ = std::fs::remove_file(root.join(".vm").join("build-state").join("greet.json"));
    let _ = std::fs::remove_file(root.join(".vm"));
    assert_eq!(code, 0, "guest 里要能编出产物");
    let artifact = module.join("build").join("hello");
    assert!(
        artifact.is_file(),
        "产物要经挂载落回模块目录：{}",
        artifact.display()
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&artifact)
        .expect("读权限")
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "产物要能直接跑（拿到 {:o}）", mode);
}

/// 目的：`--doctor` 的事实里要能看到"现在在跑什么"：账上记着、进程已经没了的条目照样如实报出来（`alive:false`）。
#[test]
pub(crate) fn doctor_reports_the_recorded_processes() {
    let root = crate::tests::scratch("doctor-procs");
    std::fs::create_dir_all(root.join(".home")).expect("建 .home");
    let book = crate::kernel::detail::procs::ProcessBook::open(&root.join(".home"));
    book.record(crate::kernel::detail::procs::Entry {
        pid: 999_999,
        since: 12,
        purpose: "guest".to_string(),
        owner: "w-doctor".to_string(),
        marker: "solomni:w-doctor".to_string(),
        created: None,
        guard_note: Some("把子进程绑进作业对象失败".to_string()),
    });
    let doc = crate::diagnostics::doctor_json(&root);
    let procs = doc
        .get("processes")
        .and_then(|p| p.as_array())
        .expect("要有 processes");
    assert_eq!(procs.len(), 1, "账上那条要报出来");
    assert_eq!(
        procs[0].get("purpose").and_then(|v| v.as_str()),
        Some("guest")
    );
    assert_eq!(
        procs[0].get("owner").and_then(|v| v.as_str()),
        Some("w-doctor")
    );
    assert_eq!(
        procs[0].get("alive").and_then(|v| v.as_bool()),
        Some(false),
        "进程早没了就要如实报 false"
    );
    assert_eq!(
        procs[0].get("orphanRisk").and_then(|v| v.as_bool()),
        Some(true),
        "父死子亡没挂上的条目要标出来（这条会留成孤儿）"
    );
    assert_eq!(
        procs[0].get("guardNote").and_then(|v| v.as_str()),
        Some("把子进程绑进作业对象失败"),
        "原因要原样报出来"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：路径级围栏的「本次实际」按平台判据：只有 Windows 受 `sandbox.policy.windows.write_acl` 授权影响。
///   用合成能力值，不依赖本机机制；Windows 分支由 Windows CI 真跑。
#[test]
fn effective_path_fence_only_depends_on_authorization_on_windows() {
    use crate::kernel::detail::confine::Capability;
    let cap = Capability {
        fs: true,
        net: true,
        tree: true,
        note: "测试".to_string(),
    };
    #[cfg(windows)]
    {
        assert_eq!(
            cap.effective_paths(false),
            (false, false),
            "Windows 未授权就不写 ACL"
        );
        assert_eq!(cap.effective_paths(true), (true, true), "授权后按本机能力");
    }
    #[cfg(not(windows))]
    {
        assert_eq!(
            cap.effective_paths(false),
            (true, true),
            "Unix 的围栏在守门进程里自足，授权与否都不影响"
        );
        assert_eq!(cap.effective_paths(true), (true, true));
    }
}

/// 目的：启动报告与 `--doctor` 共用同一份本机能力；Unix 的「本次实际」与它一致，
///   Windows 未授权时 fs/net 为假（容器围栏要写 ACL）——钉住启动报告与 `--doctor` 的口径。
#[test]
fn startup_fence_report_and_doctor_share_the_same_capability() {
    let cap = crate::kernel::detail::confine::capability();
    let report = crate::FenceReport::new(
        cap.clone(),
        false,
        std::sync::Arc::new(crate::kernel::api::SandboxMode::permissive()),
    );
    let root = crate::tests::scratch("fence-report");
    let doc = crate::diagnostics::doctor_json_with(&root, cap.clone());
    let fence = doc.get("fence").expect("--doctor 要报围栏事实");
    assert_eq!(
        fence.get("fs").and_then(|v| v.as_bool()),
        Some(cap.fs),
        "--doctor 报的是本机能力"
    );
    assert_eq!(
        fence.get("net").and_then(|v| v.as_bool()),
        Some(cap.net),
        "--doctor 报的是本机能力"
    );
    assert_eq!(fence.get("tree").and_then(|v| v.as_bool()), Some(cap.tree));
    assert_eq!(report.cap.tree, cap.tree, "进程树围栏与授权无关");
    #[cfg(windows)]
    {
        assert!(
            !report.fs && !report.net,
            "Windows 未授权时不写 ACL，容器围栏不生效"
        );
        assert!(!report.write_allowed);
    }
    #[cfg(not(windows))]
    {
        assert_eq!(
            (report.fs, report.net),
            (cap.fs, cap.net),
            "Unix 的本次实际就是本机能力，要与 --doctor 一致"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
