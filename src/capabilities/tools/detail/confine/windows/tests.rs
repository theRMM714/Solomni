use super::*;
use std::sync::Mutex;

/// 容器 profile **一个 agent 一个**：同名 agent 跨会话复用同一个容器身份（数量有界），换 agent 就换 profile。
#[test]
fn container_profile_is_one_per_agent() {
    let a = FenceSpec {
        agent: "甲".to_string(),
        rw: vec![PathBuf::from("session").join("w1")],
        cwd: PathBuf::from("modules").join("m0"),
        ro: Vec::new(),
        net: false,
    };
    let mut b = a.clone();
    b.rw = vec![PathBuf::from("session").join("w2")];
    b.cwd = PathBuf::from("modules").join("m1");
    let mut c = a.clone();
    c.agent = "乙".to_string();
    assert_eq!(
        container_name(&a),
        container_name(&b),
        "同一个 agent 的不同会话共用一个 profile"
    );
    assert_ne!(container_name(&a), container_name(&c), "不同 agent 不共用");
    assert!(is_our_profile(&container_name(&a)));
}

/// 清扫只认自己的前缀：别人的容器 profile 一个都不许动。
#[test]
fn profile_sweep_only_matches_our_prefix() {
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
/// 整条挡掉（真机上解释器目录就是这样，容器里连解释器都读不到）；通用位与展开后的具体位要等价看待。
#[test]
fn existing_ace_must_cover_the_rights_we_need() {
    assert!(
        !rights_covered(0x0010_0000 /* SYNCHRONIZE */, RIGHTS_RO),
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
    let dir = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("work");
    let module_root = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("modules")
        .join("m0");
    let spec = FenceSpec {
        agent: "probe".to_string(),
        rw: vec![dir.clone()],
        ro: vec![std::env::temp_dir()
            .join("solomni-grant-targets")
            .join("shared")],
        cwd: module_root.clone(),
        net: false,
    };
    let targets = grant_targets(&spec);
    let find = |p: &std::path::Path| targets.iter().find(|(x, _, _)| x == p).cloned();
    // 叶子：读写根递归、只读根不递归。
    assert_eq!(
        find(&dir).map(|(_, r, rec)| (r, rec)),
        Some((RIGHTS_RW, true)),
        "读写叶子要递归授权"
    );
    assert_eq!(
        find(
            &std::env::temp_dir()
                .join("solomni-grant-targets")
                .join("shared")
        )
        .map(|(_, r, rec)| (r, rec)),
        Some((RIGHTS_RO, false)),
        "用户授权的只读根不递归"
    );
    // 父目录：只读属性、不递归、不继承（grant_one 的 inherit 恒为 true，故这里看 rights 与 recursive）。
    for leaf in [&dir, &module_root] {
        let parent = leaf.parent().expect("叶子有父目录").to_path_buf();
        let got = find(&parent).expect("父目录要在落点清单里");
        assert_eq!(got.1, RIGHTS_STAT, "父目录只授读属性：{:?}", parent);
        assert!(!got.2, "父目录不递归：{:?}", parent);
        assert!(
            !rights_covered(RIGHTS_STAT, FILE_GENERIC_READ),
            "读属性不等于能读内容（只够判断存在性）"
        );
    }
}

/// 授权这条路的真机验收：真去改一个目录的 DACL。
/// 本机环境不允许改 ACL 时（例如被沙箱挡住）如实打印原因并跳过——不静默当作通过。
#[test]
fn grants_are_written_when_the_environment_allows_it() {
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：授权探针跳过（不静默当作通过）——请在普通 shell 里重跑 cargo test 验证",
            capability().note
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("solomni-grant-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建探针目录");
    let spec = FenceSpec {
        agent: "probe".to_string(),
        rw: vec![dir.clone()],
        cwd: dir.clone(),
        ro: Vec::new(),
        net: false,
    };
    let prepared = Mutex::new(std::collections::BTreeSet::new());
    // 台账落在探针自己的临时目录里（不碰真实 .home/）。
    let home = dir.join("ledger");
    let outcome = prepare_fence(&spec, "cmd", &prepared, &home);
    assert!(outcome.is_ok(), "授权应当成功：{:?}", outcome.err());
    // 收尾必须把自己写下的权限项按台账撤掉：测试不在本机留痕（撤不动就报出来，不静默）。
    let report = clean(&home).expect("回收应当成功");
    assert!(report.contains("撤销"), "回收要如实报数量：{}", report);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 撤销的真效果：授权 → 撤权 → 目标目录上不再有该容器 SID 的 ACE。
/// 与上一条一样只在能改 ACL 的环境里真跑（本机受限沙箱会如实跳过）。
#[test]
fn revoke_removes_the_container_ace_from_the_given_roots() {
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：撤销探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("solomni-revoke-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建探针目录");
    let spec = FenceSpec {
        agent: "probe".to_string(),
        rw: vec![dir.clone()],
        cwd: dir.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = dir.join("ledger");
    let prepared = Mutex::new(std::collections::BTreeSet::new());
    prepare_fence(&spec, "cmd", &prepared, &home).expect("授权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    assert!(
        has_ace_for(sid, &dir, RIGHTS_RW),
        "授权后根上应当有容器 SID 的 ACE"
    );
    free_sid(sid);
    release_fence_home(&spec, Some(&home)).expect("撤权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let still = has_ace_for(sid, &dir, RIGHTS_RW);
    free_sid(sid);
    assert!(!still, "撤权后根上不该再有该容器 SID 的 ACE");
    // 基线授权（解释器目录只读）也记在同一份台账里，一并按台账撤干净。
    clean(&home).expect("基线回收应当成功");
    let _ = std::fs::remove_dir_all(&dir);
}
/// 只读档的真机验收：授权的只读根上写下的是**只读 ACE**，且撤权能把它撤净。
/// 与读写授权分开断言——只读档的价值就在于"读得到、写不进"。
#[test]
fn read_only_grants_write_ro_aces_and_revoke_removes_them() {
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：只读授权探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("solomni-ro-probe-{}", std::process::id()));
    let ro = std::env::temp_dir().join(format!("solomni-ro-target-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建探针目录");
    std::fs::create_dir_all(&ro).expect("建只读根");
    let spec = FenceSpec {
        agent: "probe-ro".to_string(),
        rw: vec![dir.clone()],
        ro: vec![ro.clone()],
        cwd: dir.clone(),
        net: false,
    };
    let home = dir.join("ledger");
    let prepared = Mutex::new(std::collections::BTreeSet::new());
    prepare_fence(&spec, "cmd", &prepared, &home).expect("授权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    assert!(has_ace_for(sid, &ro, RIGHTS_RO), "只读根上要有只读 ACE");
    assert!(
        !has_ace_for(sid, &ro, RIGHTS_RW),
        "只读根上不该有读写 ACE——那正是只读档的意义"
    );
    free_sid(sid);
    release_fence_home(&spec, Some(&home)).expect("撤权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let still = has_ace_for(sid, &ro, RIGHTS_RO);
    free_sid(sid);
    assert!(!still, "撤权后只读根上不该再有该容器 SID 的 ACE");
    clean(&home).expect("台账回收应当成功");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&ro);
}
