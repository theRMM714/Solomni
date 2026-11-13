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

    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-view".to_string(),
        lease: String::new(),
        private: private.clone(),
        ro_tree: Vec::new(),
        rw: vec![private.clone()],
        cwd: private.clone(),
        ro: Vec::new(),
        net: false,
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

/// 【真机】外部工作区的机器级一次性只读：首写一次（授基线组、可继承），之后的准备零写；台账记机器级、无归属。
#[test]
fn machine_ro_tree_is_granted_once_and_reused_after() {
    let _fence = crate::tests::fence_acl_guard();
    let base = std::env::temp_dir().join(format!("solomni-machine-ro-{}", std::process::id()));
    discard(&base);
    let product = base.join("product");
    let home = product.join(".home");
    let private = product.join("session").join("s1");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&private).expect("建私有区");
    std::fs::create_dir_all(&home).expect("建 .home");
    std::fs::create_dir_all(&workspace).expect("建外部工作区");
    if !acl_round_trip(&base, "machine-ro") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：机器级只读探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let spec = FenceSpec {
        machine_ro_tree: vec![workspace.clone()],
        agent: "probe-machine-ro".to_string(),
        lease: String::new(),
        private: private.clone(),
        ro_tree: Vec::new(),
        rw: vec![private.clone()],
        cwd: private,
        ro: Vec::new(),
        net: false,
    };
    let first = prepare_fence(&spec, "cmd", &home);
    expect_granted(&first, "授权应当成功");
    assert!(
        first.steps.iter().any(|s| s.label == STEP_MACHINE_RO),
        "首次要让工作区可读，必须真写一次 ACL：{:?}",
        first.steps
    );
    let base_sid = baseline_sid().expect("取基线组 SID");
    assert!(
        has_inheritable_ace_for(base_sid, &workspace, RIGHTS_RO),
        "工作区上要有可继承的基线组只读 ACE"
    );
    free_sid(base_sid);
    {
        let rec = load_record(&home);
        let entry = rec
            .grants
            .iter()
            .find(|g| Path::new(&g.path) == workspace)
            .expect("机器级工作区条目要进台账");
        assert_eq!(entry.scope, GrantScope::Machine, "工作区是机器级授权");
        assert!(entry.owners.is_empty(), "机器级条目不记任何进程归属");
        assert!(entry.recursive, "工作区要递归可达");
    }
    // 第二次准备：ACE 已够用 → 零写、无步骤，台账条目仍在（下次撤权找得到它）。
    let second = prepare_fence(&spec, "cmd", &home);
    expect_granted(&second, "第二次准备应当成功");
    assert!(
        !second.steps.iter().any(|s| s.label == STEP_MACHINE_RO),
        "可继承 ACE 已存在时不该再写：{:?}",
        second.steps
    );
    clean(&home).expect("台账回收应当成功");
    discard(&base);
}

/// 【真机】显式绑定工作区：ensure 首写一次、再调只复用不写；撤权把盘上 ACE 与台账条目一起销掉。
#[test]
fn ensure_workspace_grant_is_explicit_and_revocable() {
    let _fence = crate::tests::fence_acl_guard();
    let base = std::env::temp_dir().join(format!("solomni-ws-ensure-{}", std::process::id()));
    discard(&base);
    let home = base.join("product").join(".home");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&home).expect("建 .home");
    std::fs::create_dir_all(&workspace).expect("建外部工作区");
    if !acl_round_trip(&base, "ws-ensure") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：工作区显式授权探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let first = ensure_workspace_grant(&home, &workspace).expect("首次授权应当成功");
    assert_eq!((first.written, first.reused), (1, 0), "首次要真写一次");
    let again = ensure_workspace_grant(&home, &workspace).expect("重复授权应当成功");
    assert_eq!((again.written, again.reused), (0, 1), "再次授权只复用");
    {
        let rec = load_record(&home);
        assert!(
            rec.grants
                .iter()
                .any(|g| Path::new(&g.path) == workspace && g.scope == GrantScope::Machine),
            "台账里要有这条机器级条目"
        );
    }
    assert_eq!(
        revoke_machine_grants(&home, &workspace).expect("撤权应当成功"),
        1,
        "撤权要销掉那一条台账条目"
    );
    let base_sid = baseline_sid().expect("取基线组 SID");
    assert!(
        !has_inheritable_ace_for(base_sid, &workspace, RIGHTS_RO),
        "撤权后工作区上不该再有基线组只读 ACE"
    );
    free_sid(base_sid);
    let rec = load_record(&home);
    assert!(
        !rec.grants
            .iter()
            .any(|g| Path::new(&g.path) == workspace && g.scope == GrantScope::Machine),
        "撤权后台账里不该再有这条条目"
    );
    assert_eq!(
        revoke_machine_grants(&home, &workspace).expect("重复撤权应当成功"),
        0,
        "再撤一次是幂等的"
    );
    discard(&base);
}
