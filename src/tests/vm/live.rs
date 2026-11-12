//! 目的：真机验收（默认跳过）。
//! 管：真 guest 起停、并行叠写、取消、诊断、外部工作区、UNC 往返。
//! 不管：全内存用例。
//! 联动：`SOLOMNI_VM_LIVE` 与 `docs/testing/levels.md`。

use crate::capabilities::vm::domain::guest::{GuestMount, GuestRun, GuestSpec, classify_base};
use crate::capabilities::vm::ports::{ShareBackend, VmEngine};
use crate::kernel::ports::HostProbe;

/// 目的：真机验收（默认跳过）：用项目内 QEMU 与虚拟根真起一个 guest，在它里面跑命令。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU 指向真东西、虚拟根（默认 `debian`）在场时执行；否则打印 [探针] 如实跳过。
#[test]
pub(crate) fn vm_live_guest_runs_command_inside_guest() {
    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    // 虚拟根名（默认 debian）；在场与否由 resolve_guest 解析出的那一格判。
    let guest = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian-13".to_string());
    if !live || qemu.trim().is_empty() {
        eprintln!(
            "[探针] 未开启虚拟机档真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑（虚拟根默认 debian-13，可用 SOLOMNI_VM_GUEST 换）"
        );
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir = match crate::capabilities::vm::api::resolve_guest(
        &crate::tests::root_dir(),
        &guest,
    ) {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!(
                "[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes，或用 SOLOMNI_VM_GUEST 指定",
                guest
            );
            return;
        }
    };
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    // 真机引擎级验收：共享要真进 guest（读得到标记），guest 的写要即时落到宿主（实时共享），收尾不留守护进程。
    let share = crate::tests::scratch("vm-live-share");
    std::fs::create_dir_all(&share).expect("建共享目录");
    std::fs::write(share.join("marker.txt"), "SHARE_MARKER").expect("写标记");
    // 第二个只读共享：生产会话会同时挂工作树与模块树，单挂载通过不代表多条 cifs 连接都稳。
    let ro_share = crate::tests::scratch("vm-live-share-ro");
    std::fs::create_dir_all(&ro_share).expect("建只读共享目录");
    std::fs::write(ro_share.join("ro.txt"), "RO_MARKER").expect("写只读标记");
    std::fs::create_dir_all(share.join("a")).expect("建 A 沙箱");
    std::fs::write(share.join("a").join("ok.txt"), "A_OK").expect("写 A 文件");
    std::fs::create_dir_all(share.join("b")).expect("建 B 沙箱");
    std::fs::write(share.join("b").join("secret.txt"), "B_SECRET").expect("写 B 文件");
    let guest_dir = "/solomni-vm-live-share";
    let ro_guest_dir = "/solomni-vm-live-ro";
    // 会话走**每租约可写层**（与产品同一条路）：基础镜像只读，可写层落在临时目录、随 guest 一起删。
    let overlay_dir = crate::tests::scratch("vm-live-overlays");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay_dir.clone());
    let virtiofsd = std::env::var("SOLOMNI_VM_VIRTIOFSD")
        .ok()
        .filter(|p| !p.trim().is_empty());
    let smbd = std::env::var("SOLOMNI_VM_SMBD")
        .ok()
        .filter(|p| !p.trim().is_empty());
    let spec = GuestSpec {
        lease: "vm-live".to_string(),
        image,
        mounts: vec![
            GuestMount {
                host: share.clone(),
                guest: guest_dir.to_string(),
                read_only: false,
            },
            GuestMount {
                host: ro_share.clone(),
                guest: ro_guest_dir.to_string(),
                read_only: true,
            },
        ],
        net: false,
        engine_path: Some(qemu),
        share_path: virtiofsd.clone(),
        shares: Vec::new(),
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    // 共享后端按平台用户链选（与组合根同一份判据）：smb(Windows) > virtiofs > 9p。
    // 没有可用的用户档就如实跳过探针——copy 是开发/应急通道，不在这里兜底。
    let nine = crate::capabilities::vm::detail::NinePShare::new();
    let virtiofs = crate::capabilities::vm::detail::VirtioFsShare::new();
    let smb = crate::capabilities::vm::detail::SmbShare::new();
    let use_smb = std::env::consts::OS == "windows" && spec.image.clients.cifs && smbd.is_some();
    let use_virtiofs = !use_smb && virtiofsd.is_some() && spec.image.clients.virtio_fs;
    let use_nine =
        !use_smb && !use_virtiofs && std::env::consts::OS != "windows" && spec.image.clients.nine_p;
    if !use_smb && !use_virtiofs && !use_nine {
        eprintln!(
            "[探针] 没有可用的用户共享档（Windows 需要 smbd；Linux/macOS 需要 9p 或 virtiofsd）：跳过真机验收"
        );
        return;
    }
    let backend: &dyn crate::capabilities::vm::ports::ShareBackend = if use_smb {
        &smb
    } else if use_virtiofs {
        &virtiofs
    } else {
        &nine
    };
    let tool = if use_smb {
        smbd.as_deref()
    } else {
        virtiofsd.as_deref()
    };
    let (plan, mut guard) = backend
        .prepare("qemu", &spec.mounts, tool)
        .expect("共享计划要能派生");
    let mut guest = engine.boot(&spec, &plan).expect("起 guest");
    // 根永远是磁盘镜像：每租约都该建出一份可写层。
    {
        let overlay = overlay_dir.join("vm-live.qcow2");
        assert!(
            overlay.is_file(),
            "每租约可写层要建出来：{}",
            overlay.display()
        );
    }
    let run = GuestRun {
        command: format!(
            "cat /etc/os-release; test -e '{}' && echo HOST_VISIBLE || echo HOST_HIDDEN; cat {}/marker.txt; cat {}/ro.txt; echo VM_LIVE_OK > {}/from-guest.txt; echo VM_LIVE_OK_WRITTEN",
            std::env::current_dir().expect("cwd").display(),
            guest_dir,
            ro_guest_dir,
            guest_dir
        ),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    let out = guest.channel().exec(&run);
    // 执行物本地化：共享盘不承载执行位——把脚本落进 guest 本地副本，再**直接执行**它（不是 `sh 它`）。
    let script = share.join("deploy.sh");
    std::fs::write(&script, "#!/bin/sh\necho DEPLOY_OK\n").expect("写共享区脚本");
    let dest = crate::capabilities::vm::domain::guest::staged_entry_path("probe", "deploy.sh");
    let staged = guest.channel().exec(&GuestRun {
        command: crate::capabilities::vm::domain::guest::stage_script(
            &format!("{guest_dir}/deploy.sh"),
            &dest,
        ),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    });
    assert!(staged.ok, "把入口落进 guest 本地失败：{}", staged.output);
    let ran = guest.channel().exec(&GuestRun {
        command: dest,
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    });
    assert!(
        ran.ok && ran.output.contains("DEPLOY_OK"),
        "本地副本要能直接执行：{}",
        ran.output
    );
    // per-agent 围栏：A 只拿到自己的沙箱，B 的沙箱必须被 guest 内 helper 拒绝。
    let access = crate::capabilities::vm::domain::guest::GuestAccess {
        agent: "a".to_string(),
        rw: vec![format!("{guest_dir}/a")],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: format!("{guest_dir}/a"),
        cwd: format!("{guest_dir}/a"),
        net: true,
        diagnose: false,
    };
    let fenced = guest.channel().exec_fenced(
        &GuestRun {
            command: format!(
                "cat {guest_dir}/a/ok.txt; echo '---'; cat {guest_dir}/b/secret.txt",
                guest_dir = guest_dir
            ),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        },
        &access,
    );
    assert!(
        !fenced.ok,
        "读 B 的沙箱必须被拒绝（能看到就说明围栏没生效）：{}",
        fenced.output
    );
    assert!(fenced.output.contains("A_OK"), "{}", fenced.output);
    assert!(
        fenced.output.contains("Permission denied") || fenced.output.contains("权限"),
        "B 的沙箱要被 Landlock 拒绝：{}",
        fenced.output
    );

    // 取消：sleep 30 起跑后由 host 置位取消旗，helper 收 SIGTERM 杀子进程组，不能等满 timeout。
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flipper = std::sync::Arc::clone(&cancel);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        flipper.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let started = std::time::Instant::now();
    let cancelled = guest.channel().exec_fenced_cancellable(
        &GuestRun {
            command: "sleep 30; echo SHOULD_NOT_FINISH".to_string(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        },
        &access,
        Some(&cancel),
    );
    let _ = thread.join();
    assert!(!cancelled.ok, "取消后命令不能报成功：{}", cancelled.output);
    assert!(
        cancelled.output.contains("用户停止"),
        "回执要说明用户停止：{}",
        cancelled.output
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "取消要及时，不能等满 30 秒：{:?}",
        started.elapsed()
    );

    // 实时共享：挂载式后端**不等任何回拷**，宿主此刻就该看得到 guest 写的内容。
    let written = std::fs::read_to_string(share.join("from-guest.txt")).unwrap_or_default();
    assert!(
        written.contains("VM_LIVE_OK"),
        "{} 要实时可见：宿主读到 {:?}（guest 输出：{}）",
        backend.id(),
        written,
        out.output
    );
    guest.stop();
    // 同上：删租约要把那份可写层也删掉。
    {
        let overlay = overlay_dir.join("vm-live.qcow2");
        assert!(
            !overlay.is_file(),
            "删租约要把可写层删掉：{}",
            overlay.display()
        );
    }
    // 删租约 = 守护进程与套接字都不留（与产品 service 释放的是同一份凭据）。
    guard.release();
    if use_virtiofs {
        let mut alive: Vec<String> = Vec::new();
        for _ in 0..12 {
            alive = std::process::Command::new("pgrep")
                .args(["-af", "virtiofsd"])
                .output()
                .map(|pg| {
                    String::from_utf8_lossy(&pg.stdout)
                        .lines()
                        .filter(|l| l.contains("--socket-path") && l.contains("solomni-virtiofs-"))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if alive.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        assert!(alive.is_empty(), "释放后不该残留 virtiofsd：{:?}", alive);
    }
    assert!(out.ok, "guest 里执行要成功：{}", out.output);
    assert!(out.output.contains("VM_LIVE_OK"), "输出：{}", out.output);
    assert!(
        out.output.contains("VM_LIVE_OK_WRITTEN"),
        "guest 要能往共享目录写：{}",
        out.output
    );
    assert!(
        out.output.contains("Debian GNU/Linux"),
        "要在 guest 的 os-release 里证明真跑在 guest 内：{}",
        out.output
    );
    assert!(
        out.output.contains("HOST_HIDDEN"),
        "宿主的项目根不该在 guest 里可达：{}",
        out.output
    );
    assert!(
        out.output.contains("SHARE_MARKER"),
        "额外共享要真进 guest：{}",
        out.output
    );
    assert!(
        out.output.contains("RO_MARKER"),
        "第二个只读共享要真进 guest：{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&share);
    let _ = std::fs::remove_dir_all(&ro_share);
}

/// 目的：真机验收（默认跳过）：同一基础镜像**并行起两台** guest，各自叠一份可写层，基础镜像谁都不写。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU 指向真东西、虚拟根（默认 `debian`）在场时执行；
///   同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
#[test]
pub(crate) fn vm_live_two_guests_share_one_base_without_writing_it() {
    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    let guest = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian-13".to_string());
    if !live || qemu.trim().is_empty() {
        eprintln!(
            "[探针] 未开启并发可写层的真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑"
        );
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir =
        match crate::capabilities::vm::api::resolve_guest(&crate::tests::root_dir(), &guest) {
            Ok(d) if probe.is_dir(&d) => d,
            _ => {
                eprintln!(
                    "[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes",
                    guest
                );
                return;
            }
        };
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    let base_mtime = std::fs::metadata(&image.disk)
        .and_then(|m| m.modified())
        .expect("读基础镜像 mtime");
    let overlay_dir = crate::tests::scratch("vm-live-overlays-two");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay_dir.clone());
    let backend = crate::capabilities::vm::detail::NoneShare::new();
    let (plan, _guard) = backend
        .prepare("qemu", &[], None)
        .expect("共享计划要能派生");
    let spec = |lease: &str| GuestSpec {
        lease: lease.to_string(),
        image: image.clone(),
        mounts: Vec::new(),
        net: false,
        engine_path: Some(qemu.clone()),
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    let mut a = engine
        .boot(&spec("vm-live-a"), &plan)
        .expect("起第一台 guest");
    let mut b = engine
        .boot(&spec("vm-live-b"), &plan)
        .expect("起第二台 guest");
    // 两台各自一份可写层：谁也不写基础镜像。
    for lease in ["vm-live-a", "vm-live-b"] {
        let ov = overlay_dir.join(format!("{lease}.qcow2"));
        assert!(ov.is_file(), "每租约可写层要在场：{}", ov.display());
    }
    let run = |tag: &str| GuestRun {
        command: format!("echo {tag}"),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    let out_a = a.channel().exec(&run("TWO_A"));
    let out_b = b.channel().exec(&run("TWO_B"));
    assert!(
        out_a.ok && out_a.output.contains("TWO_A"),
        "第一台要能跑：{}",
        out_a.output
    );
    assert!(
        out_b.ok && out_b.output.contains("TWO_B"),
        "第二台要能跑：{}",
        out_b.output
    );
    a.stop();
    b.stop();
    for lease in ["vm-live-a", "vm-live-b"] {
        let ov = overlay_dir.join(format!("{lease}.qcow2"));
        assert!(!ov.is_file(), "删租约要把可写层删掉：{}", ov.display());
    }
    let after = std::fs::metadata(&image.disk)
        .and_then(|m| m.modified())
        .expect("读基础镜像 mtime");
    assert_eq!(base_mtime, after, "两台 guest 跑完基础镜像 mtime 不该变");
    let _ = std::fs::remove_dir_all(&overlay_dir);
}

/// 目的：真机验收（默认跳过）：**不依赖任何共享后端**，只验 guest 内的取消与超时。
/// 约束：原来唯一覆盖取消的 `vm_live_guest_runs_command_inside_guest` 把取消和 SMB 共享绑在一起，
///   共享挂载失败时连取消脚本都跑不到；这条只走 NoneShare + guest 内 /tmp 围栏，
///   取消机制本身不该被共享链路的故障挡住。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU 指向真东西、虚拟根（默认 `debian`）在场时执行；否则打印 [探针] 如实跳过。
/// 约束：取消回执含「用户停止」且 15 秒内返回、guest 内没有残留的 sleep；`timeout_secs` 到点仍照常收口。
///   取消路径上 helper 自己的 stderr 可能被 QGA 丢掉（时有时无），所以"信号处理器生效"用**无孤儿 + 快速返回**证明；
///   超时路径由通道产出与本机同形的状态行 `[shell] 超时（N 秒）`（helper 只留机器标记，通道剥掉）。
///   本用例直接调 guest 通道：只有状态行，没有工具层追加的 cwd·耗时尾行。
#[test]
pub(crate) fn vm_live_cancel_kills_the_guest_process_tree() {
    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    let guest = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian-13".to_string());
    if !live || qemu.trim().is_empty() {
        eprintln!(
            "[探针] 未开启取消的真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑（虚拟根默认 debian-13，可用 SOLOMNI_VM_GUEST 换）"
        );
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir = match crate::capabilities::vm::api::resolve_guest(
        &crate::tests::root_dir(),
        &guest,
    ) {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!(
                "[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes，或用 SOLOMNI_VM_GUEST 指定",
                guest
            );
            return;
        }
    };
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    let overlay_dir = crate::tests::scratch("vm-live-cancel-overlays");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay_dir.clone());
    // 不挂任何共享：这条用例只验取消 / 超时，共享链路的问题归 SMB 挂载那条缺口。
    use crate::capabilities::vm::ports::ShareBackend;
    let backend = crate::capabilities::vm::detail::NoneShare::new();
    let (plan, _guard) = backend
        .prepare("qemu", &[], None)
        .expect("共享计划要能派生");
    let spec = GuestSpec {
        lease: "vm-live-cancel".to_string(),
        image,
        mounts: Vec::new(),
        net: false,
        engine_path: Some(qemu),
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    let mut guest = engine.boot(&spec, &plan).expect("起 guest");
    // guest 内的允许集：/tmp 读写，其它一切按 Landlock 拒绝。
    let access = crate::capabilities::vm::domain::guest::GuestAccess {
        agent: "cancel".to_string(),
        rw: vec!["/tmp".to_string()],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: "/tmp".to_string(),
        cwd: "/tmp".to_string(),
        net: false,
        diagnose: false,
    };
    let run = |cmd: &str, secs: u64| GuestRun {
        command: cmd.to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: "/tmp".to_string(),
        timeout_secs: secs,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Shell,
    };

    // 标准重定向：围栏里 /dev/null 必须可写（`2>/dev/null` 是常见脚本习语，只读基线会让它 EACCES）。
    let devnull = guest.channel().exec_fenced(
        &run("echo dropped >/dev/null && echo DEVNULL-OK", 10),
        &access,
    );
    assert!(devnull.ok, "围栏里要能写 /dev/null：{}", devnull.output);
    assert!(
        devnull.output.contains("DEVNULL-OK"),
        "重定向后命令要继续跑：{}",
        devnull.output
    );

    // 取消：sleep 30 起跑后由 host 置位取消旗，helper 收 SIGTERM 杀子进程组，不能等满 timeout。
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flip = std::sync::Arc::clone(&cancel);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        flip.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let started = std::time::Instant::now();
    let cancelled = guest.channel().exec_fenced_cancellable(
        &run("sleep 30; echo SHOULD_NOT_FINISH", 30),
        &access,
        Some(&cancel),
    );
    let _ = thread.join();
    let elapsed = started.elapsed();
    eprintln!(
        "[探针] 取消：ok={} 耗时={:?} 输出={:?}",
        cancelled.ok, elapsed, cancelled.output
    );
    assert!(!cancelled.ok, "取消后命令不能报成功：{}", cancelled.output);
    assert!(
        cancelled.output.contains("用户停止"),
        "回执要说明用户停止：{}",
        cancelled.output
    );
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "取消要及时，不能等满 30 秒：{:?}",
        elapsed
    );
    // 无孤儿：helper 的信号处理器杀掉的是**整棵进程组**；只杀 helper 自己会留下这个 sleep。
    // 这条探针不是 shell 回执，用 Head（别让通道补 [shell] 收尾行）。
    let alive = guest.channel().exec(&GuestRun {
        command: "pgrep -x sleep || true".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: "/tmp".to_string(),
        timeout_secs: 10,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    });
    eprintln!("[探针] 取消后残留 sleep：{:?}", alive.output);
    assert!(
        alive.output.trim().is_empty(),
        "取消后不该有残留的 guest 进程：{:?}",
        alive.output
    );

    // 超时：没有取消旗时，timeout_secs 到点由 helper 杀树，通道产出与本机同形的状态行。
    let started = std::time::Instant::now();
    let timed_out = guest
        .channel()
        .exec_fenced_cancellable(&run("sleep 30", 2), &access, None);
    let elapsed = started.elapsed();
    eprintln!(
        "[探针] 超时：ok={} 耗时={:?} 输出={:?}",
        timed_out.ok, elapsed, timed_out.output
    );
    assert!(!timed_out.ok, "超时后命令不能报成功：{}", timed_out.output);
    assert!(
        timed_out.output.contains("[shell] 超时（2 秒）"),
        "回执要给出同形超时状态行：{}",
        timed_out.output
    );
    assert!(
        !timed_out.output.contains("solomni-guest-fence:"),
        "helper 的机器标记不能漏进回执：{}",
        timed_out.output
    );
    assert!(
        elapsed < std::time::Duration::from_secs(12),
        "两秒的超时要很快收口：{:?}",
        elapsed
    );

    guest.stop();
    assert!(
        !overlay_dir.join("vm-live-cancel.qcow2").is_file(),
        "删租约要把可写层也删掉：{}",
        overlay_dir.join("vm-live-cancel.qcow2").display()
    );
    let _ = std::fs::remove_dir_all(&overlay_dir);
}

/// 目的：真机验收（默认跳过）：guest 里起一个忙进程，诊断要能把它列出来（进程表 + CPU / 内存 / 负载 / 挂载）。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU 指向真东西、虚拟根（默认 `debian`）在场时执行；否则打印 [探针] 如实跳过。
#[test]
pub(crate) fn vm_live_guest_diagnose_lists_processes() {
    use crate::capabilities::vm::api::Vm as _;
    use crate::capabilities::vm::detail::{NoneShare, QemuEngine};
    use crate::capabilities::vm::service::VmService;
    use std::sync::Arc;

    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    let guest = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian-13".to_string());
    if !live || qemu.trim().is_empty() {
        eprintln!(
            "[探针] 未开启 guest 诊断的真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑（虚拟根默认 debian-13，可用 SOLOMNI_VM_GUEST 换）"
        );
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir = match crate::capabilities::vm::api::resolve_guest(
        &crate::tests::root_dir(),
        &guest,
    ) {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!(
                "[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes，或用 SOLOMNI_VM_GUEST 指定",
                guest
            );
            return;
        }
    };
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    let overlay_dir = crate::tests::scratch("vm-live-diagnose-overlays");
    // 与产品同一条路：VmService 持引擎与共享档；这里用内部 none 档（诊断只要 QGA，不需要共享）。
    let vm = VmService::new(
        Arc::new(QemuEngine::with_overlay_dir(overlay_dir)),
        vec![Arc::new(NoneShare::new())],
    );
    let spec = GuestSpec {
        lease: "vm-live-diagnose".to_string(),
        image,
        mounts: Vec::new(),
        net: false,
        engine_path: Some(qemu),
        share_path: None,
        shares: vec!["none"],
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    vm.guest_start(&spec).expect("起 guest");
    // 忙进程：占满一个核的无限循环（后台跑，把 pid 打出来）。
    let busy = vm.guest_run(
        "vm-live-diagnose",
        &GuestRun {
            command: "while :; do :; done >/dev/null 2>&1 & echo $!".to_string(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        },
    );
    assert!(busy.ok, "guest 里要能起忙进程：{}", busy.output);
    let busy_pid: u32 = busy
        .output
        .split_whitespace()
        .last()
        .and_then(|s| s.parse().ok())
        .expect("忙进程要给 pid");
    // 采样窗口是两轮 /proc 快照（约 1 秒）：先让它跑起来再诊断。
    std::thread::sleep(std::time::Duration::from_millis(1200));
    let report = vm
        .guest_diagnose("vm-live-diagnose")
        .expect("guest 诊断要成功");
    assert!(!report.processes.is_empty(), "进程表不能是空的");
    assert!(
        report.process_total >= report.processes.len(),
        "进程总数不能小于列出的条数"
    );
    assert!(
        report.processes.iter().any(|p| p.pid == busy_pid),
        "忙进程 {} 要在进程表里：{:?}",
        busy_pid,
        report.processes.iter().map(|p| p.pid).collect::<Vec<_>>()
    );
    let busy_row = report
        .processes
        .iter()
        .find(|p| p.pid == busy_pid)
        .expect("忙进程那一行");
    assert!(
        busy_row.cpu_percent > 5.0,
        "忙进程的 CPU 占用要看得见：{}",
        busy_row.cpu_percent
    );
    assert!(
        report.memory.total_kb > 0 && report.memory.used_percent > 0.0,
        "内存摘要要有值：{:?}",
        report.memory
    );
    assert!(
        report.cpu.user + report.cpu.system + report.cpu.idle > 0,
        "CPU 汇总要有值：{:?}",
        report.cpu
    );
    assert!(!report.mounts.is_empty(), "挂载摘要不能是空的");
    assert!(
        report.guest.nproc > 0 && !report.guest.kernel.is_empty(),
        "guest 事实要有值：{:?}",
        report.guest
    );
    // 停掉 guest：诊断要如实说"没有活着的 guest"，不是"列表为空"。
    vm.guest_stop("vm-live-diagnose");
    let gone = vm
        .guest_diagnose("vm-live-diagnose")
        .expect_err("停了 guest 就要拒绝");
    assert!(gone.contains("没有活着的 guest"), "{}", gone);
}

/// 目的：真机验收（默认跳过）：外部工作区挂载计划——会话树与共享区是两棵目录时，guest 读得到外部根、
///   写得进私沙、写不了共享区（fenced 拒绝），停 guest 也不动外部文件。
/// 约束：只在 SOLOMNI_VM_LIVE=1 且 SOLOMNI_VM_QEMU 指向真东西、虚拟根（默认 debian）在场时执行；
///   与其它共虚拟根的真机用例串行（doubles::live_guest_lock）。Windows 走 smb（要 SOLOMNI_VM_SMBD），
///   Linux 各起一台验 virtiofs（要 SOLOMNI_VM_VIRTIOFSD）与 9p。
#[test]
pub(crate) fn vm_live_external_workspace_round_trip() {
    use crate::capabilities::vm::api::{ExecPlan, mounts_from};
    use crate::capabilities::vm::domain::guest::{GuestAccess, guest_path, sh_quote};
    use crate::capabilities::workspace::api::{Sandbox, Sandboxes};
    use crate::kernel::api::{FenceSpec, Tier};
    use std::collections::BTreeMap;

    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    let guest_name = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "debian-13".to_string());
    if !live || qemu.trim().is_empty() {
        eprintln!(
            "[探针] 未开启外部工作区真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU 后重跑（虚拟根默认 debian-13，可用 SOLOMNI_VM_GUEST 换）"
        );
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir = match crate::capabilities::vm::api::resolve_guest(
        &crate::tests::root_dir(),
        &guest_name,
    ) {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!(
                "[探针] 本机没有虚拟根「{}」：先 solomni --provision --ensure --yes，或用 SOLOMNI_VM_GUEST 指定",
                guest_name
            );
            return;
        }
    };
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行。
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    // 外部布局：会话树与外部根分处两棵目录（生产里前者在产品根 session/ 下，后者在任意绝对路径）。
    let base = crate::tests::scratch("vm-live-ext");
    let session_root = base.join("session");
    let external = base.join("outside");
    let private = session_root.join("a");
    std::fs::create_dir_all(&private).expect("建私沙");
    std::fs::create_dir_all(&external).expect("建外部根");
    std::fs::write(external.join("marker.txt"), "EXT_MARKER").expect("写外部标记");
    // Sandboxes / stable_roots / check_stable_roots / mounts_from 全用生产同源函数，不手拼挂载。
    let sandboxes = Sandboxes {
        shared: external.clone(),
        session_root: session_root.clone(),
        list: vec![Sandbox {
            work_name: "w".to_string(),
            session: "w".to_string(),
            agent: "a".to_string(),
            shared: external.clone(),
            private: private.clone(),
            modules: BTreeMap::new(),
            modules_with_userdata: Default::default(),
            permissions: Default::default(),
            path_codec: None,
        }],
        codec: None,
    };
    let bridge = image.bridge();
    let (rw, ro_tree) =
        crate::capabilities::conductor::service::guest_job::stable_roots(&sandboxes);
    crate::capabilities::conductor::service::guest_job::check_stable_roots(&sandboxes, &bridge)
        .expect("外部布局的稳定根要能起 guest");
    let boot = FenceSpec {
        agent: String::new(),
        lease: "vm-live-ext".to_string(),
        rw,
        ro: Vec::new(),
        ro_tree,
        private: std::path::PathBuf::new(),
        cwd: std::path::PathBuf::new(),
        net: false,
    };
    let plan = ExecPlan {
        tier: Tier::Vm,
        net: false,
        guest: Some(guest_name.clone()),
        packages: Vec::new(),
    };
    let mounts = mounts_from(&plan, &boot, &bridge);
    assert!(
        mounts.iter().any(|m| m.host == session_root),
        "会话树根要进稳定挂载：{:?}",
        mounts
    );
    assert!(
        mounts.iter().any(|m| m.host == external),
        "外部工作区根要单独进稳定挂载：{:?}",
        mounts
    );
    assert!(
        !mounts
            .iter()
            .any(|m| Some(m.host.as_path()) == external.parent()),
        "外部根的父目录不许进挂载表：{:?}",
        mounts
    );
    // 共享档按平台用户链选：Windows = smb；Linux = virtiofs（工具在场时）+ 9p。
    let nine = crate::capabilities::vm::detail::NinePShare::new();
    let virtiofs = crate::capabilities::vm::detail::VirtioFsShare::new();
    let smb = crate::capabilities::vm::detail::SmbShare::new();
    let virtiofsd = std::env::var("SOLOMNI_VM_VIRTIOFSD")
        .ok()
        .filter(|p| !p.trim().is_empty());
    let smbd = std::env::var("SOLOMNI_VM_SMBD")
        .ok()
        .filter(|p| !p.trim().is_empty());
    let mut variants: Vec<(&'static str, &dyn ShareBackend, Option<String>)> = Vec::new();
    if cfg!(windows) {
        let Some(smbd) = smbd else {
            eprintln!("[探针] 没有 SOLOMNI_VM_SMBD（Windows 外部工作区走 SMB）：跳过");
            return;
        };
        if !image.clients.cifs {
            eprintln!("[探针] 虚拟根没声明 cifs 客户端：跳过");
            return;
        }
        variants.push(("smb", &smb as &dyn ShareBackend, Some(smbd)));
    } else if cfg!(target_os = "linux") {
        if image.clients.virtio_fs
            && let Some(tool) = virtiofsd.clone()
        {
            variants.push(("virtiofs", &virtiofs as &dyn ShareBackend, Some(tool)));
        }
        if image.clients.nine_p {
            variants.push(("9p", &nine as &dyn ShareBackend, None));
        }
        if variants.is_empty() {
            eprintln!("[探针] 没有可用的用户共享档（Linux 需要 9p 或 virtiofsd）：跳过");
            return;
        }
    } else {
        eprintln!("[探针] 外部工作区真机探针本期只覆盖 Windows(SMB) 与 Linux(virtiofs/9p)：跳过");
        return;
    }
    let overlay_dir = crate::tests::scratch("vm-live-ext-overlays");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay_dir);
    let guest_private = guest_path(&private, image.os, &mounts);
    let guest_external = guest_path(&external, image.os, &mounts);
    for (id, backend, tool) in variants {
        let lease = format!("vm-live-ext-{id}");
        let spec = GuestSpec {
            lease: lease.clone(),
            image: image.clone(),
            mounts: mounts.clone(),
            net: false,
            engine_path: Some(qemu.clone()),
            share_path: tool.clone(),
            shares: Vec::new(),
            seed: None,
            isolated: true,
            cache_disk: None,
        };
        let (share_plan, mut guard) = backend
            .prepare("qemu", &spec.mounts, tool.as_deref())
            .expect("共享计划要能派生");
        let mut guest = engine.boot(&spec, &share_plan).expect("起 guest");
        // 1) 外部工作区可读：挂载式共享是实时视图，marker 要读得到。
        let marker = format!("{guest_external}/marker.txt");
        let read = guest.channel().exec(&GuestRun {
            command: format!("cat {}", sh_quote(&marker)),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        });
        assert!(
            read.ok && read.output.contains("EXT_MARKER"),
            "{} guest 要读得到外部工作区：{}",
            id,
            read.output
        );
        // 2) fenced：私沙可写（围栏只给私沙 rw，共享区在 ro_tree）。
        let per_call = FenceSpec {
            agent: "a".to_string(),
            lease: lease.clone(),
            rw: vec![private.clone()],
            ro: Vec::new(),
            ro_tree: vec![external.clone()],
            private: private.clone(),
            cwd: private.clone(),
            net: true,
        };
        let access = GuestAccess::from_fence(&per_call, image.os, &mounts);
        let own_file = format!("{guest_private}/written.txt");
        let write_own = guest.channel().exec_fenced(
            &GuestRun {
                command: format!("printf EXT_OK > {}", sh_quote(&own_file)),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
                timeout_secs: 30,
                output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
            },
            &access,
        );
        assert!(
            write_own.ok,
            "{} fenced 要能在私沙写：{}",
            id, write_own.output
        );
        let landed = std::fs::read_to_string(private.join("written.txt")).unwrap_or_default();
        assert!(
            landed.contains("EXT_OK"),
            "{} 私沙的写要实时落到宿主会话树：{:?}",
            id,
            landed
        );
        // 3) fenced 写共享区（外部根）必须被拒，且宿主外部目录里不能出现该文件。
        let escaped = format!("{guest_external}/escaped.txt");
        let write_shared = guest.channel().exec_fenced(
            &GuestRun {
                command: format!("printf NOPE > {}", sh_quote(&escaped)),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
                timeout_secs: 30,
                output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
            },
            &access,
        );
        assert!(
            !write_shared.ok,
            "{} fenced 写共享区必须被拒：{}",
            id, write_shared.output
        );
        assert!(
            !external.join("escaped.txt").exists(),
            "{} 共享区不该出现 guest 写进去的文件",
            id
        );
        guest.stop();
        // 4) 停 guest 不碰外部工作区：外部文件仍在。
        assert!(
            external.join("marker.txt").is_file(),
            "{} 停 guest 后外部文件仍要在",
            id
        );
        guard.release();
    }
}

/// 目的：真机验收（默认跳过）：UNC 共享根的可写往返——smbd 以 UNC 目录为共享根时，guest 经 CIFS 枚举、
///   建目录、写文件、改名、删除，结果都要真落到宿主的那个 UNC 目录里。
/// 约束：只在 Windows 上、SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU、SOLOMNI_VM_SMBD、SOLOMNI_UNC_ROOT 都在场时执行；
///   与其它共虚拟根的真机用例串行；共享根取 SOLOMNI_UNC_ROOT 下的临时目录，跑完删掉。
#[cfg(windows)]
#[test]
pub(crate) fn vm_live_unc_share_round_trip() {
    use crate::capabilities::vm::domain::guest::{
        GuestMount, GuestRun, OutputLimit, guest_path, sh_quote,
    };

    let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
    let qemu = std::env::var("SOLOMNI_VM_QEMU").unwrap_or_default();
    let unc = std::env::var("SOLOMNI_UNC_ROOT").unwrap_or_default();
    let smbd = std::env::var("SOLOMNI_VM_SMBD").unwrap_or_default();
    // 本机可用的虚拟根是 Windows 宿主那份 Debian 云镜像，清单里叫 windows。
    let guest_name = std::env::var("SOLOMNI_VM_GUEST").unwrap_or_else(|_| "windows".to_string());
    if !live || qemu.trim().is_empty() || unc.trim().is_empty() || smbd.trim().is_empty() {
        eprintln!(
            "[探针] 未开启 UNC 共享根真机验收：设 SOLOMNI_VM_LIVE=1、SOLOMNI_VM_QEMU、SOLOMNI_VM_SMBD、SOLOMNI_UNC_ROOT 后重跑"
        );
        return;
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let base_dir = match crate::capabilities::vm::api::resolve_guest(
        &crate::tests::root_dir(),
        &guest_name,
    ) {
        Ok(d) if probe.is_dir(&d) => d,
        _ => {
            eprintln!(
                "[探针] 本机没有虚拟根「{}」：先用 solomni --provision --ensure --yes，或用 SOLOMNI_VM_GUEST 指定",
                guest_name
            );
            return;
        }
    };
    let _live = crate::tests::doubles::live_guest_lock();
    let image = classify_base(&base_dir, &probe).expect("虚拟根要可用");
    if !image.clients.cifs {
        eprintln!("[探针] 虚拟根没声明 cifs 客户端：跳过");
        return;
    }
    // 共享根本身就是 UNC 目录：这是 cap-primitives 那条"句柄 → 路径字符串 → std::fs"断掉的现场。
    let external = std::path::PathBuf::from(unc.trim())
        .join("vm-live-unc")
        .join(format!("share-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(external.join("dir1")).expect("建 UNC 共享根");
    std::fs::write(external.join("dir1").join("a.txt"), "PRE").expect("写共享里的标记");
    let mounts = vec![GuestMount {
        host: external.clone(),
        guest: "/mnt/ext".to_string(),
        read_only: false,
    }];
    let backend = crate::capabilities::vm::detail::SmbShare::new();
    let (share_plan, mut guard) = backend
        .prepare("qemu", &mounts, Some(smbd.trim()))
        .expect("SMB 共享计划要能派生");
    let overlay_dir = crate::tests::scratch("vm-live-unc-overlays");
    let engine = crate::capabilities::vm::detail::QemuEngine::with_overlay_dir(overlay_dir);
    let spec = GuestSpec {
        lease: "vm-live-unc".to_string(),
        image: image.clone(),
        mounts: mounts.clone(),
        net: false,
        engine_path: Some(qemu.clone()),
        share_path: Some(smbd.trim().to_string()),
        shares: vec!["smb"],
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    let mut guest = engine.boot(&spec, &share_plan).expect("起 guest");
    let guest_external = guest_path(&external, image.os, &mounts);
    let ext = sh_quote(&guest_external);
    // 1) 枚举（read_dir）、建目录（create_dir）、写文件、改名（rename）都要成立。
    let made = guest.channel().exec(&GuestRun {
        command: format!(
            "ls {ext} && mkdir {ext}/made && printf UNC_OK > {ext}/made/a.txt && mv {ext}/made/a.txt {ext}/made/b.txt && cat {ext}/made/b.txt",
        ),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: OutputLimit::Head,
    });
    assert!(
        made.ok,
        "guest 要能在 UNC 共享根上枚举 / 建目录 / 写 / 改名：{}",
        made.output
    );
    assert!(
        made.output.contains("dir1") && made.output.contains("UNC_OK"),
        "枚举要看见共享里原有的目录，写入要读得回：{}",
        made.output
    );
    assert!(
        std::fs::read_to_string(external.join("made").join("b.txt"))
            .is_ok_and(|s| s.contains("UNC_OK")),
        "改名后的文件要真落在宿主 UNC 目录里"
    );
    assert!(
        !external.join("made").join("a.txt").exists(),
        "改名后旧名字要消失"
    );
    // 2) 删除（remove_file）与删空目录（remove_dir）要真的删掉，不能只回成功。
    let unlinked = guest.channel().exec(&GuestRun {
        command: format!("rm {ext}/made/b.txt"),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: OutputLimit::Head,
    });
    assert!(
        unlinked.ok,
        "guest 要能删掉 UNC 共享根里的文件：{}",
        unlinked.output
    );
    // SMB 的删除在服务端按 POSIX 语义落盘，宿主侧看见名字消失可能晚一拍：有界等它。
    let gone = std::time::Instant::now();
    while external.join("made").join("b.txt").exists() && gone.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        !external.join("made").join("b.txt").exists(),
        "删除要真落到宿主 UNC 目录（不许只回成功）"
    );
    let removed = guest.channel().exec(&GuestRun {
        command: format!("rmdir {ext}/made && ls {ext}"),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: OutputLimit::Head,
    });
    assert!(
        removed.ok,
        "guest 要能删掉 UNC 共享根里的空目录：{}",
        removed.output
    );
    assert!(
        !external.join("made").exists(),
        "删除要真落到宿主 UNC 目录（不许只回成功）"
    );
    assert!(
        external.join("dir1").join("a.txt").is_file(),
        "共享里原有的文件不许被动到"
    );
    guest.stop();
    guard.release();
    let _ = std::fs::remove_dir_all(&external);
}
