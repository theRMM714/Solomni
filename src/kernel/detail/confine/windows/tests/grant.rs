//! 目的：围栏授权的本地判据——落点形状、权限位覆盖、写后核对、台账归属与撤销。
//! 管：授权清单与 ACL 写入的真机断言（多数在 --fence-live 下跑）。
//! 不管：容器内行为（见 `container.rs`）、视图盘与外部工作区一次性只读（见 `workspace.rs`）。
//! 联动：夹具在 `tests.rs`；机制在 `confine/windows.rs`、`windows/acl.rs` 与 `record/`。

use super::*;

/// 目的：网络能力 SID 只在 `net=true` 时建出来；`net=false` 不授任何 capability（断网）。
#[test]
fn internet_client_capability_is_only_built_when_net_is_true() {
    let _fence = crate::tests::fence_acl_guard();
    assert!(
        super::super::container::internet_client_sid(false)
            .expect("net=false 不该失败")
            .is_none()
    );
    let sid = super::super::container::internet_client_sid(true)
        .expect("建 internetClient 能力 SID")
        .expect("net=true 应该有一个能力");
    assert!(!sid.is_empty(), "能力 SID 不该是空字节");
}

/// 容器 profile **按"会话 + agent + 受保护根集合"派生**：同一次派生的两个进程算出同一个身份；
/// 换会话、换 agent、换 reach（根集合）都是不同身份；`net` 不进指纹（它是令牌能力，不改变 ACL 可见性）。
#[test]
fn container_profile_follows_the_session_and_reach() {
    let _fence = crate::tests::fence_acl_guard();
    let a = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "甲".to_string(),
        lease: "w".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![PathBuf::from("session").join("w1")],
        cwd: PathBuf::from("modules").join("m0"),
        ro: Vec::new(),
        net: false,
    };
    // 同一个（会话, agent, reach）重复派生：身份稳定（外层授权与守门进程各算一次必须一致）。
    assert_eq!(container_name(&a), container_name(&a.clone()));
    // 换会话：独立身份（同 reach 也不共用）。
    let mut b = a.clone();
    b.lease = "w2".to_string();
    assert_ne!(
        container_name(&a),
        container_name(&b),
        "同 reach 的另一个会话不共用 profile（撤销按会话）"
    );
    // 同会话、不同 reach：独立身份（窄调用看不到宽调用的根）。
    let mut c = a.clone();
    c.rw = vec![PathBuf::from("session").join("w2")];
    assert_ne!(
        container_name(&a),
        container_name(&c),
        "同一会话的不同根集合必须是不同身份"
    );
    // 换 agent：自然不共用。
    let mut d = a.clone();
    d.agent = "乙".to_string();
    assert_ne!(container_name(&a), container_name(&d), "不同 agent 不共用");
    // net 不进指纹：同一 reach 开/断网共用一个身份（可见范围不变，能力在令牌里按次给）。
    let mut e = a.clone();
    e.net = true;
    assert_eq!(container_name(&a), container_name(&e), "net 不进容器身份");
    assert!(is_our_profile(&container_name(&a)));
}

/// 清扫只认自己的前缀：别人的容器 profile 一个都不许动。
#[test]
fn profile_sweep_only_matches_our_prefix() {
    let _fence = crate::tests::fence_acl_guard();
    assert!(
        is_our_profile("solomni.agent.0123456789abcdef"),
        "Windows 会把包目录名转小写"
    );
    assert!(is_our_profile("Solomni.Agent.0123456789abcdef"));
    assert!(!is_our_profile("microsoft.windows.notepad"));
    assert!(!is_our_profile("solomni"));
    assert!(!is_our_profile(""));
}

/// 已有 ACE 的权限位必须**覆盖得住**才算数：只看"SID 在场"会让基线被一个只有 SYNCHRONIZE 的继承 ACE
/// 整条挡掉（解释器目录就是这样：容器里连解释器都读不到）；通用位与展开后的具体位要等价看待。
#[test]
fn existing_ace_must_cover_the_rights_we_need() {
    let _fence = crate::tests::fence_acl_guard();
    assert!(
        // 0x0010_0000 = SYNCHRONIZE
        !rights_covered(0x0010_0000, RIGHTS_RO),
        "只有 SYNCHRONIZE 不算覆盖"
    );
    assert!(
        !rights_covered(FILE_GENERIC_READ, RIGHTS_RO),
        "只有读不算覆盖读+执行"
    );
    assert!(
        rights_covered(FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, RIGHTS_RO),
        "读+执行刚够"
    );
    assert!(
        rights_covered(GENERIC_READ | GENERIC_EXECUTE, RIGHTS_RO),
        "通用位与具体位等价"
    );
    assert!(
        rights_covered(GENERIC_ALL, RIGHTS_RW),
        "GENERIC_ALL 覆盖一切"
    );
    assert!(
        !rights_covered(FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, RIGHTS_RW),
        "只读+执行不覆盖读写"
    );
    assert!(rights_covered(FILE_ALL_ACCESS, RIGHTS_RW));
}

/// 数据边界的**父目录**必须拿到只读属性（RIGHTS_STAT）：容器里对中间目录没有这一位时，
/// `exists()` 会对一个确实存在的目录返回假，模块的"父目录不存在就先建"逻辑会一路建到盘卷根
/// （真机 CI 上抓到的 `WinError 5 Access is denied: 'D:\\'`）。这里钉住落点清单，不必真改 ACL。
#[test]
fn grant_targets_include_parents_with_stat_only() {
    let _fence = crate::tests::fence_acl_guard();
    let dir = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("work");
    let module_root = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("modules")
        .join("m0");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![dir.clone()],
        ro: vec![
            std::env::temp_dir()
                .join("solomni-grant-targets")
                .join("shared"),
        ],
        cwd: module_root.clone(),
        net: false,
    };
    let targets = grant_targets(&spec, None, true);
    let find = |p: &std::path::Path| targets.iter().find(|t| t.path == p).cloned();
    // 叶子：读写根递归、只读根不递归。
    assert_eq!(
        find(&dir).map(|t| (t.rights, t.recursive, t.inherit)),
        Some((RIGHTS_RW, true, true)),
        "读写叶子要递归授权"
    );
    let shared = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("shared");
    assert_eq!(
        find(&shared).map(|t| (t.rights, t.recursive, t.inherit)),
        Some((RIGHTS_RO, false, true)),
        "用户授权的只读根不递归"
    );
    // 必要性判据：**用户授权的只读根**是可选落点（授不上只记事实），其余叶子都是必要落点
    // （缺了这次命令在容器里起不来，或这次执行做不了该做的事）——判据见 docs/kernel/README.md 的「围栏落点：必要与可选」。
    assert_eq!(
        find(&dir).map(|t| t.part),
        Some(FencePart::DataBoundary),
        "数据边界叶子"
    );
    assert_eq!(
        find(&shared).map(|t| t.part),
        Some(FencePart::AuthorizedRead)
    );
    assert!(
        find(&dir).expect("读写叶子").part.necessary(),
        "数据边界是必要落点"
    );
    assert!(
        !find(&shared).expect("只读根").part.necessary(),
        "用户授权的只读根是可选落点（授不上只记事实）"
    );
    // 父目录：只读属性、不递归、**不继承**——继承会把 ACE 传播进整棵子树，
    // 撤权断链时残留面就是整棵子树。
    for leaf in [&dir, &module_root] {
        let parent = leaf.parent().expect("叶子有父目录").to_path_buf();
        let got = find(&parent).expect("父目录要在落点清单里");
        assert_eq!(got.rights, RIGHTS_STAT, "父目录只授读属性：{:?}", parent);
        assert!(!got.recursive, "父目录不递归：{:?}", parent);
        assert!(!got.inherit, "父目录不继承：{:?}", parent);
        assert_eq!(got.part, FencePart::Parent);
        assert!(!got.part.necessary(), "父目录是可选落点");
        assert!(
            !rights_covered(RIGHTS_STAT, FILE_GENERIC_READ),
            "读属性不等于能读内容（只够判断存在性）"
        );
    }
}

/// 目的：shell 的 cwd = private 时，PowerShell 逐级解析 cwd 所需的祖先也要有只读属性（仅 STAT、不递归）。
#[test]
fn shell_cwd_gets_every_ancestor_stat_only() {
    let _fence = crate::tests::fence_acl_guard();
    let private = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("a")
        .join("b")
        .join("work");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe".to_string(),
        lease: String::new(),
        private: private.clone(),
        ro_tree: Vec::new(),
        rw: vec![private.clone()],
        ro: Vec::new(),
        cwd: private.clone(),
        net: false,
    };
    // 视图盘生效：祖先链止于产品根（这里用 solomni-grant-targets 当产品根），根外一级都不写。
    let root = std::env::temp_dir().join("solomni-grant-targets");
    let targets = grant_targets(&spec, Some(&root), true);
    let find = |p: &std::path::Path| targets.iter().find(|t| t.path == p).cloned();
    for anc in private.ancestors().skip(1) {
        if anc.as_os_str().is_empty() || anc.parent().is_none() {
            continue;
        }
        let inside = anc == root || anc.starts_with(&root);
        if !inside {
            assert!(find(anc).is_none(), "根外祖先不得入清单：{}", anc.display());
            continue;
        }
        let got = find(anc).unwrap_or_else(|| panic!("根内祖先要在清单里：{}", anc.display()));
        assert_eq!(got.rights, RIGHTS_STAT, "祖先只授读属性：{}", anc.display());
        assert!(!got.recursive && !got.inherit, "祖先不递归、不继承");
        assert_eq!(got.part, FencePart::Parent);
    }
    assert!(find(&root).is_some(), "产品根自身（映射根）要补一条 STAT");
    // 视图生效且 cwd == private 时由读写叶子覆盖 cwd，不再重复加只读 cwd 叶子。
    let without_cwd_leaf = grant_targets(&spec, Some(&root), false);
    assert!(
        !without_cwd_leaf
            .iter()
            .any(|t| t.path == private && t.rights == RIGHTS_RO),
        "cwd == private 时不重复加只读 cwd 叶子"
    );
    // 视图不可用（root=None）时保持旧行为：祖先链补到盘根前一级。
    let legacy = grant_targets(&spec, None, true);
    assert!(
        legacy
            .iter()
            .any(|t| t.path == root && t.rights == RIGHTS_STAT),
        "回退路径仍补根外祖先"
    );
}

/// 只读**子树**（模块目录对 agent 只读）与工作目录：都给 `RIGHTS_RO` 递归，写权只能来自 `rw`。
#[test]
fn grant_targets_keep_module_read_only_and_cwd_read_only() {
    let _fence = crate::tests::fence_acl_guard();
    let base = std::env::temp_dir().join("solomni-grant-targets");
    let module = base.join("modules").join("m0");
    let userdata = module.join("userdata");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: vec![module.clone()],
        rw: vec![userdata.clone()],
        ro: Vec::new(),
        cwd: module.clone(),
        net: false,
    };
    let targets = grant_targets(&spec, None, true);
    // 同一个路径可能有多条（父目录 STAT + 叶子 RO/RW），所以要按**权限位**找那一条，
    // 不能取第一条（那往往是父目录的 STAT）。
    let has = |p: &std::path::Path, r: u32, rec: bool, inh: bool| {
        targets
            .iter()
            .any(|t| t.path == p && t.rights == r && t.recursive == rec && t.inherit == inh)
    };
    assert!(
        has(&module, RIGHTS_RO, true, true),
        "模块根（也是 cwd）要有递归只读：{:?}",
        targets
    );
    assert!(
        !has(&module, RIGHTS_RW, true, true),
        "模块根不得拿到写权：{:?}",
        targets
    );
    assert!(
        has(&userdata, RIGHTS_RW, true, true),
        "userdata 是可写叶子：{:?}",
        targets
    );
}

/// 授权这条路的真机验收：真去改一个目录的 DACL，并把台账与快照都落下来。
/// 先在自有 base 里做完整往返预检；本机做不了就如实打印并跳过（不静默当作通过）。
#[test]
fn grants_are_written_when_the_environment_allows_it() {
    let _fence = crate::tests::fence_acl_guard();
    let base = std::env::temp_dir().join(format!("solomni-grant-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "grant") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：授权探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    // 台账落在探针自己的 base 里（不碰真实 .home/）；产品根 = base，所以落点在根内走快照。
    let home = base.join(".home");
    let outcome = prepare_fence(&spec, "cmd", &home);
    expect_granted(&outcome, "授权应当成功");
    let rec = load_record(&home);
    assert!(
        rec.snapshots.iter().any(|s| Path::new(&s.path) == target),
        "根内路径要先落原始安全描述符快照"
    );
    assert!(
        rec.grants
            .iter()
            .any(|g| Path::new(&g.path) == target && !g.owners.is_empty()),
        "根内路径也要记授权条目并带上归属（收尾按快照整体还原）"
    );
    // 收尾必须把自己写下的权限项按台账撤掉或还原：测试不在本机留痕。
    let report = clean(&home).expect("回收应当成功");
    assert!(report.contains("撤销"), "回收要如实报数量：{}", report);
    discard(&base);
}

/// 分根计时只在**真写 ACL** 时产生步骤：第一次有、第二次纯命中为空；标签只用固定词表、不带用户路径。
#[test]
fn fence_steps_only_cover_real_acl_writes() {
    let _fence = crate::tests::fence_acl_guard();
    let base = std::env::temp_dir().join(format!("solomni-steps-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "steps") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：分步计时探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-steps".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = base.join(".home");
    let first = prepare_fence(&spec, "cmd", &home);
    expect_granted(&first, "授权应当成功");
    assert!(
        !first.steps.is_empty(),
        "第一次真写 ACL 要有分步计时：{:?}",
        first.steps
    );
    for step in &first.steps {
        assert!(
            [STEP_INTERPRETER, STEP_ANCESTOR, STEP_ROOT, STEP_BASE].contains(&step.label.as_str()),
            "步骤标签只用固定词表：{}",
            step.label
        );
        assert!(
            !step.label.contains('\\') && !step.label.contains('/') && !step.label.contains(":\\"),
            "步骤标签不得带用户路径：{}",
            step.label
        );
    }
    assert!(
        first.steps.iter().any(|s| s.label == STEP_ROOT),
        "根内数据边界要记成受保护根：{:?}",
        first.steps
    );
    // 第二次：ACE 已够用 → 零步骤（纯命中不吵）。
    let second = prepare_fence(&spec, "cmd", &home);
    expect_granted(&second, "第二次准备应当成功");
    assert!(
        second.steps.is_empty(),
        "纯命中不应产生步骤：{:?}",
        second.steps
    );
    clean(&home).expect("回收应当成功");
    discard(&base);
}

/// 可信执行基座是**机器级**授权：台账 scope=machine、归属为空；第二次准备不再写它（跨会话复用）。
#[test]
fn trusted_base_grants_are_machine_scope() {
    let _fence = crate::tests::fence_acl_guard();
    let base = std::env::temp_dir().join(format!("solomni-base-scope-{}", std::process::id()));
    let tool = base.join(".tools").join("toolchain");
    let private = base.join("private");
    std::fs::create_dir_all(&tool).expect("建可信基座目录");
    std::fs::create_dir_all(&private).expect("建私有沙箱");
    if !acl_round_trip(&base, "base-scope") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：可信基座归属探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-base".to_string(),
        lease: String::new(),
        private: private.clone(),
        ro_tree: vec![tool.clone()],
        rw: vec![private.clone()],
        cwd: private,
        ro: Vec::new(),
        net: false,
    };
    let home = base.join(".home");
    let first = prepare_fence(&spec, "cmd", &home);
    expect_granted(&first, "授权应当成功");
    {
        let rec = load_record(&home);
        let entry = rec
            .grants
            .iter()
            .find(|g| Path::new(&g.path) == tool)
            .expect("可信基座要进台账");
        assert_eq!(
            entry.scope,
            GrantScope::Machine,
            "可信基座要记成机器级（授给本产品能力 SID）"
        );
        assert!(entry.owners.is_empty(), "机器级条目不记任何进程归属");
        assert!(entry.recursive, "可信基座是递归只读授权");
    }
    let second = prepare_fence(&spec, "cmd", &home);
    expect_granted(&second, "第二次准备应当成功");
    assert!(
        !second.steps.iter().any(|s| s.label == STEP_BASE),
        "第二次纯命中不该再写可信基座：{:?}",
        second.steps
    );
    clean(&home).expect("回收应当成功");
    discard(&base);
}

/// 撤销的真效果：授权 → 撤权 → 目标目录上不再有该容器 SID 的 ACE。
/// 与上一条一样先做完整往返预检（本机受限沙箱会如实跳过）。
#[test]
fn revoke_removes_the_container_ace_from_the_given_roots() {
    let _fence = crate::tests::fence_acl_guard();
    let base = std::env::temp_dir().join(format!("solomni-revoke-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "revoke") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：撤销探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = base.join(".home");
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    assert!(
        has_ace_for(sid, &target, RIGHTS_RW),
        "授权后根上应当有容器 SID 的 ACE"
    );
    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let still = has_ace_for(sid, &target, RIGHTS_RW);
    free_sid(sid);
    assert!(!still, "撤权后根上不该再有该容器 SID 的 ACE");
    // 基线授权（解释器目录只读）也记在同一份台账里，一并按台账撤干净。
    clean(&home).expect("基线回收应当成功");
    discard(&base);
}

/// 只读档的真机验收：授权的只读根上写下的是**只读 ACE**，且撤权能把它撤净。
/// 与读写授权分开断言——只读档的价值就在于"读得到、写不进"。
#[test]
fn read_only_grants_write_ro_aces_and_revoke_removes_them() {
    let _fence = crate::tests::fence_acl_guard();
    let base = std::env::temp_dir().join(format!("solomni-ro-probe-{}", std::process::id()));
    let target = base.join("target");
    let ro = base.join("ro-target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    std::fs::create_dir_all(&ro).expect("建只读根");
    if !acl_round_trip(&base, "ro") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：只读授权探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-ro".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        ro: vec![ro.clone()],
        cwd: target.clone(),
        net: false,
    };
    let home = base.join(".home");
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    assert!(has_ace_for(sid, &ro, RIGHTS_RO), "只读根上要有只读 ACE");
    assert!(
        !has_ace_for(sid, &ro, RIGHTS_RW),
        "只读根上不该有读写 ACE——那正是只读档的意义"
    );
    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let still = has_ace_for(sid, &ro, RIGHTS_RO);
    free_sid(sid);
    assert!(!still, "撤权后只读根上不该再有该容器 SID 的 ACE");
    clean(&home).expect("台账回收应当成功");
    discard(&base);
}

/// 【残留探针】授权（叶子读写 + 父目录只读属性）→ 撤权后，叶子与父目录上都不得留有该容器 SID
/// 的**任何**显式 ACE。真机残留过一条只有 SYNCHRONIZE 的 (OI)(CI) ACE，整棵 tests/ 子树因此
/// 对受限进程不可读，所以撤净判定不看权限位、只看 SID 在不在场（has_any_ace_for）。
/// 父目录上的只读属性是**机器级**授权：授给本产品能力 SID，不随会话撤、--fence-clean 才收；
/// 容器 SID 只该落在叶子上。grant_targets 的落点清单或归属变了它会先红。
#[test]
fn revoke_leaves_no_container_ace_on_leaf_parents() {
    let _fence = crate::tests::fence_acl_guard();
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：残留探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-leftover-probe-{}", std::process::id()));
    discard(&base);
    let leaf = base.join("leaf");
    std::fs::create_dir_all(&leaf).expect("建探针目录");
    std::fs::write(leaf.join("data.txt"), "x").expect("写探针文件");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-leftover".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![leaf.clone()],
        cwd: leaf.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = base.join(".home");
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    // 诊断输出用 [诊断] 前缀：门禁只把 [探针] 当 env-skip，别让这两行把"跳过数"充大。
    eprintln!("[诊断] 授权后父目录 {}", dump_aces(&base));
    eprintln!("[诊断] 授权后叶子 {}", dump_aces(&leaf));
    let baseline = product_sid().expect("派生本产品能力 SID");
    assert!(
        has_explicit_ace_for(baseline, &base, RIGHTS_STAT),
        "授权后父目录上应有授给本产品能力 SID 的只读属性 ACE（祖先链是机器级授权）"
    );
    free_sid(baseline);
    assert!(
        !has_any_ace_for(sid, &base),
        "容器 SID 只该落在叶子上：父目录的只读属性不该授给容器"
    );
    assert!(
        has_ace_for(sid, &leaf, RIGHTS_RW),
        "授权后叶子上应有读写 ACE"
    );
    release_fence(&spec, &home).expect("撤权应当成功");
    eprintln!("[诊断] 撤权后父目录 {}", dump_aces(&base));
    eprintln!("[诊断] 撤权后叶子 {}", dump_aces(&leaf));
    let on_parent = has_any_ace_for(sid, &base);
    let on_leaf = has_any_ace_for(sid, &leaf);
    free_sid(sid);
    assert!(
        !on_parent,
        "撤权后父目录不得残留该容器 SID 的任何 ACE（残留会把父目录对受限进程藏住）"
    );
    assert!(!on_leaf, "撤权后叶子不得残留该容器 SID 的任何 ACE");
    // 机器级授权按设计不随会话撤：别的会话还要靠它零写启动。
    let baseline = product_sid().expect("派生本产品能力 SID");
    let machine_kept = has_explicit_ace_for(baseline, &base, RIGHTS_STAT);
    free_sid(baseline);
    assert!(
        machine_kept,
        "撤权不该动祖先链的机器级只读属性 ACE（它跨会话复用，只由整份台账回收）"
    );
    clean(&home).expect("台账回收应当成功");
    let baseline = product_sid().expect("派生本产品能力 SID");
    let machine_after_clean = has_explicit_ace_for(baseline, &base, RIGHTS_STAT);
    free_sid(baseline);
    discard(&base);
    // 只看对象自己带的 ACE：产品根有机器级 `(OI)(CI)` 授权时，父目录会从祖先继承到一条同主体的 STAT，
    // 那是祖先授权的投影、不归这份台账撤（夹具的 TMP 在仓库内时正是这种情形）。
    assert!(
        !machine_after_clean,
        "整份台账回收后父目录不该再有自己写的机器级只读属性 ACE"
    );
}

/// 【真机往返】对象 ACE（SID 前还带类型 GUID）也进写后核对的账：
/// 真写下一条对象 ACE → 核对看得见它 → 我们的授权往返（记录 → 写 → 读回）不弄丢它 →
/// 再把"弄丢"注入一次，证明核对确实会报（不然这条断言可能永远为真）。
/// 会改本机状态（写临时文件的 DACL），按测试约定只在 --fence-live 下跑；做不了往返预检就如实跳过。
#[test]
fn object_ace_is_covered_by_the_write_then_equal_check() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：object_ace_is_covered_by_the_write_then_equal_check 会改本机 ACL（临时文件），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    let base =
        std::env::temp_dir().join(format!("solomni-object-ace-probe-{}", std::process::id()));
    discard(&base);
    std::fs::create_dir_all(&base).expect("建探针目录");
    if !acl_round_trip(&base, "object-ace") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：对象 ACE 探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let file = base.join("data.txt");
    std::fs::write(&file, b"x").expect("写探针文件");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-object-ace".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![base.clone()],
        cwd: base.clone(),
        ro: Vec::new(),
        net: false,
    };
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let snapshot = sd_bytes(&file).expect("记下探针文件的原始安全描述符");
    let guid = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x01,
    ];
    let outcome = grant_one_object_ace(sid, &file, RIGHTS_RO, guid);
    assert!(
        outcome.is_ok(),
        "写一条对象 ACE 应当成功：{:?}",
        outcome.err()
    );
    eprintln!("[诊断] 含对象 ACE 的文件 DACL {}", dump_aces(&file));
    let with_object = acl_entries(&file).expect("读含对象 ACE 的集合");
    let object_entry = with_object
        .iter()
        .find(|e| e.0 == 5)
        .expect("对象 ACE 必须被核对看得见——这就是这条缺口的分界线");
    assert!(
        !object_entry.3.is_empty(),
        "身份里要带上对象类型 GUID，否则两条只差 GUID 的 ACE 会互相顶包：{:?}",
        object_entry
    );
    eprintln!("[诊断] 对象 ACE 的身份：{:?}", object_entry);

    // 我们的授权往返：记录 → 写 → 读回核对。对象 ACE 必须活下来，否则核对会如实报"弄丢了"。
    let grant = grant_verified(sid, &file, RIGHTS_RO, false, false);
    assert!(
        grant.is_ok(),
        "写后核对不该把对象 ACE 判成丢失：{:?}",
        grant.err()
    );
    let after = acl_entries(&file).expect("读我们授权后的集合");
    assert!(
        !lost_entries(&with_object, &after).iter().any(|e| e.0 == 5),
        "对象 ACE 不能在授权往返里丢：{:?}",
        lost_entries(&with_object, &after)
    );
    revoke_one(sid, &file, false).expect("撤掉我们写的那条 ACE");

    // 注入一次"对象 ACE 被弄丢"：核对必须报出来。
    restore_sd(&file, &snapshot).expect("还原成没有对象 ACE 的原始安全描述符");
    let without = acl_entries(&file).expect("读还原后的集合");
    let lost = lost_entries(&with_object, &without);
    assert!(
        lost.iter().any(|e| e.0 == 5),
        "弄丢对象 ACE 必须被核对报出来，实际丢失：{:?}",
        lost
    );
    eprintln!("[诊断] 注入丢失后核对报出：{:?}", lost);

    free_sid(sid);
    discard(&base);
}

/// ACL 写法的语义与后果验证：非递归 `SetNamedSecurityInfoW` 与递归 `TreeSetNamedSecurityInfoW`（带/不带继承标志）
/// 各自把 ACE 铺到哪些节点、写入后目标还能不能读回自己的 DACL——`grant_one` 的 `recursive` 分支就靠 TreeSet，
/// 「去继承、全显式」能不能成立全看这里。探针只打印事实、不预设结论；本机不允许改 ACL 时如实跳过。
#[test]
fn acl_write_flavours_are_probed_for_scope_and_readability() {
    let _fence = crate::tests::fence_acl_guard();
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：ACL 写法探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-acl-probe-{}", std::process::id()));
    discard(&base);
    // 预检：先在自有目录里做一次完整往返（写 → 读回 → 撤）。受限环境里"写成功但读不回/删不掉"，
    // 那样的进程做不了观察，也留不了干净的现场——如实跳过，不静默当作通过，更不制造残留。
    let pre = base.join("preflight");
    std::fs::create_dir_all(&pre).expect("建预检目录");
    let pre_spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "acl-probe-preflight".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![pre.clone()],
        cwd: pre.clone(),
        ro: Vec::new(),
        net: false,
    };
    let pre_sid = container_sid(&container_name(&pre_spec)).expect("派生预检容器 SID");
    let round_trip = grant_one(pre_sid, &pre, RIGHTS_RO, false, false).is_ok()
        && has_any_ace_for(pre_sid, &pre)
        && revoke_one(pre_sid, &pre, false).is_ok();
    free_sid(pre_sid);
    if !round_trip {
        eprintln!("[探针] 本机做不了完整往返（写→读回→撤）：ACL 写法探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let cases = [
        ("set-rec0-inh0", false, false),
        ("tree-rec1-inh0", true, false),
        ("tree-rec1-inh1", true, true),
    ];
    for (tag, recursive, inherit) in cases {
        let tree = base.join(tag);
        std::fs::create_dir_all(tree.join("sub")).expect("建探针树");
        std::fs::write(tree.join("a.txt"), b"a").expect("写根文件");
        std::fs::write(tree.join("sub").join("b.txt"), b"b").expect("写子文件");
        let spec = FenceSpec {
            machine_ro_tree: Vec::new(),
            agent: format!("acl-probe-{tag}"),
            lease: String::new(),
            private: PathBuf::new(),
            ro_tree: Vec::new(),
            rw: vec![tree.clone()],
            cwd: tree.clone(),
            ro: Vec::new(),
            net: false,
        };
        let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
        let root_before = acl_entries(&tree).expect("读写入前根 ACE 集合");
        let sub_before = acl_entries(&tree.join("sub")).expect("读写入前子目录 ACE 集合");
        let grant = grant_one(sid, &tree, RIGHTS_RO, recursive, inherit);
        let root_ace = has_any_ace_for(sid, &tree);
        let sub_ace = has_any_ace_for(sid, &tree.join("sub"));
        let file_ace = has_any_ace_for(sid, &tree.join("sub").join("b.txt"));
        let revoke = revoke_one(sid, &tree, recursive);
        let root_after = acl_entries(&tree).expect("读撤销后根 ACE 集合");
        let sub_after = acl_entries(&tree.join("sub")).expect("读撤销后子目录 ACE 集合");
        eprintln!(
            "[诊断] {tag}：grant_ok={} 根ACE={} 子目录ACE={} 子文件ACE={} revoke_ok={}",
            grant.is_ok(),
            root_ace,
            sub_ace,
            file_ace,
            revoke.is_ok()
        );
        free_sid(sid);
        assert!(grant.is_ok(), "{tag}：授予应当成功");
        assert!(revoke.is_ok(), "{tag}：撤销应当成功");
        assert!(
            lost_entries(&root_before, &root_after).is_empty(),
            "{tag}：写入并撤销后根原有的不同 ACE 必须都在（集合语义）"
        );
        assert!(
            lost_entries(&sub_before, &sub_after).is_empty(),
            "{tag}：写入并撤销后子目录原有的不同 ACE 必须都在（集合语义）"
        );
        match (recursive, inherit) {
            // 非递归只设根本身；TreeSet 配**不带继承标志**的 ACE 同样只落在根——
            // 所以「整树覆盖」只有继承这一条路，去继承就得自己逐节点写。
            (false, _) | (true, false) => assert!(
                root_ace && !sub_ace && !file_ace,
                "{tag}：只应落在根本身（root={root_ace} sub={sub_ace} file={file_ace}）"
            ),
            (true, true) => assert!(
                root_ace && sub_ace && file_ace,
                "{tag}：带继承标志应覆盖整棵树（root={root_ace} sub={sub_ace} file={file_ace}）"
            ),
        }
    }
    discard(&base);
}

/// 目的：预检的“能不能写 DACL”用 AccessCheck 判，不只认属主（nvm 真身属主是 Administrators，
///   当前用户却有 WRITE_DAC）。
#[test]
fn can_write_dacl_accepts_a_directory_we_own() {
    let _fence = crate::tests::fence_acl_guard();
    let dir = crate::tests::scratch("can-write-dacl");
    std::fs::create_dir_all(&dir).expect("建目录");
    assert!(
        can_write_dacl(&dir).expect("AccessCheck 应当能跑"),
        "自己拥有的目录应当可写 DACL"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：写都没成功时不许报“回滚失败”，也不许动预存的 ACE（真机上它就是链接上那条 SYNCHRONIZE）。
#[test]
fn failed_write_keeps_preexisting_ace_and_never_says_rollback() {
    let _fence = crate::tests::fence_acl_guard();
    let base = crate::tests::scratch("grant-failure-rollback");
    let dir = base.join("mod");
    let home = base.join(".home");
    std::fs::create_dir_all(&dir).expect("建目录");
    std::fs::create_dir_all(&home).expect("建台账目录");
    let sid = product_sid().expect("派生本产品能力 SID");
    // 预存一条**不够用**的 ACE（真机上就是链接上那条只有 SYNCHRONIZE 的项）。
    const SYNCHRONIZE: u32 = 0x0010_0000;
    grant_one(sid, &dir, SYNCHRONIZE, false, false).expect("预存 ACE");
    assert!(!has_ace_for(sid, &dir, RIGHTS_RO), "前置：预存 ACE 不够用");
    assert!(has_any_ace_for(sid, &dir), "前置：预存 ACE 在场");
    let before = acl_scan(&dir).expect("读 ACL").entries.len();
    FORCE_GRANT_FAILURE.with(|f| f.set(true));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            FORCE_GRANT_FAILURE.with(|f| f.set(false));
        }
    }
    let _reset = Reset;
    let target = GrantTarget {
        path: dir.clone(),
        rights: RIGHTS_RO,
        recursive: false,
        inherit: false,
        part: FencePart::Interpreter,
    };
    let err = grant_one_journaled(
        &home,
        sid,
        &sid_to_string(sid),
        &target,
        &base,
        current_owner(),
    )
    .expect_err("注入的写失败要如实返回");
    assert!(
        !err.contains("回滚失败"),
        "写都没成功不许报回滚失败：{}",
        err
    );
    assert!(has_any_ace_for(sid, &dir), "预存 ACE 必须还在");
    assert_eq!(
        acl_scan(&dir).expect("读 ACL").entries.len(),
        before,
        "失败路径不许改 DACL"
    );
    revoke_one(sid, &dir, false).expect("清掉预存 ACE");
    free_sid(sid);
    let _ = std::fs::remove_dir_all(&base);
}

/// 根内判定不能把词法上的上级段当成仍在根内：规范化失败时 root/../escape
/// 必须按根外处理，否则会为一条越界路径先存原始安全描述符快照。
#[test]
fn inside_root_rejects_parent_escape_even_when_canonicalize_fails() {
    let _fence = crate::tests::fence_acl_guard();
    let base = crate::tests::scratch("inside-root");
    let root = base.join("root");
    std::fs::create_dir_all(&root).expect("建根目录");

    let missing_inside = root.join("missing").join("child");
    assert!(
        inside_root(&missing_inside, &root),
        "不存在的根内路径仍应按词法判定为根内"
    );

    let escaped = root.join("..").join("escape");
    assert!(
        !inside_root(&escaped, &root),
        "含上级段的路径不能因为前缀相同就被判为根内"
    );

    let sibling = base.join("sibling");
    std::fs::create_dir_all(&sibling).expect("建同级目录");
    assert!(!inside_root(&sibling, &root), "同级目录不在根内");

    let _ = std::fs::remove_dir_all(&base);
}

/// 【真机退役】旧的全应用包机器级授权：**能力 SID 的 ACE 覆盖得住才撤**，覆盖不住就保留。
/// 产品根与它的子目录各有一条旧授权、能力 SID 的读属性已就位 → 旧主体撤净、台账销账；
/// 另一条只授了全应用包只读、能力 SID 那边只有读属性（覆盖不住只读）→ ACE 与台账条目都保留。
/// 会改本机 ACL（临时目录），按测试约定只在 --fence-live 下跑。
#[test]
fn legacy_aapp_machine_grants_retire_only_after_capability_is_in_place() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：legacy_aapp_machine_grants_retire_only_after_capability_is_in_place 会改本机 ACL（临时目录），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    let base = crate::tests::scratch("retire-aapp");
    discard(&base);
    let root = base.join("product");
    let sub = root.join("sub");
    let ro_only = base.join("ro-only");
    std::fs::create_dir_all(&sub).expect("建产品根与子目录");
    std::fs::create_dir_all(&ro_only).expect("建只读落点");
    if !acl_round_trip(&base, "retire-aapp") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：旧授权退役探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = root.join(".home");
    let aapp = system_baseline_sid().expect("取全应用包 SID");
    let aapp_text = sid_to_string(aapp);
    let cap = product_sid().expect("派生本产品能力 SID");
    let cap_text = sid_to_string(cap);
    let target = |path: &Path, rights: u32| GrantTarget {
        path: path.to_path_buf(),
        rights,
        recursive: false,
        inherit: true,
        part: FencePart::Parent,
    };
    // 造旧状态用「落台账 + 真写」两半，不走 grant_machine_journaled：夹具的 TMP 在仓库内时，
    // 探针目录会从真实产品根继承到一条同主体的 ACE，那条路的「已够用就跳过」会让这里根本没有自己的 ACE 可断言。
    let legacy = |path: &Path, rights: u32| {
        claim_machine(&home, &aapp_text, &target(path, rights), &root).expect("旧授权先落台账");
        grant_verified(aapp, path, rights, false, true).expect("写旧的全应用包 ACE");
    };
    legacy(&root, RIGHTS_STAT);
    legacy(&sub, RIGHTS_STAT);
    legacy(&ro_only, RIGHTS_RO);
    // 接替者只到产品根，且只有读属性：覆盖得住根与子目录那两条，覆盖不住只读那条。
    claim_machine(&home, &cap_text, &target(&root, RIGHTS_STAT), &root).expect("能力授权先落台账");
    grant_verified(cap, &root, RIGHTS_STAT, false, true).expect("写能力 SID 的 ACE");
    assert!(
        has_explicit_ace_for(aapp, &root, RIGHTS_STAT),
        "前置：产品根上旧 ACE 在场"
    );
    assert!(
        !has_ace_for(cap, &ro_only, RIGHTS_RO),
        "前置：只读落点上能力 SID 覆盖不住只读位"
    );
    let mut lines: Vec<String> = Vec::new();
    retire_legacy_machine_grants(&home, &mut |line| lines.push(line.to_string()))
        .expect("退役应当成功");
    eprintln!("[诊断] 产品根退役后 {}", dump_aces(&root));
    // 只看对象自己带着的 ACE：从真实产品根继承来的投影属于祖先那条授权，不归这一条撤。
    assert!(
        !has_explicit_any_ace_for(aapp, &root),
        "有接替者时旧主体必须撤净（哪怕只留一位）"
    );
    assert!(
        !has_explicit_any_ace_for(aapp, &sub),
        "子目录那条旧授权也要撤净"
    );
    assert!(
        has_explicit_ace_for(cap, &root, RIGHTS_STAT),
        "能力 SID 的 ACE 必须还在——先授后撤，不留读不到的窗口"
    );
    assert!(
        has_explicit_ace_for(aapp, &ro_only, RIGHTS_RO),
        "没有接替者的那条必须保留（解释器目录与根外祖先正是这种情形）"
    );
    let rec = load_record(&home);
    let recorded = |sid: &str, path: &Path| {
        rec.grants
            .iter()
            .any(|g| g.sid == sid && Path::new(&g.path) == path)
    };
    assert!(!recorded(&aapp_text, &root), "撤掉的那条要销账");
    assert!(!recorded(&aapp_text, &sub), "子目录那条也要销账");
    assert!(
        recorded(&aapp_text, &ro_only),
        "保留的那条台账也要留着（重跑或显式撤都靠它）"
    );
    assert!(
        recorded(&cap_text, &root),
        "同路径上能力 SID 的条目不受影响"
    );
    let summary = lines.last().expect("要有汇总行");
    assert!(
        summary.contains("退役 2 条") && summary.contains("保留 1 条"),
        "汇总要如实报撤了几条、留了几条：{}",
        summary
    );
    // 收尾：保留的那条也撤掉，台账整份回收，不在本机留痕。
    revoke_one(aapp, &ro_only, false).expect("清掉只读落点的探针 ACE");
    free_sid(aapp);
    free_sid(cap);
    clean(&home).expect("台账回收应当成功");
    discard(&base);
}
