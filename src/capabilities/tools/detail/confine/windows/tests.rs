use super::*;

/// 授权应当成立：必要落点全授上（不成立就把结论打出来——它是这一路的判据）。
fn expect_granted(prep: &FencePrep, what: &str) {
    assert!(prep.ok(), "{}：必要落点授不上 {:?}", what, prep.blocked);
}

/// 把对象 DACL 里的 ACE 逐条转储成可读文本（残留调查用：原始权限位 + 继承标志 + SID + 对象 GUID）。
/// 走 acl_scan：**偏移与认不出的条数都由它一处给出**，诊断不会另抄一套读法而看不出对象 ACE。
fn dump_aces(path: &Path) -> String {
    let scan = match acl_scan(path) {
        Ok(scan) => scan,
        Err(e) => return format!("{}：{}", path.display(), e),
    };
    let mut out = format!("{}：\n", path.display());
    for v in &scan.entries {
        out.push_str(&format!(
            "  type={} flags=0x{:02X} mask=0x{:08X} inherited={} sid={}\n",
            v.ace_type,
            v.flags,
            v.mask,
            v.flags & INHERITED_ACE != 0,
            v.sid
        ));
        if !v.object_type.is_empty() || !v.inherited_object_type.is_empty() {
            out.push_str(&format!(
                "    object={} inherited_object={}\n",
                v.object_type, v.inherited_object_type
            ));
        }
    }
    if scan.unparsed > 0 {
        out.push_str(&format!("  认不出的 ACE：{} 条\n", scan.unparsed));
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
    // （缺了这次命令在容器里起不来，或这次执行做不了该做的事）——判据见 docs/tools/README.md。
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
    expect_granted(&outcome, "授权应当成功");
    let rec = load_record(&home);
    assert!(
        rec.snapshots.iter().any(|s| Path::new(&s.path) == target),
        "根内路径要先落原始安全描述符快照"
    );
    assert!(
        !rec.grants.iter().any(|g| Path::new(&g.path) == target),
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
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
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

/// 【真机往返】对象 ACE（SID 前还带类型 GUID）也进写后核对的账：
/// 真写下一条对象 ACE → 核对看得见它 → 我们的授权往返（记录 → 写 → 读回）不弄丢它 →
/// 再把"弄丢"注入一次，证明核对确实会报（不然这条断言可能永远为真）。
/// 会改本机状态（写临时文件的 DACL），按测试约定只在 --fence-live 下跑；做不了往返预检就如实跳过。
#[test]
fn object_ace_is_covered_by_the_write_then_equal_check() {
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
        agent: "probe-object-ace".to_string(),
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
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
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
    //    刻意不给：给了就等于让容器枚举父目录里的其它格子）；`dir` 与 PowerShell 的 `Get-ChildItem`
    //    在托管 runner 的容器里会被拒（真机实测：同一个叶子上 `for` 枚举与 python 的 `os.listdir` 都正常），
    //    拿它们量会量错东西。
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

    // 4) 列自己的产物：容器要在自己的边界里看得见自己的东西。判据用 cmd 的 `for` 枚举（FindFirstFile）——
    //    **不用 `dir`**：同一叶子上 `dir` 与 PowerShell 的 `Get-ChildItem` 会被拒，`for` 与 python 的 `os.listdir` 正常。
    let code = run_in_container(sid, &spec, "(for %f in (*) do @echo %f) > listing.txt 2>&1")
        .expect("容器进程应当能启动");
    let listing = std::fs::read_to_string(leaf.join("listing.txt")).unwrap_or_default();
    assert!(
        code == 0 && listing.contains("data.txt"),
        "容器里要能列出自己的产物（exit={}，拿到 {:?}）",
        code,
        listing
    );
    // `dir` 只记录、不断言：它是这个环境的已知异常，环境变了要能在日志里看见。
    let dir_code =
        run_in_container(sid, &spec, "dir /b > dir-listing.txt 2>&1").expect("容器进程应当能启动");
    eprintln!(
        "[诊断] 同一叶子上的 cmd dir：exit={} 输出={:?}",
        dir_code,
        std::fs::read_to_string(leaf.join("dir-listing.txt"))
            .unwrap_or_default()
            .trim()
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
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
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

/// 本机有没有能跑的 node（没有就如实跳过需要它的探针）。
fn node_runs() -> bool {
    std::process::Command::new("node")
        .arg("-v")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 【真机往返探针·node 形态】容器里 `node <文件>` 必须跑得起来：node 的 `fs.realpathSync` 会先 lstat 盘卷根、
/// 再逐级 lstat 祖先前缀，而这两类落点按设计都不在可达范围——没有解释器基线（`NODE_OPTIONS` 跳过 realpath），
/// 进程在脚本执行前就 EPERM 死。所以分两段：带运行期环境跑通一次完整往返（模块脚本 + require 进来的依赖 +
/// 产物落进 userdata），再用同一份环境**关掉开关**复现失败现场（根因钉死，不是"容器坏了"）。
/// 夹具的模块根带一份 package.json：主模块格式判定会逐级向上找作用域配置，容器里够不到的祖先会让它报
/// `ERR_INVALID_PACKAGE_CONFIG` 判死；模块要自带这份作用域（见 MODULE_SPEC 的「模块目录就是发现边界」）。
/// 会创建 AppContainer profile（改本机状态），只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑；本机没有 node 时如实跳过。
#[test]
fn container_runs_a_node_module_tool_with_realpath_skipped() {
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_runs_a_node_module_tool_with_realpath_skipped 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：node 形态往返探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    if !node_runs() {
        eprintln!("[探针] 本机没有能跑的 node：node 形态往返探针跳过（不静默当作通过）");
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-node-probe-{}", std::process::id()));
    let module = base.join("modules").join("m0");
    let userdata = module.join("userdata");
    std::fs::create_dir_all(module.join("tools")).expect("建模块 tools");
    std::fs::create_dir_all(&userdata).expect("建模块 userdata");
    // 主脚本 require 同目录的依赖：依赖那一路也要 realpath，光有主模块那个开关不够（这正是两个开关的理由）。
    // 模块根放一份 package.json：主模块格式判定会逐级向上找作用域配置，够不到的祖先会直接把它判死。
    std::fs::write(module.join("package.json"), "{ \"type\": \"commonjs\" }")
        .expect("写模块根 package.json");
    std::fs::write(
        module.join("tools").join("helper.js"),
        "module.exports = { marker: 'node-tool-ok' };",
    )
    .expect("写依赖脚本");
    std::fs::write(
        module.join("tools").join("report.js"),
        "const helper = require('./helper.js'); const fs = require('fs'); fs.writeFileSync(process.argv[2], helper.marker);",
    )
    .expect("写主脚本");
    let spec = FenceSpec {
        agent: "probe-node".to_string(),
        rw: vec![userdata.clone()],
        ro: Vec::new(),
        ro_tree: vec![module.clone()],
        private: userdata.clone(),
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
    let command = "node tools/report.js userdata/report.txt";
    let prep = prepare_fence(&spec, command, &home);
    if let Some(blocked) = &prep.blocked {
        // 解释器目录（PATH 里那个 node 的安装处）授不上权限时，容器里读不到解释器，这条链路本机做不了：
        // 环境结论，如实跳过。授权代码真坏了会被同族的往返探针响亮抓住（它们用的是系统里的 cmd）。
        eprintln!(
            "[探针] 本机给授权落点写不了权限（{}）：node 形态往返探针跳过（不静默当作通过）",
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

    // ① 运行期白名单这一层就要带上两个开关：机制不在这儿就位，容器里再补已经晚了。
    let env = crate::capabilities::tools::detail::confine::fence_env(&spec, command);
    let opts = env
        .iter()
        .find(|(k, _)| k == "NODE_OPTIONS")
        .map(|(_, v)| v.to_string_lossy().to_string())
        .unwrap_or_default();
    assert!(
        opts.contains("--preserve-symlinks") && opts.contains("--preserve-symlinks-main"),
        "运行期环境要带两个开关（缺哪个都会在另一半上照样 realpath）：{:?}",
        opts
    );

    // ② 容器里跑得通：产物落在 userdata（rw 里），内容来自 require 进来的依赖。
    let code = run_in_container(sid, &spec, command).expect("容器进程应当能启动");
    let got = std::fs::read_to_string(userdata.join("report.txt")).unwrap_or_default();
    assert!(
        code == 0 && got.contains("node-tool-ok"),
        "容器里 node 工具应当跑通（exit={}，产物 {:?}）——容器里 stdout/stderr 已透传到本进程输出",
        code,
        got.trim()
    );

    // ③ 对照：命令行把两个开关关掉（命令行优先于 NODE_OPTIONS）→ 复现"脚本执行前就倒"的失败现场。
    let off = "node --no-preserve-symlinks --no-preserve-symlinks-main tools/report.js userdata/off.txt 2> userdata/off-err.txt";
    let off_code = run_in_container(sid, &spec, off).expect("容器进程应当能启动");
    let off_err = std::fs::read_to_string(userdata.join("off-err.txt")).unwrap_or_default();
    if off_code == 0 {
        eprintln!(
            "[诊断] 关掉开关也跑通了（本机这条路径不再需要它）：{:?}",
            off_err.trim()
        );
    } else {
        assert!(
            off_err.contains("lstat"),
            "关掉开关的失败现场应当是 realpath 的 lstat 落点（exit={}，原话 {:?}）",
            off_code,
            off_err.trim()
        );
    }

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
            .any(|s| Path::new(&s.path) == target && s.bytes == bytes),
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
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let rec = load_record(&home);
    assert!(
        rec.snapshots.iter().any(|s| Path::new(&s.path) == target),
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

/// 【缺落点跳过】路径不存在的落点只跳过、不判整次失败：存在的照常授权，台账里不留不存在的那条。
#[test]
fn missing_grant_target_is_skipped_and_not_journaled() {
    let base = std::env::temp_dir().join(format!("solomni-skip-probe-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "skip") {
        eprintln!("[探针] 本机做不了 ACL 完整往返（写→读回→撤）：缺落点探针跳过（不静默当作通过）");
        discard(&base);
        return;
    }
    let absent = base.join("missing");
    let spec = FenceSpec {
        agent: "probe-skip".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone(), absent.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let home = base.join(".home");
    let outcome = prepare_fence(&spec, "cmd", &home);
    expect_granted(&outcome, "不存在的落点应跳过、不判整次失败");
    let rec = load_record(&home);
    assert!(
        !rec.snapshots.iter().any(|s| Path::new(&s.path) == absent),
        "跳过的落点不得留快照"
    );
    assert!(
        !rec.grants.iter().any(|g| Path::new(&g.path) == absent),
        "跳过的落点不得留 ACE 摘要"
    );
    assert!(
        rec.snapshots.iter().any(|s| Path::new(&s.path) == target),
        "存在的落点要照常授权"
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
    // 台账落不了盘 = **必要**的一环走不动：如实进结论（调用方据此问用户或拒绝，不许降级）。
    assert_eq!(
        outcome.blocked.as_ref().map(|b| b.part),
        Some(FencePart::Ledger),
        "台账落不了盘必须如实报成必要落点授不上：{:?}",
        outcome.blocked
    );
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let wrote = has_ace_for(sid, &target, RIGHTS_RW);
    free_sid(sid);
    assert!(!wrote, "台账没落盘就不许写 ACL");
    discard(&base);
}

/// 【按条处置】清单看得见每一条的现状；按路径只还原一条、按 SID + 路径只撤一条，
/// 其余条目与整份 DACL 不受影响；台账外的根外残留也能按 SID + 路径撤掉并如实标注它不在台账里。
#[test]
fn ledger_catalog_and_per_item_disposal_keep_the_rest_untouched() {
    let base = std::env::temp_dir().join(format!("solomni-grant-probe-{}", std::process::id()));
    let inside = base.join("inside");
    let outside =
        std::env::temp_dir().join(format!("solomni-grant-outside-{}", std::process::id()));
    std::fs::create_dir_all(&inside).expect("建根内目录");
    std::fs::create_dir_all(&outside).expect("建根外目录");
    if !acl_round_trip(&base, "grant") || !acl_round_trip(&outside, "grant-outside") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：按条处置探针跳过（不静默当作通过）"
        );
        discard(&base);
        discard(&outside);
        return;
    }
    let home = base.join(".home");
    let spec = FenceSpec {
        agent: "probe-grant".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![inside.clone()],
        cwd: inside.clone(),
        ro: Vec::new(),
        net: false,
    };
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let sid_text = sid_to_string(sid);
    // 一条根内授权（走快照）+ 一条根外授权（走摘要）：都经“先落台账、再写 ACL、写后核对”那条路。
    let mut rec = load_record(&home);
    let inside_target = GrantTarget {
        path: inside.clone(),
        rights: RIGHTS_RW,
        recursive: false,
        inherit: false,
        part: FencePart::DataBoundary,
    };
    grant_one_journaled(&home, &mut rec, sid, &sid_text, &inside_target, &base)
        .expect("根内授权应当成功");
    let outside_target = GrantTarget {
        path: outside.clone(),
        rights: RIGHTS_RO,
        recursive: false,
        inherit: false,
        part: FencePart::Interpreter,
    };
    grant_one_journaled(&home, &mut rec, sid, &sid_text, &outside_target, &base)
        .expect("根外授权应当成功");
    free_sid(sid);
    // 清单：两条都看得见，都如实标成“现在还在”，且与盘上的 ACE 对得上。
    let view = catalog(&home);
    assert_eq!(
        view.entries.len(),
        2,
        "台账里应当有两条：{:?}",
        view.entries
    );
    assert!(
        view.entries.iter().all(|e| e.present),
        "两条授权都还在盘上：{:?}",
        view.entries
    );
    assert!(
        view.entries
            .iter()
            .any(|e| e.kind == "snapshot" && Path::new(&e.path) == inside),
        "根内那条要如实标成快照：{:?}",
        view.entries
    );
    assert!(
        view.entries
            .iter()
            .any(|e| e.kind == "grant" && Path::new(&e.path) == outside),
        "根外那条要如实标成授权摘要：{:?}",
        view.entries
    );
    // 按路径只还原根内那一条：它自己的容器 ACE 消失，根外那条与它的整份 DACL 不受影响。
    let before = acl_entries(&outside).expect("读根外 ACE 集合");
    let said = restore_one(&home, &inside).expect("按路径还原应当成功");
    assert!(
        said.contains(&inside.to_string_lossy().into_owned()),
        "{}",
        said
    );
    let sid = container_sid(&container_name(&spec)).expect("再派生容器 SID");
    assert!(
        !has_any_ace_for(sid, &inside),
        "还原后根内那条不该再有该容器 SID 的 ACE"
    );
    assert!(has_any_ace_for(sid, &outside), "其余条目不受影响");
    free_sid(sid);
    assert_eq!(
        acl_entries(&outside).expect("读根外 ACE 集合"),
        before,
        "还原一条不得改动其余条目的 DACL"
    );
    assert_eq!(catalog(&home).entries.len(), 1, "还原过的那条已从台账销掉");
    // 按 SID + 路径撤根外那一条，只动它。
    let said = revoke_grant(&home, &sid_text, &outside).expect("按条撤销应当成功");
    assert!(said.contains("按台账"), "{}", said);
    let sid = container_sid(&container_name(&spec)).expect("再派生容器 SID");
    assert!(
        !has_any_ace_for(sid, &outside),
        "撤过的落点不该再有该 SID 的 ACE"
    );
    free_sid(sid);
    assert!(load_record(&home).is_empty(), "两条都处置完，台账应当清空");
    // 台账外残留：直接写一条 ACE、不经台账，再按 SID + 路径撤掉——如实标注它不在台账里。
    let orphan = container_sid("Solomni.Agent.GrantOrphanProbe").expect("派生孤儿容器 SID");
    let orphan_text = sid_to_string(orphan);
    grant_verified(orphan, &outside, RIGHTS_RO, false, false).expect("写下台账外 ACE");
    let said = revoke_grant(&home, &orphan_text, &outside).expect("台账外残留也要撤得掉");
    assert!(
        said.contains("不在台账里"),
        "要如实标注它不在台账里：{}",
        said
    );
    assert!(!has_any_ace_for(orphan, &outside), "台账外 ACE 应当被撤掉");
    free_sid(orphan);
    // 台账里没有的快照：按条还原如实拒绝（当次调用什么都没做）。
    assert!(
        restore_one(&home, &inside).is_err(),
        "台账里没有这条快照就如实拒绝"
    );
    discard(&base);
    discard(&outside);
}
