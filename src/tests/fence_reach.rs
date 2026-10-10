//! 目的：外部 harness 的围栏可达范围——缺口 module.external-harness-fence-reach 的 T3 结论。
//! 管：PATH 名解析的程序目录可达 / 绝对路径的程序不自动放行（不可达即如实失败）/ 模块目录之外的运行时依赖不可达。
//! 不管：围栏机制本身（在 kernel）；平台差异（只在 unix 且本机文件系统围栏真能强制时跑，否则如实跳过）。
//! 联动：结论落在 MODULE_SPEC.md 的「外部可执行文件与可达范围」；缺口账见 tests/gaps.yaml。

#[cfg(unix)]
#[test]
fn fence_reach_for_external_harness() {
    use crate::kernel::api::FenceSpec;
    use crate::kernel::detail::confine;
    use crate::kernel::detail::process::ProcTools;
    use crate::kernel::ports::ProcessRunner;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过围栏可达范围契约");
        return;
    };
    let Some(py) = crate::tests::doubles::python() else {
        eprintln!("[探针] 本机没有可用的 python，跳过围栏可达范围契约");
        return;
    };
    if !confine::capability().fs {
        eprintln!("[探针] 本机文件系统围栏不能强制，跳过围栏可达范围契约");
        return;
    }

    let root = crate::tests::scratch("fence-reach");
    let module = root.join("module");
    let outside = root.join("outside");
    std::fs::create_dir_all(module.join("tools")).expect("建模块目录");
    std::fs::create_dir_all(&outside).expect("建模块外目录");

    // 模块内的脚本：能跑通 = 解释器（命令里按 PATH 名的那个程序）目录被放行。
    std::fs::write(module.join("tools").join("probe.py"), "print('PROBE-OK')\n")
        .expect("写模块内脚本");
    // 模块外的可执行文件：绝对路径的程序 token 不按 PATH 解析，不自动进可达范围。
    let outside_sh = outside.join("run.sh");
    std::fs::write(&outside_sh, "#!/bin/sh\necho OUTSIDE-OK\n").expect("写模块外脚本");
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&outside_sh, std::fs::Permissions::from_mode(0o755));
    }
    // 模块外的数据：模块目录之外的运行时依赖（缓存 / 数据）不该可达。
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, "SECRET\n").expect("写模块外数据");
    std::fs::write(
        module.join("tools").join("read_outside.py"),
        format!(
            "try:\n    open({:?}).read()\n    print('READ-OK')\nexcept Exception:\n    print('READ-DENIED')\n",
            secret.to_string_lossy()
        ),
    )
    .expect("写读取脚本");

    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    let fence = FenceSpec::standalone(&module, None, false);

    // 1) 命令里按 PATH 名的程序（python）目录被放行：模块工具跑通一次完整往返。
    let a = runner.run(&fence, &format!("{} tools/probe.py", py), "{}", &[], None);
    assert!(
        a.ok && a.output.contains("PROBE-OK"),
        "命令里按 PATH 名的程序应可达：{}",
        a.output
    );

    // 2) 绝对路径的程序不被自动放行：模块目录外的可执行文件跑不起来（如实失败，不静默）。
    let b = runner.run(&fence, &outside_sh.to_string_lossy(), "{}", &[], None);
    assert!(
        !b.ok,
        "模块目录外的绝对路径程序不该可达（要放进模块目录，或用 PATH 名）：{}",
        b.output
    );

    // 3) 模块目录之外的运行时数据不可达。
    let c = runner.run(
        &fence,
        &format!("{} tools/read_outside.py", py),
        "{}",
        &[],
        None,
    );
    assert!(
        c.output.contains("READ-DENIED"),
        "模块目录之外的数据不该可达：{}",
        c.output
    );

    let _ = std::fs::remove_dir_all(&root);
}
