//! 目的：视图盘与外部工作区机器级一次性只读的真机契约。
//! 管：建 profile、盘符映射与写 ACL 的用例，--fence-live 且本机能改 ACL 才跑。
//! 不管：容器内往返（见 `container.rs`）、根内授权形状（见 `grant.rs`）。
//! 联动：映射在 `src/kernel/detail/view_drive.rs`，一次性只读在 `windows/machine.rs`。

use super::*;

/// 【真机视图盘】产品根映射成盘符后，容器 cwd 走视图写法、命令与产物都落在真实路径：
/// 这是「根外祖先链一级都不写」的全部依据——容器里的路径没有产品根之外的前缀，就只差映射根那一级的读属性。
/// 约束：只在 --fence-live（SOLOMNI_FENCE_LIVE=1）且本机能改 ACL 时跑；会建 AppContainer profile 与盘符映射，
///   收尾撤权、删 profile、撤映射、删临时树。
#[test]
fn view_drive_puts_the_container_cwd_on_the_mapped_letter() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：view_drive_puts_the_container_cwd_on_the_mapped_letter 会建 AppContainer profile 与盘符映射，已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：视图盘往返探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-view-probe-{}", std::process::id()));
    discard(&base);
    let root = base.join("product");
    let home = root.join(".home");
    let private = root.join("session").join("s1");
    std::fs::create_dir_all(&private).expect("建私有区");
    std::fs::create_dir_all(&home).expect("建 .home");

    struct ReleaseOnDrop(PathBuf);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            crate::kernel::detail::view_drive::release_view_drive(&self.0);
        }
    }
    let view = match crate::kernel::detail::view_drive::ensure_view_drive(&root, &home) {
        Ok(view) => view,
        Err(e) => {
            eprintln!(
                "[探针] 视图盘分配不了（{}）：视图盘往返探针跳过（不静默当作通过）",
                e
            );
            discard(&base);
            return;
        }
    };
    let _release = ReleaseOnDrop(home.clone());

    let spec = FencePlan {
        agent: "probe-view".to_string(),
        lease: String::new(),
        cwd: private.clone(),
        net: false.into(),
        home: private.clone(),
        grants: crate::kernel::contracts::fence::grants_of(
            vec![private.clone()],
            Vec::new(),
            Vec::new(),
        ),
    };
    let container = container_name(&spec);
    if let Err(e) = ensure_profile(&container) {
        if e.contains(PROFILE_ENV_BLOCKED_MARK) {
            eprintln!(
                "[探针] 本环境不允许建 AppContainer profile（环境结论，如实跳过）：{}",
                e
            );
            discard(&base);
            return;
        }
        panic!("建容器 profile：{}", e);
    }
    let command = "cd > cwd.txt && echo view-ok > out.txt";
    let prep = prepare_fence(&spec, command, &home);
    if let Some(blocked) = &prep.blocked {
        eprintln!(
            "[探针] 本机给授权落点写不了权限（{}）：视图盘往返探针跳过（不静默当作通过）",
            blocked.line()
        );
        if let Err(e) = release_fence(&spec, &home) {
            eprintln!("[诊断] 撤权未完成（{}）：要收尾请跑 --fence-clean", e);
        }
        clean(&home).ok();
        delete_profile(&container);
        discard(&base);
        return;
    }
    let sid = container_sid(&container).expect("派生容器 SID");
    let code = run_in_container(sid, &spec, command);
    let cwd_text = std::fs::read_to_string(private.join("cwd.txt")).unwrap_or_default();
    let out_text = std::fs::read_to_string(private.join("out.txt")).unwrap_or_default();

    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    // 先撤视图盘再删临时树：release 要读记录文件，树先删就再也撤不掉盘符映射（会留下可见的空盘）。
    crate::kernel::detail::view_drive::release_view_drive(&home);
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);

    let expected = view.view_of(&private).expect("私有区应当在视图盘里");
    eprintln!(
        // 成功诊断不用 [探针] 前缀：门禁把它当 env-skip，真正的跳过行才带这个前缀。
        "[视图盘] 容器内 cwd = {:?}，期望视图写法 = {:?}",
        cwd_text.trim(),
        expected.display()
    );
    assert_eq!(code, Ok(0), "容器应当跑通（exit={:?}）", code);
    assert_eq!(
        out_text.trim(),
        "view-ok",
        "命令产物应当落在真实路径的私有区里"
    );
    assert!(
        cwd_text
            .trim()
            .eq_ignore_ascii_case(&expected.to_string_lossy()),
        "容器内 cwd 应当是视图写法（拿到 {:?}，期望 {:?}）",
        cwd_text.trim(),
        expected.display()
    );
}
