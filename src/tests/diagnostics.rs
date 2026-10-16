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
