//! 目的：Windows 围栏的真机探针——AppContainer 授权、容器往返、ACL 台账与撤权。
//! 管：需要改本机权限项（建 profile、写 ACL）的用例，--fence-live 才跑。
//! 不管：非 Windows 平台的行为（见 tests/linux、tests/macos）。
//! 联动：机制在 confine/windows/ 各文件；判据见 docs/kernel/README.md 的「围栏落点：必要与可选」。

use super::*;

/// 目的：授权应当成立——必要落点全授上，不成立就把结论打出来（它是这一路的判据）。
pub(super) fn expect_granted(prep: &FencePrep, what: &str) {
    assert!(prep.ok(), "{}：必要落点授不上 {:?}", what, prep.blocked);
}

#[cfg(test)]
mod container;
#[cfg(test)]
mod grant;
#[cfg(test)]
mod workspace;

/// 目的：本产品能力 SID 的**可释放**形式——用例沿用 free_sid 收尾，不必各自持有派生出的字节缓冲。
pub(super) fn product_sid() -> Result<PSID, String> {
    let owned = product_capability_sid()?;
    sid_from_string(&sid_to_string(owned.as_ptr() as PSID))
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

/// 目的：探针目录的收尾——删不掉就如实打印，不静默（受限环境里写坏的 DACL 会让删除失败）。
pub(super) fn discard(base: &Path) {
    if let Err(e) = std::fs::remove_dir_all(base)
        && base.exists()
    {
        eprintln!("[诊断] 探针目录未清理（{}）：{}", base.display(), e);
    }
}

/// 目的：ACL 往返预检——在自有 base 里记 ACE 集合、写、读回、撤后集合不变；做不了就返回 false。
pub(super) fn acl_round_trip(base: &Path, tag: &str) -> bool {
    let target = base.join("acl-preflight");
    if std::fs::create_dir_all(&target).is_err() {
        return false;
    }
    let before = match acl_entries(&target) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
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
