//! 目的：共享后端的落地判据与候选挑选。
//! 管：9p/smb/virtiofs/copy 的在场、候选顺序、guest 客户端要求。
//! 不管：引擎参数（`engine.rs`）。
//! 联动：`src/capabilities/vm/detail/share_ninep.rs`、`share_smb.rs`、`share_virtiofs.rs`、`share_none.rs`。

/// 目的：自定义 guest 没声明任何可用客户端时，起 guest 前就拒绝，不等到挂载失败。
#[test]
pub(crate) fn a_guest_without_supported_clients_is_rejected_before_boot() {
    use crate::capabilities::vm::api::tier_readiness;
    use crate::kernel::api::{ExecSpec as VmSpec, Tier};
    let root = crate::tests::scratch("vm-guest-share-readiness");
    let probe = crate::kernel::detail::HostProbeAdapter;
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let write_guest = |name: &str, clients: &str| {
        let dir = root.join(".vm").join("guest").join(name);
        std::fs::create_dir_all(&dir).expect("建 guest 目录");
        std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写磁盘镜像");
        std::fs::write(
            dir.join("guest.yaml"),
            format!(
                "id: {name}\narch: {arch}\nos: linux\nagent: qga\nclients: [{clients}]\nruntimes: [python]\n"
            ),
        )
        .expect("写 guest.yaml");
    };
    write_guest("good", "9p, virtiofs, cifs");
    write_guest("bad", "");
    // 给一个可执行的共享工具路径：Windows 按文件存在判，Unix 要执行位。
    let tool = root.join("share-tool");
    std::fs::write(&tool, b"stub").expect("写工具占位");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("加执行位");
    }
    let tool = tool.to_string_lossy().into_owned();
    let spec = |guest: &str| VmSpec {
        tier: Tier::Vm,
        guest: Some(guest.to_string()),
        ..Default::default()
    };
    let find = |guest: &str| {
        let readiness = tier_readiness(&root, &spec(guest), None, Some(tool.as_str()), &probe);
        readiness
            .requirements
            .into_iter()
            .find(|r| r.id == "share-guest")
            .expect("要有 guest 级共享判据")
    };
    let good = find("good");
    assert!(good.met, "声明了可用客户端的 guest 要能承载：{:?}", good);
    let bad = find("bad");
    assert!(!bad.met, "没声明客户端的 guest 必须拒绝：{:?}", bad);
    assert!(
        bad.detail.contains("未声明") && bad.how.contains("guest.yaml"),
        "拒绝要说清缺什么、怎么补：{:?}",
        bad
    );
    let _ = std::fs::remove_dir_all(&root);
}
