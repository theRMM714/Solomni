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

/// 探针目录的收尾：删不掉就如实打印，不静默（受限环境里写坏的 DACL 会让删除失败）。
fn discard(base: &Path) {
    if let Err(e) = std::fs::remove_dir_all(base) {
        if base.exists() {
            eprintln!("[诊断] 探针目录未清理（{}）：{}", base.display(), e);
        }
    }
}

/// ACL 往返预检：在自有 base 里记 ACE 集合 → 写 → 读回 → 撤 → 集合不变；做不了就返回 false。
fn acl_round_trip(base: &Path, tag: &str) -> bool {
    let target = base.join("acl-preflight");
    if std::fs::create_dir_all(&target).is_err() {
        return false;
    }
    let before = match acl_entries(&target) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let spec = FenceSpec {
        agent: format!("acl-preflight-{tag}"),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let sid = match container_sid(&container_name(&spec)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let ok = grant_one(sid, &target, RIGHTS_RO, false, false).is_ok()
        && has_any_ace_for(sid, &target)
        && revoke_one(sid, &target, false).is_ok()
        && acl_entries(&target)
            .map(|after| lost_entries(&before, &after).is_empty())
            .unwrap_or(false);
    free_sid(sid);
    ok
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
    // 同一个路径可能有多条（父目录 STAT + 叶子 RO/RW），所以要按**权限位**找那一条，
    // 不能取第一条（那往往是父目录的 STAT）。
    let has = |p: &std::path::Path, r: u32, rec: bool, inh: bool| {
        targets
            .iter()
            .any(|(x, rr, rc, ii)| x == p && *rr == r && *rc == rec && *ii == inh)
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
    let base = std::env::temp_dir().join(format!("solomni-grant-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "grant") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：授权探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let spec = FenceSpec {
        agent: "probe".to_string(),
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
    assert!(outcome.is_ok(), "授权应当成功：{:?}", outcome.err());
    let rec = load_record(&home);
    assert!(
        rec.snapshots.iter().any(|(p, _)| Path::new(p) == target),
        "根内路径要先落原始安全描述符快照"
    );
    assert!(
        !rec.grants.iter().any(|(_, p, _)| Path::new(p) == target),
        "根内路径收尾走快照还原，不记 ACE 摘要"
    );
    // 收尾必须把自己写下的权限项按台账撤掉或还原：测试不在本机留痕。
    let report = clean(&home).expect("回收应当成功");
    assert!(report.contains("撤销"), "回收要如实报数量：{}", report);
    discard(&base);
}

/// 撤销的真效果：授权 → 撤权 → 目标目录上不再有该容器 SID 的 ACE。
/// 与上一条一样先做完整往返预检（本机受限沙箱会如实跳过）。
#[test]
fn revoke_removes_the_container_ace_from_the_given_roots() {
    let base = std::env::temp_dir().join(format!("solomni-revoke-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "revoke") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：撤销探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let spec = FenceSpec {
        agent: "probe".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = base.join(".home");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
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
        agent: "probe-ro".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        ro: vec![ro.clone()],
        cwd: target.clone(),
        net: false,
    };
    let home = base.join(".home");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
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
    let home = base.join(".home");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    // 诊断输出用 [诊断] 前缀：门禁只把 [探针] 当 env-skip，别让这两行把"跳过数"充大。
    eprintln!("[诊断] 授权后父目录 {}", dump_aces(&base));
    eprintln!("[诊断] 授权后叶子 {}", dump_aces(&leaf));
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
    release_fence(&spec, &home).expect("撤权应当成功");
    eprintln!("[诊断] 撤权后父目录 {}", dump_aces(&base));
    eprintln!("[诊断] 撤权后叶子 {}", dump_aces(&leaf));
    let on_parent = has_any_ace_for(sid, &base);
    let on_leaf = has_any_ace_for(sid, &leaf);
    free_sid(sid);
    clean(&home).expect("台账回收应当成功");
    discard(&base);
    assert!(
        !on_parent,
        "撤权后父目录不得残留该容器 SID 的任何 ACE（残留会把父目录对受限进程藏住）"
    );
    assert!(!on_leaf, "撤权后叶子不得残留该容器 SID 的任何 ACE");
}

/// 失败现场：把容器自己的视图落进叶子再报出来（"被拒"还是"真没有"要分得清）。
/// 诊断本身只写授权落点，不会把失败吞掉。
fn container_diag(sid: PSID, spec: &FenceSpec, leaf: &Path) -> String {
    let _ = run_in_container(sid, spec, "whoami /priv > diag.txt 2>&1");
    let _ = run_in_container(
        sid,
        spec,
        "cd .. >> diag.txt 2>&1 & echo CD-DONE >> diag.txt",
    );
    std::fs::read_to_string(leaf.join("diag.txt")).unwrap_or_default()
}
/// 【真机往返探针】授权（叶子读写 + 父目录只读属性）→ 容器里：在自己的边界里写得到也读得回、
/// 从父目录**按名**穿得到叶子里的文件（RIGHTS_STAT 存在的全部理由：存在性判断不会假不存在）、
/// 却读不到父目录里的其它条目——口径端到端成立。
/// 会创建 AppContainer profile（改本机状态），按测试约定只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑。
///
/// **这里不再用"容器内 `whoami /groups` 里找包 SID 组"判容器是否生效**：包 SID 在现代 Windows 上
/// 不在令牌的组列表里（在 `TokenAppContainerSid` 字段），容器真生效也会因此被判成"环境降级"而跳过
/// （真机令牌转储：`TokenIsAppContainer=1`、`AppContainerSid` = 该 profile 的 SID、`capabilities=0`，
/// 而同一个进程的 `whoami /groups` 里没有任何 `S-1-15-2-`）。现在改由**行为**给结论：
/// 授权落点写得进、父目录内容列不到；做不到就响亮失败（那才是口径破了，不是环境不允许）。
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
    // 父目录里另放一个条目：容器应当**读不到**它（只读属性只够按名穿过，不够读内容）。
    std::fs::write(base.join("parent-secret.txt"), "PARENT-SECRET").expect("写父目录条目");
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
    // 建不出 profile 分两种：环境不允许（如实跳过）与我们的步骤写错（响亮失败），分开处理。
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
    let home = base.join("ledger");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
    let sid = container_sid(&container).expect("派生容器 SID");

    // 1) 数据边界里写得到、读得回（cwd 相对路径，与真实工具的形态一致）：写不进授权落点等于授权没生效，
    //    这一条同时也是"容器进程真拿到了 rw 写权"的正向对照。
    let code = run_in_container(
        sid,
        &spec,
        "echo data > out.txt & type out.txt > read.txt 2>&1",
    )
    .expect("容器进程应当能启动");
    let read = std::fs::read_to_string(leaf.join("read.txt")).unwrap_or_default();
    if code != 0 || !read.contains("data") {
        panic!(
            "容器里写自己的边界并读回来应当成功（exit={}，拿到 {:?}）：容器内诊断——{}",
            code,
            read.trim(),
            container_diag(sid, &spec, &leaf)
        );
    }

    // 2) 父目录能被**按名穿过**：从父目录按名字读到叶子里的文件——这正是父目录只拿"只读属性"
    //    （RIGHTS_STAT）的那条口径：中间目录判不了存在性时，工具会以为"父目录不存在"而一路往上建
    //    （真机 CI 上抓到过 `WinError 5: 'D:\'`）。
    //    **不用 `if exist` / `attrib` / `dir` 当尺子**：前两个取属性要走父目录的**列举权**（设计上
    //    刻意不给：给了就等于让容器枚举父目录里的其它格子）；`dir` 在托管 runner 的容器里另有异常
    //    （见 tests/gaps.yaml 的 fence.container-dir-listing-denied），拿它们量会量错东西。
    let code = run_in_container(sid, &spec, "type ..\\leaf\\data.txt > reach.txt 2>&1")
        .expect("容器进程应当能启动");
    let reach = std::fs::read_to_string(leaf.join("reach.txt")).unwrap_or_default();
    if code != 0 || !reach.contains('x') {
        panic!(
            "父目录要能按名穿过（exit={}，拿到 {:?} = 穿不过去，存在性判断会假不存在）：容器内诊断——{}",
            code,
            reach.trim(),
            container_diag(sid, &spec, &leaf)
        );
    }

    // 3) 父目录里的其它条目读不到：只读属性只够按名穿过，不等于能读父目录的内容。
    let code = run_in_container(sid, &spec, "type ..\\parent-secret.txt > psecret.txt 2>&1")
        .expect("容器进程应当能启动");
    let got = std::fs::read_to_string(leaf.join("psecret.txt")).unwrap_or_default();
    assert!(
        code != 0 && !got.contains("PARENT-SECRET"),
        "父目录里的其它条目不该读得到：{:?}（exit={}）",
        got,
        code
    );

    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    // 探针建的容器 profile 也要带走：测试不在本机留痕。
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);
}

/// 【真机往返探针·模块形态】**垂直分工的权限方位**端到端验收：`ro_tree` = 模块根（递归只读）、
/// `rw` = `<模块>/userdata`（可写）、`cwd` = 模块根，而另一席的沙箱不在任何授权里。容器里要同时成立：
/// ① 模块脚本读得到、`userdata/` 写得进；② 模块目录里写新文件被机制拒（写权只来自 rw）；
/// ③ 另一席沙箱里的明文拿不到（跨 agent 不可达）。
/// 会创建 AppContainer profile（改本机状态），只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑。
#[test]
fn container_roundtrip_keeps_module_read_only_and_peer_unreachable() {
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_roundtrip_keeps_module_read_only_and_peer_unreachable 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：模块形态往返探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-module-probe-{}", std::process::id()));
    let module = base.join("modules").join("m0");
    let userdata = module.join("userdata");
    let peer = base.join("session").join("peer");
    std::fs::create_dir_all(&userdata).expect("建模块 userdata");
    std::fs::create_dir_all(&peer).expect("建另一席的沙箱");
    std::fs::write(module.join("script.py"), "print(1)\n").expect("写模块脚本");
    std::fs::write(peer.join("secret.txt"), "PEER-SECRET").expect("写另一席的明文");
    let spec = FenceSpec {
        agent: "probe-module".to_string(),
        private: userdata.clone(),
        ro_tree: vec![module.clone()],
        rw: vec![userdata.clone()],
        ro: Vec::new(),
        cwd: module.clone(),
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
    let home = base.join("ledger");
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
    let sid = container_sid(&container).expect("派生容器 SID");

    // ① 模块目录读得到、userdata 写得进：一条命令验两件事（脚本内容经重定向落进 userdata）。
    let code = run_in_container(sid, &spec, "type script.py > userdata\\read.txt 2>&1")
        .expect("容器进程应当能启动");
    let read = std::fs::read_to_string(userdata.join("read.txt")).unwrap_or_default();
    assert_eq!(code, 0, "模块目录要读得到、userdata 要写得进：{}", read);
    assert!(read.contains("print(1)"), "模块脚本内容要读得到：{}", read);

    // ② 模块目录默认只读：模块根里写不进新文件（写权只来自 rw）。
    let forbidden = module.join("should-not-exist.txt");
    let code =
        run_in_container(sid, &spec, "echo x > should-not-exist.txt").expect("容器进程应当能启动");
    assert!(
        !forbidden.exists() && code != 0,
        "模块目录默认只读：模块根里不该写得进新文件（exit={}）：{:?}",
        code,
        std::fs::read_dir(&module).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
    );

    // ③ 另一席的沙箱不在任何授权里：明文拿不到（跨 agent 不可达）。
    let peer_file = peer.join("secret.txt");
    let code = run_in_container(
        sid,
        &spec,
        &format!("type \"{}\" > userdata\\peer.txt 2>&1", peer_file.display()),
    )
    .expect("容器进程应当能启动");
    let got = std::fs::read_to_string(userdata.join("peer.txt")).unwrap_or_default();
    assert!(
        !got.contains("PEER-SECRET") && code != 0,
        "另一席的沙箱不可达：拿到 {:?}（exit={}）",
        got,
        code
    );

    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);
}

/// ACL 写法的语义与后果验证：非递归 `SetNamedSecurityInfoW` 与递归 `TreeSetNamedSecurityInfoW`（带/不带继承标志）
/// 各自把 ACE 铺到哪些节点、写入后目标还能不能读回自己的 DACL——`grant_one` 的 `recursive` 分支就靠 TreeSet，
/// 「去继承、全显式」能不能成立全看这里。探针只打印事实、不预设结论；本机不允许改 ACL 时如实跳过。
#[test]
fn acl_write_flavours_are_probed_for_scope_and_readability() {
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
        agent: "acl-probe-preflight".to_string(),
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
            agent: format!("acl-probe-{tag}"),
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
            "[探针] {tag}：grant_ok={} 根ACE={} 子目录ACE={} 子文件ACE={} revoke_ok={}",
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

/// 【台账契约】写前先落盘：快照与授权摘要都要能在 ACL 改动前读回；收尾按快照整体还原。
#[test]
fn journal_records_snapshot_and_grant_before_touching_acl() {
    let base = std::env::temp_dir().join(format!("solomni-journal-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "journal") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：台账探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let bytes = sd_bytes(&target).expect("读原始安全描述符");
    let mut rec = load_record(&home);
    assert!(
        journal_add_snapshot(&home, &mut rec, &target, bytes.clone()).expect("台账先落盘"),
        "第一次写该路径的快照应当新增"
    );
    let reread = load_record(&home);
    assert!(
        reread
            .snapshots
            .iter()
            .any(|(p, b)| Path::new(p) == target && b == &bytes),
        "动 ACL 之前台账里就要有原始安全描述符"
    );
    let spec = FenceSpec {
        agent: "probe-journal".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    prepare_fence(&spec, "cmd", &home).expect("授权应当成功");
    let rec = load_record(&home);
    assert!(
        rec.snapshots.iter().any(|(p, _)| Path::new(p) == target),
        "收尾还原要用的快照必须在场"
    );
    clean(&home).expect("回收应当成功");
    discard(&base);
}

/// 【写后核对】写下前先记 ACE 集合；写入并还原后，原有 ACE 集合必须与之前一致。
#[test]
fn write_then_restore_keeps_the_original_ace_set() {
    let base = std::env::temp_dir().join(format!("solomni-multiset-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "multiset") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：ACE 集合探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let before = acl_entries(&target).expect("读写入前 ACE 集合");
    let bytes = sd_bytes(&target).expect("存原始安全描述符");
    let spec = FenceSpec {
        agent: "probe-multiset".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    grant_verified(sid, &target, RIGHTS_RO, false, false).expect("写入并核对应当成功");
    assert!(
        has_ace_for(sid, &target, RIGHTS_RO),
        "写入后我们的 ACE 要在场"
    );
    free_sid(sid);
    restore_sd(&target, &bytes).expect("还原原始安全描述符");
    let after = acl_entries(&target).expect("读还原后 ACE 集合");
    // 集合语义：还原可能把重复 ACE 收敛成一份，出现次数变少不算不一致；不同三元组必须一致。
    assert!(
        lost_entries(&before, &after).is_empty() && lost_entries(&after, &before).is_empty(),
        "写入并还原后原有 ACE 集合必须一致"
    );
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    assert!(
        !has_any_ace_for(sid, &target),
        "还原后不该再有该容器 SID 的 ACE"
    );
    free_sid(sid);
    discard(&base);
}

/// 【失败回滚】路径不存在这类写不了的落点必须如实返回 Err，且不给它留台账条目。
#[test]
fn failed_grant_is_rolled_back_and_not_journaled() {
    let base = std::env::temp_dir().join(format!("solomni-rollback-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "rollback") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：回滚探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let missing = base.join("missing");
    let spec = FenceSpec {
        agent: "probe-rollback".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone(), missing.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = base.join(".home");
    let outcome = prepare_fence(&spec, "cmd", &home);
    assert!(outcome.is_err(), "写不了的落点必须如实失败：{:?}", outcome);
    let rec = load_record(&home);
    assert!(
        !rec.snapshots.iter().any(|(p, _)| Path::new(p) == missing),
        "失败落点不得留快照"
    );
    assert!(
        !rec.grants.iter().any(|(_, p, _)| Path::new(p) == missing),
        "失败落点不得留 ACE 摘要"
    );
    clean(&home).expect("回收应当成功");
    discard(&base);
}

/// 【孤儿回收】没有台账、也没有 profile 时，按显式包 SID 也要能连树撤掉残留。
#[test]
fn sweep_reclaims_orphan_package_ace_without_a_ledger() {
    let base = std::env::temp_dir().join(format!("solomni-sweep-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "sweep") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：孤儿回收探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let sid = container_sid("Solomni.Agent.SweepOrphanProbe").expect("派生容器 SID");
    grant_one(sid, &target, RIGHTS_RW, false, false).expect("写下孤儿 ACE");
    assert!(has_any_ace_for(sid, &target), "孤儿 ACE 要在场");
    let swept = sweep_orphan_aces(&base).expect("孤儿清扫应当成功");
    assert!(swept >= 1, "至少在 target 处命中一次");
    assert!(
        !has_any_ace_for(sid, &target),
        "清扫后不该再有该包 SID 的显式 ACE"
    );
    free_sid(sid);
    discard(&base);
}

/// 【先落盘】台账落不了盘时绝不动 ACL：写被台账门禁挡住，目标目录不留任何我们的 ACE。
#[test]
fn journal_failure_blocks_the_acl_write() {
    let base = std::env::temp_dir().join(format!("solomni-journal-gate-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "journal-gate") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：先落盘探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    // home 用一个普通文件占位：save_record 的 create_dir_all 必然失败，台账落不了盘。
    let home = base.join("home-as-file");
    std::fs::write(&home, b"not a dir").expect("写占位文件");
    let spec = FenceSpec {
        agent: "probe-journal-gate".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let outcome = prepare_fence(&spec, "cmd", &home);
    assert!(
        outcome.is_err(),
        "台账落不了盘时必须如实失败：{:?}",
        outcome
    );
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let wrote = has_ace_for(sid, &target, RIGHTS_RW);
    free_sid(sid);
    assert!(!wrote, "台账没落盘就不许写 ACL");
    discard(&base);
}
