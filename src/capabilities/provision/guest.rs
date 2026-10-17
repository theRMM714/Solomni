//! 目的：虚拟根的运行期编排——把 guest 编到可用，并把它**真有的**运行能力写进画像。
//! 管：启动 / 等 agent / 执行 / 收尾的顺序，首启与维护启动的分工，标记文件（.qga-ready / .runtimes-ready）与 guest.yaml 的客户端 / 运行能力声明。
//! 不管：清单、下载与作业状态（service.rs）；guest 引擎本身（vm 能力）；运行包库（runtimes/ 与包库扫描）。
//! 联动：由 service.rs 的 guest-runtime 分支调用；vm 能力经 ::api 注入（组合根或 --provision 入口装配）；契约见 RUNTIME_SPEC.md。

use super::service::{deps, under, Dep};
use crate::capabilities::vm::api::{
    classify_base, GuestArch, GuestClients, GuestImage, GuestRun, GuestSpec, SeedDocs, Vm,
};
use std::path::Path;
use std::time::Duration;

/// 目的：首启 cloud-init 的 user-data——装 qemu-guest-agent 并让它随 virtio-serial 端口出现而启动。
const USER_DATA: &str = "#cloud-config\nruncmd:\n  - >-\n    dpkg -s qemu-guest-agent >/dev/null 2>&1 || (apt-get update -o Acquire::Languages=none &&\n    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends qemu-guest-agent);\n    systemctl enable qemu-guest-agent >/dev/null 2>&1; systemctl start qemu-guest-agent >/dev/null 2>&1; true\n";

/// 目的：首启 cloud-init 的 meta-data（固定 instance-id，避免每次启动重跑）。
const META_DATA: &str = "instance-id: solomni-provision\nlocal-hostname: solomni-guest\n";

/// 目的：guest 里的 node 从 **NodeSource** 装——Debian trixie 的 nodejs 依赖 node-corepack，镜像源里不可得。
/// 约束：尽力而为（装不上就不声明）；装完由调用方自证（node --version）。加源之后要**再 update 一次**才看得到 nodejs。
const NODE_REPO: &str = "curl -fsSL https://deb.nodesource.com/setup_22.x -o /tmp/nodesource_setup.sh || true; bash /tmp/nodesource_setup.sh || true";

/// 目的：一次 guest 运行（命令交给 guest 的 /bin/sh；args/env/cwd 都空）。
fn sh(command: &str) -> GuestRun {
    GuestRun {
        command: command.to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    }
}

/// 目的：这次维护启动要装的运行包——Windows 虚拟根的共享客户端是 SMB，要 cifs-utils；Linux 虚拟根走 9p/virtiofs，不需要。
fn required_pkgs(is_windows: bool) -> &'static str {
    if is_windows {
        "python3 cifs-utils"
    } else {
        "python3"
    }
}

/// 目的：镜像声明的共享客户端（写回 guest.yaml 时按它列，按固定顺序）。
fn clients_of(c: &GuestClients) -> Vec<&'static str> {
    let mut out = Vec::new();
    if c.nine_p {
        out.push("9p");
    }
    if c.virtio_fs {
        out.push("virtiofs");
    }
    if c.cifs {
        out.push("cifs");
    }
    out
}

/// 目的：按镜像画像重写 guest.yaml——客户端按这次编排确认的写，运行能力只列**自证过**的那些。
/// 参数：runtimes = 编排在 guest 里逐个自证可用的能力名（没列 = 镜像里确实没有）；clients = 这次确认可用的共享客户端。
fn write_guest_yaml(
    base: &Path,
    image: &GuestImage,
    runtimes: &[String],
    clients: &[&str],
) -> Result<(), String> {
    // arch 要用 guest.yaml 认的写法（amd64 / arm64），不是平台三元组（x86_64 / aarch64）。
    let arch = match image.arch {
        GuestArch::Amd64 => "amd64",
        GuestArch::Arm64 => "arm64",
    };
    let list = runtimes.join(", ");
    let clients = clients.join(", ");
    let text = format!(
        "# 由产品生成：命令通道 QGA；共享客户端 {clients}；运行能力 [{list}]（运行期编排装好并逐个自证过的；\n# 这里没列的 = 镜像里确实没有。虚拟机档的选型诊断据此认定列出的能力已满足，不必再往 runtimes/ 放包）。\n\nid: {id}\narch: {arch}\nos: {os}\nagent: qga\nclients: [{clients}]\nruntimes: [{list}]\n",
        clients = clients,
        list = list,
        id = image.id,
        arch = arch,
        os = image.os.as_str()
    );
    std::fs::write(base.join("guest.yaml"), text).map_err(|e| format!("写 guest.yaml 失败：{}", e))
}

/// 目的：找清单里**本机** QEMU 的真实落点（运行期编排要用它起 guest）。
/// 参数：host_os = 宿主平台名（清单条目按它筛，条目 id 形如 vm.qemu.linux）。
fn qemu_path(root: &Path, host_os: &str) -> Result<std::path::PathBuf, String> {
    let id = format!("vm.qemu.{host_os}");
    deps()?
        .into_iter()
        .find(|d| d.id == id)
        .map(|d| under(root, &d.probe))
        .ok_or_else(|| format!("清单里没有 {id}（先补齐 QEMU）"))
}

/// 目的：把虚拟根编到可用——磁盘根先首启装 agent，随后维护启动装运行包；目录根直接维护启动。
/// 参数：root = 产品根；dep = guest-runtime 清单条目；vm = vm 能力面（None = 组合根没注入，如实拒绝）；
///   force = 用户显式点名这一条（重跑维护启动，不再被 `.runtimes-ready` 挡住）；on_line = 进度行。
/// 约束：同一租约 "provision"，每次启动前先销毁旧的；装完才写标记，失败不写（下次重试）。
/// 约束：只声明**自证过**的能力——python 必须通（模块工具基线），node 尽力装、通了才写进画像。
pub(super) fn install_guest_runtime(
    root: &Path,
    dep: &Dep,
    vm: Option<&(dyn Vm + Send + Sync)>,
    force: bool,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let vm = vm.ok_or_else(|| "guest 运行期编排需要 vm 能力（组合根未注入）".to_string())?;
    let base = under(root, &dep.install);
    let probe = crate::kernel::detail::HostProbeAdapter;
    let image = classify_base(&base, &probe)
        .map_err(|why| format!("虚拟根不可用（{why}）：先跑 solomni --provision --ensure --yes"))?;
    // Windows 虚拟根里没有 agent，要靠首启 cloud-init 装；Linux 虚拟根的 agent 由 guest-rootfs 装进磁盘镜像。
    let is_windows = dep.os == "windows";
    let qemu = qemu_path(root, &dep.os)?;
    if !qemu.is_file() {
        return Err(format!(
            "QEMU 不在场（{}）：先跑 solomni --provision --ensure --yes",
            qemu.display()
        ));
    }
    let engine_path = qemu.to_string_lossy().into_owned();
    let lease = "provision".to_string();
    let qga_marker = base.join(".qga-ready");
    let runtimes_marker = base.join(".runtimes-ready");

    // 首启（只有 Windows 虚拟根要）：cloud-init 从宿主小 HTTP 取 user-data，装 qemu-guest-agent。
    if is_windows && !qga_marker.is_file() {
        on_line("首启 guest：cloud-init 装 qemu-guest-agent");
        let spec = GuestSpec {
            lease: lease.clone(),
            image: image.clone(),
            mounts: Vec::new(),
            net: true,
            engine_path: Some(engine_path.clone()),
            share_path: None,
            // 编排不要挂载：只经 QGA 在 guest 里装东西，所以固定用不依赖外部工具的复制档。
            shares: vec!["copy"],
            seed: Some(SeedDocs {
                user_data: USER_DATA.to_string(),
                meta_data: META_DATA.to_string(),
            }),
            // 供给编排要改的就是基础镜像本身（装 agent / 装运行包），不能写进会被丢掉的可写层。
            isolated: false,
        };
        if let Err(e) = vm.guest_start(&spec) {
            return Err(format!("首启 guest 起不来：{}", e));
        }
        if !vm.guest_wait_agent(&lease, Duration::from_secs(20 * 60)) {
            vm.guest_stop(&lease);
            return Err("等首启 guest 的 qemu-guest-agent 超时".to_string());
        }
        // agent 活着之后顺手关掉 cloud-init；失败不影响"agent 已确认"这个事实。
        let _ = vm.guest_run(&lease, &sh("touch /etc/cloud/cloud-init.disabled; sync"));
        vm.guest_stop(&lease);
        std::fs::write(&qga_marker, "QGA guest-sync 通过（产品 Rust 编排）\n")
            .map_err(|e| format!("写 {} 失败：{}", qga_marker.display(), e))?;
        on_line("guest 里的 qemu-guest-agent 已确认");
    }

    // 维护启动：装运行包并逐个自证，然后把客户端与运行能力写进画像。
    // 触发条件：还没自证过；或磁盘根还没把 cifs-utils 装上（客户端声明要跟着走）。
    let clients: Vec<&'static str> = if is_windows {
        vec!["cifs"]
    } else {
        clients_of(&image.clients)
    };
    let clients_pending = is_windows && !image.clients.cifs;
    if force || !runtimes_marker.is_file() || clients_pending {
        on_line(&format!(
            "维护启动：经 QGA 装运行包（必需 {}；node 走 {}，尽力）并逐个自证",
            required_pkgs(is_windows),
            "apt + NodeSource"
        ));
        let spec = GuestSpec {
            lease: lease.clone(),
            image: image.clone(),
            mounts: Vec::new(),
            net: true,
            engine_path: Some(engine_path),
            share_path: None,
            shares: vec!["copy"],
            seed: None,
            isolated: false,
        };
        if let Err(e) = vm.guest_start(&spec) {
            return Err(format!("维护启动起不来：{}", e));
        }
        if !vm.guest_wait_agent(&lease, Duration::from_secs(15 * 60)) {
            vm.guest_stop(&lease);
            return Err("等维护启动的 qemu-guest-agent 超时".to_string());
        }
        // 镜像里可能残留不完整的 apt lists（首启中途中断过）：整份刷新一遍再装，避免「包明明在却算不可安装」。
        // 磁盘根上就是普通 ext4：apt 与 NodeSource 都按标准路子走（不再有 9p 上的 chown / 解包慢那些绕法）。
        let cmd = format!(
            "rm -rf /var/lib/apt/lists; mkdir -p /var/lib/apt/lists; \
             DEBIAN_FRONTEND=noninteractive apt-get update -o Acquire::Languages=none || true; \
             DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends {req} || true; \
             DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends curl ca-certificates gnupg || true; \
             {repo}; \
             DEBIAN_FRONTEND=noninteractive apt-get update -o Acquire::Languages=none || true; \
             DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends nodejs || true; \
             command -v python >/dev/null 2>&1 || ln -sf /usr/bin/python3 /usr/bin/python; sync",
            req = required_pkgs(is_windows),
            repo = NODE_REPO
        );
        let out = vm.guest_run(&lease, &sh(&cmd));
        // 自证：python 必须通（模块工具的基线）；node 可选，通才写进声明。
        let python_ok = out.ok && vm.guest_run(&lease, &sh("python -V >/dev/null 2>&1")).ok;
        let node_ok = vm
            .guest_run(&lease, &sh("node --version >/dev/null 2>&1"))
            .ok;
        vm.guest_stop(&lease);
        if !python_ok {
            return Err(format!(
                "guest 运行包没装成（python 不可用）：{}",
                out.output
            ));
        }
        let mut proven = vec!["python".to_string()];
        if node_ok {
            proven.push("node".to_string());
        }
        write_guest_yaml(&base, &image, &proven, &clients)?;
        std::fs::write(
            &runtimes_marker,
            format!(
                "产品 Rust 编排：python3 / python 已自证可用{}；共享客户端 [{}]\n",
                if node_ok {
                    "，node 也已自证可用"
                } else {
                    "；node 在这个镜像源里装不上，未声明"
                },
                clients.join(", ")
            ),
        )
        .map_err(|e| format!("写 {} 失败：{}", runtimes_marker.display(), e))?;
        if !qga_marker.is_file() {
            // 目录根的 agent 由 guest-rootfs 装好；维护启动能通就说明它在场——如实落这条事实。
            let _ = std::fs::write(
                &qga_marker,
                "目录根自带 qemu-guest-agent（维护启动已确认）\n",
            );
        }
        if !node_ok {
            let tail: String = {
                let s = out.output.as_str();
                let n = s.chars().count();
                s.chars().skip(n.saturating_sub(1200)).collect()
            };
            on_line(&format!("node 未声明的编排输出（尾段）：{}", tail));
        }
        on_line(if node_ok {
            "guest 运行包已装好（python / node 都自证可用）"
        } else {
            "guest 运行包已装好（python 自证可用；node 装不上，未声明）"
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::provision::service::Dep;

    /// 目的：造一条 guest-runtime 清单条目（字段取自 dependencies.yaml 的真实形状）。
    fn dep(id: &str, os: &str, guest: &str, install: &str, probe: &str) -> Dep {
        Dep {
            id: id.to_string(),
            os: os.to_string(),
            arch: "x86_64".to_string(),
            kind: "guest-runtime".to_string(),
            install: install.to_string(),
            probe: probe.to_string(),
            guest: Some(guest.to_string()),
            source: "guest.debian.image".to_string(),
            strip: 0,
            size_mb: 0,
            sha256: String::new(),
            note: String::new(),
        }
    }

    /// 目的：造一条磁盘根的条目（Windows 形态）。
    fn disk_dep() -> Dep {
        dep(
            "vm.guest-runtime.windows",
            "windows",
            "windows",
            ".vm/guest/windows",
            ".vm/guest/windows/.runtimes-ready",
        )
    }

    /// 目的：在临时产品根里铺一份磁盘根虚拟根 + QEMU 落点。
    fn seed_disk_root(root: &Path) -> std::path::PathBuf {
        let base = root.join(".vm").join("guest").join("windows");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::write(base.join("rootfs.qcow2"), b"qcow2").expect("disk");
        std::fs::write(
            base.join("guest.yaml"),
            "id: debian-13-genericcloud-amd64\narch: amd64\nagent: qga\nclients: []\n",
        )
        .expect("yaml");
        let qemu = root
            .join(".vm")
            .join("qemu")
            .join("bin")
            .join("qemu-system-x86_64.exe");
        std::fs::create_dir_all(qemu.parent().expect("parent")).expect("qemu dir");
        std::fs::write(&qemu, b"exe").expect("qemu");
        base
    }

    /// 目的：没有 vm 能力面时如实拒绝（不假装能编排）。
    #[test]
    fn guest_runtime_refuses_without_vm() {
        let root = crate::tests::scratch("provision-guest-no-vm");
        let err =
            install_guest_runtime(&root, &disk_dep(), None, false, &mut |_| {}).expect_err("要拒");
        assert!(err.contains("vm 能力"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：虚拟根不在场时如实拒绝——先装 vm.guest.名字 那一条。
    #[test]
    fn guest_runtime_refuses_without_image() {
        let root = crate::tests::scratch("provision-guest-no-image");
        let vm = crate::tests::doubles::RecordingVm::default();
        let err = install_guest_runtime(&root, &disk_dep(), Some(&vm), false, &mut |_| {})
            .expect_err("要拒");
        assert!(err.contains("虚拟根不可用"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：磁盘根（Windows 形态）走完首启 + 维护启动，落两个标记，并把客户端写成 cifs。
    #[test]
    fn guest_runtime_writes_markers_and_declares_cifs() {
        let root = crate::tests::scratch("provision-guest-ok");
        let base = seed_disk_root(&root);
        let vm = crate::tests::doubles::RecordingVm::default();
        install_guest_runtime(&root, &disk_dep(), Some(&vm), false, &mut |_| {}).expect("编排");
        assert!(base.join(".qga-ready").is_file(), "首启标记要落");
        assert!(base.join(".runtimes-ready").is_file(), "运行包标记要落");
        let y = std::fs::read_to_string(base.join("guest.yaml")).expect("yaml");
        assert!(y.contains("clients: [cifs]"), "{}", y);
        assert!(y.contains("runtimes: [python, node]"), "{}", y);
        assert!(y.contains("arch: amd64"), "arch 要用画像认的写法：{}", y);
        // 写出的画像必须仍可被识别（arch 写法错了就会在这里炸），并如实声明装好的运行能力。
        let image = crate::capabilities::vm::api::classify_base(
            &base,
            &crate::kernel::detail::HostProbeAdapter,
        )
        .expect("运行期编排写出的画像要仍可识别");
        assert_eq!(
            image.runtimes,
            vec!["node".to_string(), "python".to_string()],
            "镜像要如实声明它装好的运行能力"
        );
        let runs = vm.runs.lock().expect("锁").clone();
        assert!(
            runs.iter().any(|c| c.contains("cifs-utils")),
            "磁盘根的运行包清单要带 cifs-utils：{:?}",
            runs
        );
        assert!(
            runs.iter().any(|c| c.contains("deb.nodesource.com")),
            "node 要从 NodeSource 装：{:?}",
            runs
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：真机验收（默认跳过）：Linux 虚拟根的运行期编排真起一次 guest，把自证过的能力写进画像。
    /// 约束：只在 SOLOMNI_VM_LIVE=1、本机是 Linux、虚拟根与 QEMU 都在场时执行；否则打印 [探针] 如实跳过。
    #[test]
    fn provision_linux_runtime_proves_runtimes_live() {
        use crate::kernel::ports::HostProbe;
        let live = std::env::var("SOLOMNI_VM_LIVE").as_deref() == Ok("1");
        if !live || std::env::consts::OS != "linux" {
            eprintln!("[探针] 未开启 Linux 运行期编排的真机验收：设 SOLOMNI_VM_LIVE=1（仅 Linux 宿主）后重跑");
            return;
        }
        let root = crate::tests::root_dir();
        let Some(dep) = deps()
            .expect("清单")
            .into_iter()
            .find(|d| d.id == "vm.guest-runtime.linux")
        else {
            eprintln!("[探针] 清单里没有 vm.guest-runtime.linux");
            return;
        };
        let base = under(&root, &dep.install);
        let probe = crate::kernel::detail::HostProbeAdapter;
        let qemu_ok = qemu_path(&root, &dep.os)
            .map(|p| p.is_file())
            .unwrap_or(false);
        if !probe.is_dir(&base) || !qemu_ok {
            eprintln!("[探针] 虚拟根或 QEMU 不在场：先 solomni --provision --ensure=vm.qemu.linux,vm.guest.linux --yes");
            return;
        }
        // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
        let _live = crate::tests::doubles::live_guest_lock();
        let vm = crate::capabilities::vm::service::VmService::new(
            std::sync::Arc::new(crate::capabilities::vm::detail::FsPackages::new(
                root.join("runtimes"),
            )),
            std::sync::Arc::new(crate::capabilities::vm::detail::QemuEngine::new()),
            vec![std::sync::Arc::new(
                crate::capabilities::vm::detail::CopyShare::new(),
            )],
        );
        let mut sink = |l: &str| eprintln!("[探针] {}", l);
        install_guest_runtime(&root, &dep, Some(&vm), true, &mut sink)
            .expect("真机运行期编排要成功");
        let image = classify_base(&base, &probe).expect("编排后的画像要仍可识别");
        assert!(
            image.runtimes.iter().any(|r| r == "python"),
            "python 必须自证并写进画像：{:?}",
            image.runtimes
        );
        assert!(base.join(".runtimes-ready").is_file(), "运行包标记要落");
    }
}
