//! 目的：VM 共享边界的真机探针——只读挂载在 guest 里真的写不进，可写挂载的写入真的落回宿主。
//! 管：SMB（Windows 宿主）与 9p（非 Windows 宿主）两条只读边界的验收；宿主不提供的档位如实 env-skip。
//! 不管：共享后端怎么准备 / 清理（`src/capabilities/vm/detail/share_*.rs`）、guest 通道细节。
//! 联动：`src/tests/vm/live.rs`（真机总验收）与 `tests/windows/gaps.yaml`（Windows 上没有的 9p 路径）。

use crate::capabilities::vm::domain::guest::{GuestMount, GuestRun, GuestSpec, classify_base};
use crate::capabilities::vm::ports::{ShareBackend, VmEngine};
use crate::kernel::ports::HostProbe;

/// 目的：真机探针的公共前置：读 `SOLOMNI_VM_LIVE` / `SOLOMNI_VM_QEMU` / 虚拟根；缺失就如实打印并返回 None。
fn live_guest(what: &str) -> Option<(String, std::path::PathBuf)> {
    if std::env::var("SOLOMNI_VM_LIVE").as_deref() != Ok("1") {
        eprintln!(
            "[探针] 未开启虚拟机档真机验收（{}）：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑",
            what
        );
        return None;
    }
    let qemu = match std::env::var("SOLOMNI_VM_QEMU") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("[探针] 未设 SOLOMNI_VM_QEMU：{}边界探针跳过", what);
            return None;
        }
    };
    let guest = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian-13".to_string());
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base = match crate::capabilities::vm::api::resolve_guest(&crate::tests::root_dir(), &guest)
    {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!(
                "[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes，或用 SOLOMNI_VM_GUEST 指定",
                guest
            );
            return None;
        }
    };
    Some((qemu, base))
}

/// 目的：起一台真 guest，挂一个可写共享与一个只读共享，断言「只读写不进、可写落回宿主」。
fn probe_read_only_boundary(
    backend: &dyn ShareBackend,
    tool: Option<&str>,
    qemu: String,
    base_dir: std::path::PathBuf,
) {
    let _live = crate::tests::doubles::live_guest_lock();
    let probe = crate::kernel::detail::HostProbeAdapter;
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    let rw = crate::tests::scratch("vm-boundary-rw");
    let ro = crate::tests::scratch("vm-boundary-ro");
    std::fs::create_dir_all(&rw).expect("建可写共享");
    std::fs::create_dir_all(&ro).expect("建只读共享");
    std::fs::write(rw.join("rw.txt"), "RW_MARKER").expect("写可写标记");
    std::fs::write(ro.join("ro.txt"), "RO_MARKER").expect("写只读标记");
    let mounts = vec![
        GuestMount {
            host: rw.clone(),
            guest: "/boundary-rw".to_string(),
            read_only: false,
        },
        GuestMount {
            host: ro.clone(),
            guest: "/boundary-ro".to_string(),
            read_only: true,
        },
    ];
    let spec = GuestSpec {
        lease: "vm-share-boundary".to_string(),
        image,
        mounts: mounts.clone(),
        net: false,
        engine_path: Some(qemu),
        share_path: tool.map(str::to_string),
        shares: Vec::new(),
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    let overlay = crate::tests::scratch("vm-boundary-overlays");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay);
    // 共享守护进程冷启动或慢启动时，第一次挂载可能三次重试都撞上瞬时报错；重建共享后再试一次，
    // 仍失败才判探针失败（不是把失败吞成通过）。
    let (mut guest, guard) = {
        let mut attempt = 0;
        loop {
            let (plan, guard) = backend
                .prepare("qemu", &mounts, tool)
                .expect("共享计划要能派生");
            match engine.boot(&spec, &plan) {
                Ok(g) => break (g, guard),
                Err(e) if attempt == 0 => {
                    attempt += 1;
                    eprintln!("[诊断] 第一次起 guest 失败，重建共享后重试：{}", e);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                Err(e) => panic!("起 guest：{}", e),
            }
        }
    };
    let run = |command: &str| GuestRun {
        command: command.to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    let read = guest
        .channel()
        .exec(&run("cat /boundary-ro/ro.txt; cat /boundary-rw/rw.txt"));
    assert!(
        read.ok && read.output.contains("RO_MARKER") && read.output.contains("RW_MARKER"),
        "两条共享都要在 guest 里读得到：{}",
        read.output
    );
    let ro_write = guest.channel().exec(&run(
        "if echo X > /boundary-ro/new.txt 2>/dev/null; then echo RO_WRITE_WENT_THROUGH; else echo RO_WRITE_DENIED; fi",
    ));
    assert!(
        ro_write.ok && ro_write.output.contains("RO_WRITE_DENIED"),
        "只读共享在 guest 里必须写不进：{}",
        ro_write.output
    );
    assert!(
        !ro.join("new.txt").exists(),
        "只读共享的宿主目录不得出现 guest 写下的文件"
    );
    let rw_write = guest
        .channel()
        .exec(&run("echo RW_OK > /boundary-rw/new.txt && echo RW_WRITTEN"));
    assert!(
        rw_write.ok && rw_write.output.contains("RW_WRITTEN"),
        "可写共享要能写入：{}",
        rw_write.output
    );
    assert_eq!(
        std::fs::read(rw.join("new.txt")).unwrap_or_default(),
        b"RW_OK\n",
        "可写共享的写入要实时落回宿主"
    );
    drop(guard);
}

/// 目的：Windows 宿主的 SMB 只读边界真跑——只读共享写不进、可写共享落回宿主。
#[test]
pub(crate) fn smb_read_only_share_rejects_guest_writes() {
    if std::env::consts::OS != "windows" {
        eprintln!("[探针] SMB 共享只在 Windows 宿主上提供：SMB 边界探针跳过");
        return;
    }
    let smbd = match std::env::var("SOLOMNI_VM_SMBD") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("[探针] 未设 SOLOMNI_VM_SMBD：SMB 边界探针跳过（不静默当作通过）");
            return;
        }
    };
    let Some((qemu, base)) = live_guest("SMB ") else {
        return;
    };
    let image =
        classify_base(&base, &crate::kernel::detail::HostProbeAdapter).expect("虚拟根要可用");
    if !image.clients.cifs {
        eprintln!("[探针] 虚拟根未声明 cifs 客户端：SMB 边界探针跳过");
        return;
    }
    probe_read_only_boundary(
        &crate::capabilities::vm::detail::SmbShare::new(),
        Some(&smbd),
        qemu,
        base,
    );
}

/// 目的：非 Windows 宿主的 9p 只读边界真跑；Windows 宿主没有这条生产路径，如实 env-skip 并登记缺口。
#[test]
pub(crate) fn ninep_read_only_share_rejects_guest_writes() {
    if std::env::consts::OS == "windows" {
        eprintln!(
            "[探针] Windows 宿主的生产共享档是 SMB，不提供 9p：9p 边界探针 env-skip（缺口见 tests/windows/gaps.yaml）"
        );
        return;
    }
    let Some((qemu, base)) = live_guest("9p ") else {
        return;
    };
    let image =
        classify_base(&base, &crate::kernel::detail::HostProbeAdapter).expect("虚拟根要可用");
    if !image.clients.nine_p {
        eprintln!("[探针] 虚拟根未声明 9p 客户端：9p 边界探针跳过");
        return;
    }
    probe_read_only_boundary(
        &crate::capabilities::vm::detail::NinePShare::new(),
        None,
        qemu,
        base,
    );
}
