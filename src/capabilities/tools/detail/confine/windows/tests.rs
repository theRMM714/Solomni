use super::*;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::{
    GetAce, GetAclInformation, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION,
};

/// 把对象 DACL 里的允许 ACE 逐条转储成可读文本（残留调查用：原始权限位 + 继承标志 + SID）。
fn dump_aces(path: &Path) -> String {
    const ACL_SIZE_INFORMATION_CLASS: i32 = 2;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSID = std::ptr::null_mut();
    let w = wide(path);
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return format!("{}：读 DACL 失败（{}）", path.display(), rc);
    }
    let mut out = format!(
        "{}：
",
        path.display()
    );
    if !dacl.is_null() {
        let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            GetAclInformation(
                dacl,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                ACL_SIZE_INFORMATION_CLASS,
            )
        };
        if ok != 0 {
            for i in 0..info.AceCount {
                let mut ace: *mut c_void = std::ptr::null_mut();
                if unsafe { GetAce(dacl, i, &mut ace) } == 0 || ace.is_null() {
                    continue;
                }
                let base = ace as *const u8;
                let ace_type = unsafe { *base };
                let flags = unsafe { *base.add(1) };
                let mask = unsafe { std::ptr::read_unaligned(base.add(4) as *const u32) };
                let sid_text = sid_to_string(unsafe { base.add(8) as PSID });
                out.push_str(&format!(
                    "  type={} flags=0x{:02X} mask=0x{:08X} inherited={} sid={}
",
                    ace_type,
                    flags,
                    mask,
                    flags & 0x10 != 0,
                    sid_text
                ));
            }
        }
    }
    unsafe {
        LocalFree(sd);
    }
    out
}

/// 容器 profile **一个 agent 一个**：同名 agent 跨会话复用同一个容器身份（数量有界），换 agent 就换 profile。
#[test]
fn container_profile_is_one_per_agent() {
    let a = FenceSpec {
        agent: "甲".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
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
/// 整条挡掉（解释器目录就是这样：容器里连解释器都读不到）；通用位与展开后的具体位要等价看待。
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
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![dir.clone()],
        ro: vec![std::env::temp_dir()
            .join("solomni-grant-targets")
            .join("shared")],
        cwd: module_root.clone(),
        net: false,
    };
    let targets = grant_targets(&spec);
    let find = |p: &std::path::Path| targets.iter().find(|(x, _, _, _)| x == p).cloned();
    // 叶子：读写根递归、只读根不递归。
    assert_eq!(
        find(&dir).map(|(_, r, rec, inh)| (r, rec, inh)),
        Some((RIGHTS_RW, true, true)),
        "读写叶子要递归授权"
    );
    assert_eq!(
        find(
            &std::env::temp_dir()
                .join("solomni-grant-targets")
                .join("shared")
        )
        .map(|(_, r, rec, inh)| (r, rec, inh)),
        Some((RIGHTS_RO, false, true)),
        "用户授权的只读根不递归"
    );
    // 父目录：只读属性、不递归、**不继承**——继承会把 ACE 传播进整棵子树，
    // 撤权断链时残留面就是整棵子树。
    for leaf in [&dir, &module_root] {
        let parent = leaf.parent().expect("叶子有父目录").to_path_buf();
        let got = find(&parent).expect("父目录要在落点清单里");
        assert_eq!(got.1, RIGHTS_STAT, "父目录只授读属性：{:?}", parent);
        assert!(!got.2, "父目录不递归：{:?}", parent);
        assert!(!got.3, "父目录不继承：{:?}", parent);
        assert!(
            !rights_covered(RIGHTS_STAT, FILE_GENERIC_READ),
            "读属性不等于能读内容（只够判断存在性）"
        );
    }
}

/// 只读**子树**（模块目录默认只读）与工作目录：都给 `RIGHTS_RO` 递归，写权只能来自 `rw`。
#[test]
fn grant_targets_keep_module_read_only_and_cwd_read_only() {
    let base = std::env::temp_dir().join("solomni-grant-targets");
    let module = base.join("modules").join("m0");
    let userdata = module.join("userdata");
    let spec = FenceSpec {
        agent: "probe".to_string(),
        private: PathBuf::new(),
        ro_tree: vec![module.clone()],
        rw: vec![userdata.clone()],
        ro: Vec::new(),
        cwd: module.clone(),
        net: false,
    };
    let targets = grant_targets(&spec);
    let find = |p: &std::path::Path| targets.iter().find(|(x, _, _, _)| x == p).cloned();
    assert_eq!(
        find(&module).map(|(_, r, rec, inh)| (r, rec, inh)),
        Some((RIGHTS_RO, true, true)),
        "模块根（也是 cwd）不得拿到写权"
    );
    assert_eq!(
        find(&userdata).map(|(_, r, rec, inh)| (r, rec, inh)),
        Some((RIGHTS_RW, true, true)),
        "userdata 是可写叶子"
    );
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
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![dir.clone()],
        cwd: dir.clone(),
        ro: Vec::new(),
        net: false,
    };
    // 台账落在探针自己的临时目录里（不碰真实 .home/）。
    let home = dir.join("ledger");
    let outcome = prepare_fence(&spec, "cmd", &home);
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
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![dir.clone()],
        cwd: dir.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = dir.join("ledger");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
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
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![dir.clone()],
        ro: vec![ro.clone()],
        cwd: dir.clone(),
        net: false,
    };
    let home = dir.join("ledger");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
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

/// 【残留探针】授权（叶子读写 + 父目录只读属性）→ 撤权后，叶子与父目录上都不得留有该容器 SID
/// 的**任何**显式 ACE。真机残留过一条只有 SYNCHRONIZE 的 (OI)(CI) ACE
/// （tests/gaps.yaml 的 fence.leftover-grant-hides-parent：整棵 tests/ 子树因此在受限进程里不可读），
/// 所以撤净判定不看权限位、只看 SID 在不在场（has_any_ace_for）。
/// 现有撤权测试只盯叶子；这条把父目录一并盯住——grant_targets 的落点清单变了它会先红。
#[test]
fn revoke_leaves_no_container_ace_on_leaf_parents() {
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：残留探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-leftover-probe-{}", std::process::id()));
    let leaf = base.join("leaf");
    std::fs::create_dir_all(&leaf).expect("建探针目录");
    std::fs::write(leaf.join("data.txt"), "x").expect("写探针文件");
    let spec = FenceSpec {
        agent: "probe-leftover".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![leaf.clone()],
        cwd: leaf.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = base.join("ledger");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    eprintln!("[探针] 授权后父目录 {}", dump_aces(&base));
    eprintln!("[探针] 授权后叶子 {}", dump_aces(&leaf));
    assert!(
        has_any_ace_for(sid, &base),
        "授权后父目录上该容器 SID 的显式 ACE 应在场"
    );
    assert!(
        has_ace_for(sid, &base, RIGHTS_STAT),
        "授权后父目录上应有只读属性 ACE"
    );
    assert!(
        has_ace_for(sid, &leaf, RIGHTS_RW),
        "授权后叶子上应有读写 ACE"
    );
    release_fence_home(&spec, Some(&home)).expect("撤权应当成功");
    eprintln!("[探针] 撤权后父目录 {}", dump_aces(&base));
    eprintln!("[探针] 撤权后叶子 {}", dump_aces(&leaf));
    let on_parent = has_any_ace_for(sid, &base);
    let on_leaf = has_any_ace_for(sid, &leaf);
    free_sid(sid);
    clean(&home).expect("台账回收应当成功");
    let _ = std::fs::remove_dir_all(&base);
    assert!(
        !on_parent,
        "撤权后父目录不得残留该容器 SID 的任何 ACE（残留会把父目录对受限进程藏住）"
    );
    assert!(!on_leaf, "撤权后叶子不得残留该容器 SID 的任何 ACE");
}

/// 【真机往返探针】授权（叶子读写 + 父目录只读属性）→ 容器里：能列叶子、能判断"父目录下叶子在
/// 不在"（RIGHTS_STAT 存在的全部理由）、列不了父目录的内容——口径端到端成立，不悄悄放宽。
/// 会创建 AppContainer profile（改本机状态），按测试约定只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑。
#[test]
fn container_roundtrip_sees_leaf_but_not_parent_content() {
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_roundtrip_sees_leaf_but_not_parent_content 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：往返探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-roundtrip-probe-{}", std::process::id()));
    let leaf = base.join("leaf");
    std::fs::create_dir_all(&leaf).expect("建探针目录");
    std::fs::write(leaf.join("data.txt"), "x").expect("写探针文件");
    let spec = FenceSpec {
        agent: "probe-roundtrip".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![leaf.clone()],
        cwd: leaf.clone(),
        ro: Vec::new(),
        net: false,
    };
    let container = container_name(&spec);
    ensure_profile(&container).expect("建容器 profile");
    let home = base.join("ledger");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
    let sid = container_sid(&container).expect("派生容器 SID");

    // 先确认容器**真的**生效：某些受管环境里 AppContainer 会被静默降级（令牌里没有包 SID 组、
    // 也没有 ALL APPLICATION PACKAGES），后续断言会拿环境结论冒充口径结论。没生效就如实跳过。
    let _ = run_in_container(sid, &spec, "cmd /C whoami /groups > g.txt 2>&1");
    let groups = std::fs::read_to_string(leaf.join("g.txt")).unwrap_or_default();
    if !groups.contains("S-1-15-2-") {
        eprintln!(
            "[探针] 本环境没有真正把进程放进 AppContainer（容器内 whoami /groups 无包 SID 组）：往返探针跳过（不静默当作通过）——请在普通 shell 里重跑本探针"
        );
        free_sid(sid);
        let _ = release_fence_home(&spec, Some(&home));
        let _ = clean(&home);
        let _ = delete_profile(&container);
        let _ = std::fs::remove_dir_all(&base);
        return;
    }
    // 1) 叶子能列：工具在自己的数据边界里看得见自己的产物。
    let code =
        run_in_container(sid, &spec, "cmd /C dir /b > out.txt 2>&1").expect("容器进程应当能启动");
    let out = std::fs::read_to_string(leaf.join("out.txt")).unwrap_or_default();
    assert_eq!(code, 0, "容器里列叶子应当成功：{}", out);
    assert!(
        out.contains("data.txt"),
        "容器里应能看到叶子里的文件：{}",
        out
    );

    // 2) 父目录能 stat：对"父目录下叶子在不在"的判断要成立（不是假不存在）。
    let code = run_in_container(
        sid,
        &spec,
        "cmd /C if exist ..\\leaf (echo STAT-OK> stat.txt) else (echo STAT-MISSING> stat.txt)",
    )
    .expect("容器进程应当能启动");
    assert_eq!(code, 0, "容器里判断叶子的存在性应当成功");
    let stat = std::fs::read_to_string(leaf.join("stat.txt")).unwrap_or_default();
    assert!(
        stat.contains("STAT-OK"),
        "父目录下叶子应被判断为存在：{}",
        stat
    );

    // 3) 父目录的内容读不到：只读属性不等于能读（口径不能悄悄放宽）。
    let code = run_in_container(sid, &spec, "cmd /C dir .. > denied.txt 2>&1")
        .expect("容器进程应当能启动");
    assert_ne!(code, 0, "容器里列父目录的内容应当被拒");

    free_sid(sid);
    release_fence_home(&spec, Some(&home)).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    // 探针建的容器 profile 也要带走：测试不在本机留痕。
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    let _ = std::fs::remove_dir_all(&base);
}
