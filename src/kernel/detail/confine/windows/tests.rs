//! 目的：Windows 围栏的真机探针——AppContainer 授权、容器往返、ACL 台账与撤权。
//! 管：需要改本机权限项（建 profile、写 ACL）的用例，--fence-live 才跑。
//! 不管：非 Windows 平台的行为（见 tests/linux、tests/macos）。
//! 联动：机制在 confine/windows/ 各文件；判据见 docs/kernel/README.md 的「围栏落点：必要与可选」。

use super::super::Owner;
use super::*;

/// 授权应当成立：必要落点全授上（不成立就把结论打出来——它是这一路的判据）。
fn expect_granted(prep: &FencePrep, what: &str) {
    assert!(prep.ok(), "{}：必要落点授不上 {:?}", what, prep.blocked);
}

/// 目的：网络能力 SID 只在 `net=true` 时建出来；`net=false` 不授任何 capability（断网）。
#[test]
fn internet_client_capability_is_only_built_when_net_is_true() {
    assert!(container::internet_client_sid(false)
        .expect("net=false 不该失败")
        .is_none());
    let sid = container::internet_client_sid(true)
        .expect("建 internetClient 能力 SID")
        .expect("net=true 应该有一个能力");
    assert!(!sid.is_empty(), "能力 SID 不该是空字节");
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
        lease: String::new(),
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

/// 容器 profile **按调用身份（agent + lease）**：同一调用两次派生同一个身份；
/// 同 agent 的两个并发调用、不同 agent，都必须是不同身份（否则窄调用看得见宽调用临时加的根）。
#[test]
fn container_profile_follows_the_call_identity() {
    let a = FenceSpec {
        agent: "甲".to_string(),
        lease: "甲#call-1".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![PathBuf::from("session").join("w1")],
        cwd: PathBuf::from("modules").join("m0"),
        ro: Vec::new(),
        net: false,
    };
    // 同一个调用重复派生：身份稳定（外层授权与守门进程各算一次必须一致）。
    assert_eq!(container_name(&a), container_name(&a.clone()));
    // 同一 agent 的另一次调用：独立身份。
    let mut b = a.clone();
    b.lease = "甲#call-2".to_string();
    assert_ne!(
        container_name(&a),
        container_name(&b),
        "同一 agent 的两次调用不能共用一个 profile（窄调用会看到宽调用的根）"
    );
    // 换 agent：自然不共用。
    let mut c = a.clone();
    c.agent = "乙".to_string();
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
    let dir = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("work");
    let module_root = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("modules")
        .join("m0");
    let spec = FenceSpec {
        agent: "probe".to_string(),
        lease: String::new(),
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
    let private = std::env::temp_dir()
        .join("solomni-grant-targets")
        .join("a")
        .join("b")
        .join("work");
    let spec = FenceSpec {
        agent: "probe".to_string(),
        lease: String::new(),
        private: private.clone(),
        ro_tree: Vec::new(),
        rw: vec![private.clone()],
        ro: Vec::new(),
        cwd: private.clone(),
        net: false,
    };
    let targets = grant_targets(&spec);
    let find = |p: &std::path::Path| targets.iter().find(|t| t.path == p).cloned();
    for anc in private.ancestors().skip(1) {
        if anc.as_os_str().is_empty() || anc.parent().is_none() {
            continue;
        }
        let got = find(anc).unwrap_or_else(|| panic!("祖先要在落点清单里：{}", anc.display()));
        assert_eq!(got.rights, RIGHTS_STAT, "祖先只授读属性：{}", anc.display());
        assert!(!got.recursive && !got.inherit, "祖先不递归、不继承");
        assert_eq!(got.part, FencePart::Parent);
    }
}

/// 只读**子树**（模块目录对 agent 只读）与工作目录：都给 `RIGHTS_RO` 递归，写权只能来自 `rw`。
#[test]
fn grant_targets_keep_module_read_only_and_cwd_read_only() {
    let base = std::env::temp_dir().join("solomni-grant-targets");
    let module = base.join("modules").join("m0");
    let userdata = module.join("userdata");
    let spec = FenceSpec {
        agent: "probe".to_string(),
        lease: String::new(),
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
        lease: String::new(),
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

    // 2) 父目录能被**按名穿过**（这正是「父目录只拿只读属性」那条口径）：从父目录按名字读到叶子里的文件。
    //    不用 if exist / attrib / dir 当尺子——前两个要走父目录的列举权；dir 与 Get-ChildItem 在容器里会被拒。
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
        lease: String::new(),
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

    // ② 模块目录对 agent 只读：模块根里写不进新文件（写权只来自 rw）。
    let forbidden = module.join("should-not-exist.txt");
    let code =
        run_in_container(sid, &spec, "echo x > should-not-exist.txt").expect("容器进程应当能启动");
    assert!(
        !forbidden.exists() && code != 0,
        "模块目录对 agent 只读：模块根里不该写得进新文件（exit={}）：{:?}",
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
        lease: String::new(),
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
    let env = crate::kernel::detail::confine::fence_env(&spec, command);
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

/// 【真机】容器里**绝对路径**的程序也要能跑：执行前把程序名改写成相对 cwd 的路径。
///   现象：同一文件用 `D:\…\x.exe` 报 Access denied，用 `..\x.exe` 正常（相对路径走 cwd 的目录句柄，
///   不遍历卷根；非系统盘的卷根 DACL 要管理员才写得动）。容器执行因此在 `run_in_container` 里做这次改写。
/// 会创建 AppContainer profile 与写目录 ACL，只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑。
#[test]
fn container_runs_a_program_given_by_absolute_path() {
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_runs_a_program_given_by_absolute_path 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：绝对路径执行探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base = repo.join("target").join("probe-exec");
    let c_leaf = base.join("c-leaf");
    let d_leaf = base.join("d-leaf");
    std::fs::create_dir_all(&c_leaf).expect("建 cwd 夹具");
    std::fs::create_dir_all(&d_leaf).expect("建相邻夹具");
    let cmd_exe = PathBuf::from(std::env::var("SystemRoot").unwrap_or_default())
        .join("System32")
        .join("cmd.exe");
    std::fs::copy(&cmd_exe, c_leaf.join("cmd2.exe")).expect("拷到 cwd");
    std::fs::copy(&cmd_exe, d_leaf.join("cmd2.exe")).expect("拷到相邻目录");
    let spec = FenceSpec {
        agent: "probe-exec".to_string(),
        lease: String::new(),
        rw: vec![c_leaf.clone(), d_leaf.clone()],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: c_leaf.clone(),
        cwd: c_leaf.clone(),
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
    let sid = container_sid(&container).expect("派生 SID");
    // 相对名（基线对照）与绝对路径（相邻目录，必须被改写成相对路径才跑得起来）。
    let rel = run_in_container(sid, &spec, "cmd2.exe /c echo REL > rel.txt").expect("起容器");
    let abs = run_in_container(
        sid,
        &spec,
        &format!(
            "\"{}\" /c echo ABS > abs.txt",
            d_leaf.join("cmd2.exe").display()
        ),
    )
    .expect("起容器");
    assert!(
        rel == 0 && c_leaf.join("rel.txt").is_file(),
        "相对名要能跑（exit={}）",
        rel
    );
    assert!(
        abs == 0 && c_leaf.join("abs.txt").is_file(),
        "绝对路径要能跑（执行前应改写成相对 cwd 的路径；exit={}）",
        abs
    );
    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);
}

/// 目的：容器临时目录按 AppContainer 的 `GetTempPath2` 布局给——`<私有沙箱>/Packages/<容器名>/AC/Temp`。
/// 约束：私有沙箱为空时退回工作目录；两者都空就没有落点（返回 `None`，不猜）。
#[test]
fn container_temp_dir_follows_gettemp2_layout() {
    let mut spec = FenceSpec {
        agent: "probe-temp".to_string(),
        lease: String::new(),
        rw: Vec::new(),
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: PathBuf::from("sandbox"),
        cwd: PathBuf::from("module"),
        net: false,
    };
    let dir = container_temp_dir(&spec).expect("私有沙箱在场");
    assert_eq!(
        dir,
        PathBuf::from("sandbox")
            .join("Packages")
            .join(container_name(&spec))
            .join("AC")
            .join("Temp")
    );
    spec.private = PathBuf::new();
    assert!(
        container_temp_dir(&spec)
            .expect("退回工作目录")
            .starts_with("module"),
        "私有沙箱为空时要退回工作目录"
    );
    spec.cwd = PathBuf::new();
    assert!(container_temp_dir(&spec).is_none(), "没有落点就不猜");
}

/// 【真机】容器里的系统临时目录由 `prepare_fence` 建好且可写：编译器能在里面落临时文件。
///   不建它的现场：g++ 报 `Cannot create temporary file in …\AC\Temp\` 并直接 abort（0xC0000409）。
/// 会创建 AppContainer profile 与写目录 ACL，只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑；没有项目内 MinGW 时如实跳过。
#[test]
fn container_has_a_writable_temp_dir_for_compilers() {
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_has_a_writable_temp_dir_for_compilers 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：容器临时目录探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let mingw = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".tools")
        .join("mingw64");
    let gpp = mingw.join("bin").join("g++.exe");
    if !gpp.is_file() {
        eprintln!("[探针] 本机没有项目内 MinGW：容器临时目录探针跳过（不静默当作通过）");
        return;
    }
    // 夹具放在仓库卷上（与项目内编译器同卷）：跨卷时程序名无法相对化，而这条用例要验的是"编译器能跑"。
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("temp-probe-{}", std::process::id()));
    let sandbox = base.join("sandbox");
    let leaf = base.join("leaf");
    std::fs::create_dir_all(&sandbox).expect("建私有沙箱");
    std::fs::create_dir_all(&leaf).expect("建工作目录");
    std::fs::write(leaf.join("probe.cpp"), "int probe() { return 0; }\n").expect("写编译单元");
    let spec = FenceSpec {
        agent: "probe-temp".to_string(),
        lease: String::new(),
        rw: vec![sandbox.clone(), leaf.clone()],
        ro: Vec::new(),
        ro_tree: vec![mingw.clone()],
        private: sandbox.clone(),
        cwd: leaf.clone(),
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
    let temp = container_temp_dir(&spec).expect("私有沙箱在场");
    assert!(
        temp.is_dir(),
        "prepare 要先建好容器临时目录：{}",
        temp.display()
    );
    let sid = container_sid(&container).expect("派生容器 SID");

    // ① 容器里的 TEMP 就是它（GetTempPath2 的落点），且可写（授权靠 rw 的递归 ACE，不单列一条）。
    let probe = temp.join("probe.txt");
    let seen = leaf.join("temp-path.txt");
    let code = run_in_container(
        sid,
        &spec,
        &format!(
            "echo %TEMP%> \"{}\" & echo ok > \"{}\"",
            seen.display(),
            probe.display()
        ),
    )
    .expect("容器进程应当能启动");
    assert!(
        code == 0 && probe.is_file(),
        "容器要能写临时目录（exit={}，路径 {}）",
        code,
        probe.display()
    );
    let got = std::fs::read_to_string(&seen).unwrap_or_default();
    assert_eq!(
        got.trim().to_lowercase(),
        temp.display().to_string().to_lowercase(),
        "容器里的 TEMP 应当就是 prepare 建的落点"
    );

    // ② 编译器能用它：g++ 的临时文件走 GetTempPath2，目录不可写会直接 abort。
    //    程序名是绝对路径——容器执行前会改写成相对 cwd（见 container_runs_a_program_given_by_absolute_path）。
    let log = leaf.join("gpp.log");
    let code = run_in_container(
        sid,
        &spec,
        &format!(
            "\"{}\" -std=c++17 -c probe.cpp -o out.o > \"{}\" 2>&1",
            gpp.display(),
            log.display()
        ),
    )
    .expect("容器进程应当能启动");
    let said = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        code == 0 && leaf.join("out.o").is_file(),
        "容器里的 g++ 要能编译（exit={}，输出 {:?}，临时目录 {}）",
        code,
        said.trim(),
        temp.display()
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
        lease: String::new(),
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
        lease: String::new(),
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
        lease: String::new(),
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
        lease: String::new(),
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
    // 独立临时目录名：另一个用例（第 280 行起）也以 solomni-grant-probe-<pid> 为根，
    // 并行跑会共享同一份台账，必须错开（否则台账条数被对方写脏）。
    let base = std::env::temp_dir().join(format!("solomni-grant-catalog-{}", std::process::id()));
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
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![inside.clone()],
        cwd: inside.clone(),
        ro: Vec::new(),
        net: false,
    };
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    let sid_text = sid_to_string(sid);
    let owner = current_owner();
    // 一条根内授权（走快照）+ 一条根外授权（走摘要）：都经“先落台账、再写 ACL、写后核对”那条路。
    let mut rec = load_record(&home);
    let inside_target = GrantTarget {
        path: inside.clone(),
        rights: RIGHTS_RW,
        recursive: false,
        inherit: false,
        part: FencePart::DataBoundary,
    };
    grant_one_journaled(
        &home,
        &mut rec,
        sid,
        &sid_text,
        &inside_target,
        &base,
        owner.clone(),
    )
    .expect("根内授权应当成功");
    let outside_target = GrantTarget {
        path: outside.clone(),
        rights: RIGHTS_RO,
        recursive: false,
        inherit: false,
        part: FencePart::Interpreter,
    };
    grant_one_journaled(
        &home,
        &mut rec,
        sid,
        &sid_text,
        &outside_target,
        &base,
        owner.clone(),
    )
    .expect("根外授权应当成功");
    free_sid(sid);
    // 清单：两条都看得见，都如实标成“现在还在”，且与盘上的 ACE 对得上。
    let view = catalog(&home);
    assert_eq!(
        view.entries.len(),
        3,
        "台账里应当有三条（根内快照 + 根内授权 + 根外授权）：{:?}",
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

/// 【启动对账·纯逻辑】归属集合的结论：有活归属则留、全死则回收、缺归属或判不了则跳过。
/// PID 复用（同一 pid、创建时刻对不上）必须判死——否则旧进程的残留永远回收不掉。
#[test]
fn owner_state_distinguishes_live_dead_and_unjudgeable() {
    let me = current_owner();
    if me.start == 0 {
        eprintln!("[探针] 本机读不到进程创建时刻：归属判定用例跳过（不静默当作通过）");
        return;
    }
    let dead = Owner {
        pid: 0x7fff_fff0,
        start: 1,
        lease: String::new(),
    };
    assert_eq!(
        owners_state(std::slice::from_ref(&me)),
        OwnersState::Keep,
        "有活归属要留"
    );
    assert_eq!(
        owners_state(std::slice::from_ref(&dead)),
        OwnersState::Reclaim,
        "全死可回收"
    );
    assert_eq!(
        owners_state(&[me.clone(), dead.clone()]),
        OwnersState::Keep,
        "一活一死不回收"
    );
    assert_eq!(owners_state(&[]), OwnersState::Unjudgeable, "缺归属要跳过");
    assert_eq!(
        owners_state(&[Owner {
            pid: me.pid,
            start: me.start.wrapping_add(1),
            lease: String::new()
        }]),
        OwnersState::Reclaim,
        "PID 被复用（创建时刻对不上）要判死"
    );
}

/// 【启动对账】归属明确已死的陈旧授权必须被回收（ACE 消失、台账销账）；活归属一个字都不动。
#[test]
fn reconcile_reclaims_dead_owners_and_keeps_live_ones() {
    let base = std::env::temp_dir().join(format!("solomni-reconcile-{}", std::process::id()));
    let live = base.join("live");
    let stale = base.join("stale");
    std::fs::create_dir_all(&live).expect("建活落点");
    std::fs::create_dir_all(&stale).expect("建陈旧落点");
    if !acl_round_trip(&base, "reconcile") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：启动对账探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let spec_live = FenceSpec {
        agent: "probe-reconcile-live".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![live.clone()],
        cwd: live.clone(),
        ro: Vec::new(),
        net: false,
    };
    let spec_stale = FenceSpec {
        agent: "probe-reconcile-stale".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![stale.clone()],
        cwd: stale.clone(),
        ro: Vec::new(),
        net: false,
    };
    expect_granted(&prepare_fence(&spec_live, "cmd", &home), "活授权应当成功");
    expect_granted(
        &prepare_fence(&spec_stale, "cmd", &home),
        "陈旧授权应当成功",
    );
    let stale_sid = container_sid(&container_name(&spec_stale)).expect("派生陈旧 SID");
    let stale_text = sid_to_string(stale_sid);
    free_sid(stale_sid);
    // 把陈旧那条的归属记为一个绝不存在的进程。
    let dead = Owner {
        pid: 0x7fff_fff0,
        start: 1,
        lease: "dead-session".to_string(),
    };
    {
        let mut rec = load_record(&home);
        for g in rec.grants.iter_mut() {
            if g.sid == stale_text {
                g.owners = vec![dead.clone()];
            }
        }
        save_record(&home, &rec).expect("写回台账");
    }
    let rep = reconcile(&home);
    assert!(rep.errors.is_empty(), "对账不该报错：{:?}", rep.errors);
    assert!(
        rep.reclaimed_grants >= 1 || rep.restored_snapshots >= 1,
        "陈旧授权至少要回收一条：{:?}",
        rep
    );
    let sid = container_sid(&container_name(&spec_stale)).expect("再派生陈旧 SID");
    assert!(
        !has_any_ace_for(sid, &stale),
        "陈旧授权回收后不该再有该容器 SID 的 ACE"
    );
    free_sid(sid);
    let sid = container_sid(&container_name(&spec_live)).expect("再派生活 SID");
    assert!(has_any_ace_for(sid, &live), "活归属的授权不许被回收");
    free_sid(sid);
    clean(&home).expect("收尾回收应当成功");
    discard(&base);
}

/// 【启动对账】台账外的按次 profile：只报告，不自动删（没有归属证明，删了会误伤在跑实例）。
#[test]
fn reconcile_reports_ledger_less_profiles_without_deleting_them() {
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：reconcile_reports_ledger_less_profiles_without_deleting_them 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    let base =
        std::env::temp_dir().join(format!("solomni-reconcile-orphan-{}", std::process::id()));
    let home = base.join(".home");
    std::fs::create_dir_all(&home).expect("建台账目录");
    let spec = FenceSpec {
        agent: "probe-reconcile-orphan".to_string(),
        lease: "call-orphan".to_string(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: Vec::new(),
        cwd: PathBuf::new(),
        ro: Vec::new(),
        net: false,
    };
    let name = container_name(&spec);
    // 只建 profile、不记台账（模拟台账被删 / 非台账路径留下的残留）。
    assert!(ensure_profile(&name).is_ok(), "建探针 profile");
    let rep = reconcile(&home);
    assert!(
        rep.orphan_profiles
            .iter()
            .any(|n| n.eq_ignore_ascii_case(&name)),
        "台账外的 profile 要列进报告：{:?}",
        rep.orphan_profiles
    );
    assert!(
        rep.summary().contains("台账外还有"),
        "摘要要说明现象与处置入口：{}",
        rep.summary()
    );
    assert!(profile_exists(&name), "只报告，不自动删");
    // 收尾：探针自己删，不留痕。
    assert!(delete_profile(&name), "探针 profile 应当删得掉");
    discard(&base);
}

/// 【启动对账】缺归属/判不了的条目：如实报告并跳过，一条都不回收。
#[test]
fn reconcile_skips_entries_without_ownership() {
    let base = std::env::temp_dir().join(format!("solomni-reconcile-skip-{}", std::process::id()));
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "reconcile-skip") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：对账跳过探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let spec = FenceSpec {
        agent: "probe-reconcile-skip".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    // 清空全部归属（模拟归属读不出来 / 台账丢失）：对账必须报告并跳过，不许回收。
    {
        let mut rec = load_record(&home);
        for g in rec.grants.iter_mut() {
            g.owners.clear();
        }
        for p in rec.profiles.iter_mut() {
            p.owners.clear();
        }
        save_record(&home, &rec).expect("写回台账");
    }
    let rep = reconcile(&home);
    assert!(rep.errors.is_empty(), "对账不该报错：{:?}", rep.errors);
    assert_eq!(rep.reclaimed_grants, 0, "缺归属不得回收");
    assert_eq!(rep.restored_snapshots, 0, "缺归属不得还原快照");
    assert!(
        rep.skipped_unjudgeable >= 1,
        "缺归属要如实报告跳过：{:?}",
        rep
    );
    let sid = container_sid(&container_name(&spec)).expect("派生容器 SID");
    assert!(has_any_ace_for(sid, &target), "无法判定时不许动 ACE");
    free_sid(sid);
    clean(&home).expect("收尾回收应当成功");
    discard(&base);
}

/// 【真机并发】同一 agent 的两个不同 scope 调用**重叠**时：各自独立容器身份，窄调用读不到宽调用临时加的根；
/// 释放一次不影响另一次；收尾不留孤儿 ACE / profile / 临时目录。
/// 约束：只在 --fence-live（SOLOMNI_FENCE_LIVE=1）且本机能做 ACL 往返时跑；会创建 AppContainer profile，
///   收尾按台账撤权、删 profile 与临时目录。
#[test]
fn concurrent_calls_of_one_agent_are_isolated_and_released_independently() {
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：concurrent_calls_of_one_agent_are_isolated_and_released_independently 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：并发隔离探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    use crate::kernel::ports::FenceHost;
    let base = std::env::temp_dir().join(format!("solomni-overlap-{}", std::process::id()));
    let wide = base.join("wide");
    let narrow = base.join("narrow");
    std::fs::create_dir_all(&wide).expect("建宽根");
    std::fs::create_dir_all(&narrow).expect("建窄根");
    std::fs::write(wide.join("secret.txt"), "WIDE-SECRET").expect("写宽根文件");
    std::fs::write(narrow.join("ok.txt"), "NARROW-OK").expect("写窄根文件");
    if !acl_round_trip(&base, "overlap") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：并发隔离探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let spec_of = |lease: &str, root: &Path, private: &Path| FenceSpec {
        agent: "probe-overlap".to_string(),
        lease: lease.to_string(),
        private: private.to_path_buf(),
        ro_tree: Vec::new(),
        rw: vec![root.to_path_buf()],
        cwd: root.to_path_buf(),
        ro: Vec::new(),
        net: false,
    };
    let wide_spec = spec_of("call-wide", &wide, &base.join("private-wide"));
    let narrow_spec = spec_of("call-narrow", &narrow, &base.join("private-narrow"));
    expect_granted(
        &prepare_fence(&wide_spec, "cmd", &home),
        "宽调用授权应当成功",
    );
    expect_granted(
        &prepare_fence(&narrow_spec, "cmd", &home),
        "窄调用授权应当成功",
    );
    let wide_name = container_name(&wide_spec);
    let narrow_name = container_name(&narrow_spec);
    assert!(ensure_profile(&wide_name).is_ok(), "建宽调用的容器 profile");
    assert!(
        ensure_profile(&narrow_name).is_ok(),
        "建窄调用的容器 profile"
    );
    let wide_sid = container_sid(&wide_name).expect("派生宽调用 SID");
    let narrow_sid = container_sid(&narrow_name).expect("派生窄调用 SID");
    assert_ne!(
        sid_to_string(wide_sid),
        sid_to_string(narrow_sid),
        "同一 agent 的两次调用必须是不同容器身份"
    );

    // 宽调用在容器里活着（约 5 秒），同时窄调用尝试读宽根。
    let wide_addr = wide_sid as usize;
    let wide_spec_bg = wide_spec.clone();
    let holding = std::thread::spawn(move || {
        // PSID 只在本次探针内有效；线程结束前 main 不会 free。
        run_in_container(
            wide_addr as PSID,
            &wide_spec_bg,
            "ping -n 6 127.0.0.1 > nul",
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(800));

    // 窄调用：自己的文件读得到。
    let own = run_in_container(narrow_sid, &narrow_spec, "type ok.txt > own.txt 2>&1")
        .expect("窄调用要能启动");
    let own_text = std::fs::read_to_string(narrow.join("own.txt")).unwrap_or_default();
    assert!(
        own == 0 && own_text.contains("NARROW-OK"),
        "窄调用自己的根要读得到（exit={}，拿到 {:?}）",
        own,
        own_text
    );
    // 窄调用：宽根的密文读不到（宽调用此刻还活着）。
    let leak_cmd = format!(
        "type \"{}\" > \"{}\" 2>&1",
        wide.join("secret.txt").display(),
        narrow.join("leak.txt").display()
    );
    let leak = run_in_container(narrow_sid, &narrow_spec, &leak_cmd).expect("窄调用要能启动");
    let leak_text = std::fs::read_to_string(narrow.join("leak.txt")).unwrap_or_default();
    assert!(
        leak != 0 && !leak_text.contains("WIDE-SECRET"),
        "窄调用不该读到宽调用的额外根（exit={}，拿到 {:?}）",
        leak,
        leak_text
    );

    let wide_exit = holding.join().expect("宽调用线程");
    assert!(
        wide_exit.is_ok(),
        "宽调用容器进程应当正常结束：{:?}",
        wide_exit
    );

    // 释放宽调用：窄调用的授权与自己的根不受影响。
    let host = crate::kernel::detail::confine::FenceHostAdapter::new(home.clone());
    host.release_call(&wide_spec).expect("释放宽调用应当成功");
    assert!(!has_any_ace_for(wide_sid, &wide), "宽调用的 ACE 要撤掉");
    assert!(
        has_ace_for(narrow_sid, &narrow, RIGHTS_RW),
        "窄调用还在用，它的 ACE 必须留着"
    );
    let still = run_in_container(narrow_sid, &narrow_spec, "type ok.txt > after.txt 2>&1")
        .expect("释放宽调用后窄调用仍要能跑");
    assert_eq!(still, 0, "释放宽调用不该影响窄调用");
    assert!(!profile_exists(&wide_name), "宽调用的 profile 要删掉");
    assert!(profile_exists(&narrow_name), "窄调用的 profile 要留着");

    // 释放窄调用：收尾不留孤儿。
    host.release_call(&narrow_spec).expect("释放窄调用应当成功");
    assert!(
        !has_any_ace_for(wide_sid, &wide) && !has_any_ace_for(narrow_sid, &narrow),
        "两个调用都释放后不该再有它们的 ACE"
    );
    assert!(!profile_exists(&narrow_name), "窄调用的 profile 要删掉");
    assert!(
        orphan_package_aces(&narrow).unwrap_or_default().is_empty()
            && orphan_package_aces(&wide).unwrap_or_default().is_empty(),
        "收尾后不该留台账外的包 SID ACE"
    );
    free_sid(wide_sid);
    free_sid(narrow_sid);
    let _ = clean(&home);
    discard(&base);
}

/// 【按次身份】同一 agent 的两次调用各有独立容器身份与独立授权：
/// 释放一次不影响另一次；收尾要把自己的 ACE、profile 与按次临时目录都带走，不留孤儿。
#[test]
fn release_call_only_removes_its_own_grant_and_profile() {
    use crate::kernel::ports::FenceHost;
    let base = std::env::temp_dir().join(format!("solomni-release-owner-{}", std::process::id()));
    let target = base.join("target");
    let private = base.join("private");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "release-owner") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：归属释放探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let spec_of = |lease: &str| FenceSpec {
        agent: "probe-per-call".to_string(),
        lease: lease.to_string(),
        private: private.clone(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let s1 = spec_of("call-1");
    let s2 = spec_of("call-2");
    // 同一 agent 的两次调用：各自授权、各自 profile。
    expect_granted(&prepare_fence(&s1, "cmd", &home), "调用 1 授权应当成功");
    expect_granted(&prepare_fence(&s2, "cmd", &home), "调用 2 授权应当成功");
    assert!(
        ensure_profile(&container_name(&s1)).is_ok()
            && ensure_profile(&container_name(&s2)).is_ok(),
        "探针要建出两个按次 profile"
    );
    let sid1 = container_sid(&container_name(&s1)).expect("派生调用 1 的 SID");
    let sid2 = container_sid(&container_name(&s2)).expect("派生调用 2 的 SID");
    assert_ne!(
        sid_to_string(sid1),
        sid_to_string(sid2),
        "同一 agent 的两次调用必须是不同容器身份"
    );
    assert!(
        has_ace_for(sid1, &target, RIGHTS_RW) && has_ace_for(sid2, &target, RIGHTS_RW),
        "两次调用各自的 ACE 都要在"
    );
    let host = crate::kernel::detail::confine::FenceHostAdapter::new(home.clone());
    // 释放调用 1：只带走它自己的 ACE / profile / 临时目录。
    host.release_call(&s1).expect("调用 1 的按次释放应当成功");
    assert!(!has_any_ace_for(sid1, &target), "调用 1 的 ACE 要撤掉");
    assert!(
        has_ace_for(sid2, &target, RIGHTS_RW),
        "调用 2 还在跑，它的 ACE 必须留着"
    );
    assert!(
        !profile_exists(&container_name(&s1)),
        "调用 1 的 profile 要删掉"
    );
    assert!(
        profile_exists(&container_name(&s2)),
        "调用 2 的 profile 要留着"
    );
    if let Some(dir) = container_package_dir(&s1) {
        assert!(
            !dir.exists(),
            "调用 1 的按次临时目录要删掉：{}",
            dir.display()
        );
    }
    // 释放调用 2：收尾不留孤儿。
    host.release_call(&s2).expect("调用 2 的按次释放应当成功");
    assert!(!has_any_ace_for(sid1, &target) && !has_any_ace_for(sid2, &target));
    assert!(
        !profile_exists(&container_name(&s2)),
        "调用 2 的 profile 要删掉"
    );
    assert!(
        orphan_package_aces(&target).unwrap_or_default().is_empty(),
        "按次收尾后不该留台账外的包 SID ACE"
    );
    free_sid(sid1);
    free_sid(sid2);
    let _ = clean(&home);
    discard(&base);
}

/// 目的：预检的“能不能写 DACL”用 AccessCheck 判，不只认属主（nvm 真身属主是 Administrators，
///   当前用户却有 WRITE_DAC）。
#[test]
fn can_write_dacl_accepts_a_directory_we_own() {
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
    let base = crate::tests::scratch("grant-failure-rollback");
    let dir = base.join("mod");
    let home = base.join(".home");
    std::fs::create_dir_all(&dir).expect("建目录");
    std::fs::create_dir_all(&home).expect("建台账目录");
    let sid = baseline_sid().expect("基线 SID");
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
    let mut rec = GrantRecord::default();
    let target = GrantTarget {
        path: dir.clone(),
        rights: RIGHTS_RO,
        recursive: false,
        inherit: false,
        part: FencePart::Interpreter,
    };
    let err = grant_one_journaled(
        &home,
        &mut rec,
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
