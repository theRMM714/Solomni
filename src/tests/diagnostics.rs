//! 目的：隐藏模式（自检 / 探针）的契约——`--build` 真能编出产物、命中缓存、失败如实报。
//! 管：构建的产品路径（查模块 / 查构建期能力 / 缓存 / 跑命令 / 入口复验 / 状态落盘）。
//! 不管：CLI 参数解析（在 main.rs）、模块清单校验（在 `modules.rs`）、围栏机制本身（在 kernel）。
//! 联动：模块契约见 MODULE_SPEC 的「构建」；状态落 `.vm/build-state/<模块>.json`。

#[cfg(unix)]
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
    std::fs::create_dir_all(module.join("tools")).expect("建模块目录");
    // 一个不需要编译器的"构建"：写一个可执行脚本当产物（unix 上哪台机器都跑得动）。
    std::fs::write(
        module.join("module.yaml"),
        "id: hello\nbrief: 构建契约用例\nsystem: 你负责打招呼\nbuild:\n  command: \"mkdir -p build && printf '#!/bin/sh\\necho HELLO-OK\\n' > build/hello && chmod +x build/hello\"\ntools:\n  hi:\n    command: build/hello\n",
    )
    .expect("写模块清单");
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
    let artifact = module.join("build").join("hello");
    assert!(
        artifact.is_file(),
        "产物要落进模块目录：{}",
        artifact.display()
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&artifact)
        .expect("读权限")
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "产物要能直接跑（拿到 {:o}）", mode);
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
        crate::capabilities::vm::api::missing_entries(&scanned.modules, &|p| probe
            .is_executable(p))
        .is_empty(),
        "构建完入口判据要放行"
    );
    // 第二次：指纹没变 → 跳过（产物不被重写，mtime 不动）。
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
    // 源变了 → 真重编（mtime 前进）。
    std::fs::write(module.join("tools").join("src.txt"), "changed\n").expect("改源文件");
    assert_eq!(
        crate::diagnostics::build_module(&root, "hello", &runner),
        0,
        "改源后要重编"
    );
    let after2 = std::fs::metadata(&artifact)
        .expect("读 mtime")
        .modified()
        .expect("mtime");
    assert!(after2 > after, "源变了要真重编");
}

#[cfg(unix)]
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
        "id: bad\nbrief: 构建失败用例\nsystem: 你负责失败\nbuild:\n  command: \"echo boom >&2; exit 7\"\ntools:\n  hi:\n    command: build/bad\n",
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
