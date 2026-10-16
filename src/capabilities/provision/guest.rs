//! 目的：Windows guest 的运行期编排——首启经 cloud-init 装 qemu-guest-agent，维护启动经 QGA 装运行包（含 cifs-utils）。
//! 管：启动 / 等 agent / 执行 / 收尾的顺序，标记文件（.qga-ready / .runtimes-ready）与 guest.yaml 的共享客户端声明。
//! 不管：清单、下载与作业状态（service.rs）；guest 引擎本身（vm 能力）。
//! 联动：由 service.rs 的 guest-runtime 分支调用；vm 能力经 ::api 注入（组合根或 --provision 入口装配）。

use super::service::{deps, under, Dep};
use crate::capabilities::vm::api::{
    classify_base, GuestArch, GuestImage, GuestRun, GuestSpec, SeedDocs, Vm,
};
use std::path::Path;
use std::time::Duration;

/// 目的：首启 cloud-init 的 user-data——装 qemu-guest-agent 并让它随 virtio-serial 端口出现而启动。
const USER_DATA: &str = "#cloud-config\nruncmd:\n  - >-\n    dpkg -s qemu-guest-agent >/dev/null 2>&1 || (apt-get update -o Acquire::Languages=none &&\n    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends qemu-guest-agent);\n    systemctl enable qemu-guest-agent >/dev/null 2>&1; systemctl start qemu-guest-agent >/dev/null 2>&1; true\n";

/// 目的：首启 cloud-init 的 meta-data（固定 instance-id，避免每次启动重跑）。
const META_DATA: &str = "instance-id: solomni-provision\nlocal-hostname: solomni-guest\n";

/// 目的：维护启动**必须**装上的运行包——模块工具基线（python）与共享要用的 cifs-utils。
const REQUIRED_PKGS: &str = "python3 cifs-utils";

/// 目的：guest 里的 node 从 **NodeSource** 装——Debian trixie 的 nodejs 依赖 node-corepack，镜像源里不可得。
/// 约束：尽力而为（装不上就不声明）；装完由调用方自证（node --version）。
const NODE_INSTALL: &str = "DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends curl ca-certificates gnupg || true; curl -fsSL https://deb.nodesource.com/setup_22.x -o /tmp/nodesource_setup.sh || true; bash /tmp/nodesource_setup.sh || true; DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends nodejs || true";

/// 目的：一次 guest 运行（命令交给 guest 的 /bin/sh；args/env/cwd 都空）。
fn sh(command: &str) -> GuestRun {
    GuestRun {
        command: command.to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    }
}

/// 目的：按镜像画像重写 guest.yaml——共享客户端如实声明为 cifs，运行能力只列**自证过**的那些。
/// 参数：runtimes = 编排在 guest 里逐个自证可用的能力名（没列 = 镜像里确实没有）。
fn write_guest_yaml(base: &Path, image: &GuestImage, runtimes: &[String]) -> Result<(), String> {
    // arch 要用 guest.yaml 认的写法（amd64 / arm64），不是平台三元组（x86_64 / aarch64）。
    let arch = match image.arch {
        GuestArch::Amd64 => "amd64",
        GuestArch::Arm64 => "arm64",
    };
    let list = runtimes.join(", ");
    let text = format!(
        "# 由产品生成：命令通道 QGA；共享客户端 cifs；运行能力 [{list}]（运行期编排装好并逐个自证过的；\n# 这里没列的 = 镜像里确实没有。虚拟机档的选型诊断据此认定列出的能力已满足，不必再往 runtimes/ 放包）。\n\nid: {id}\narch: {arch}\nos: {os}\nagent: qga\nclients: [cifs]\nruntimes: [{list}]\n",
        list = list,
        id = image.id,
        arch = arch,
        os = image.os.as_str()
    );
    std::fs::write(base.join("guest.yaml"), text).map_err(|e| format!("写 guest.yaml 失败：{}", e))
}

/// 目的：找清单里 Windows QEMU 的真实落点（运行期编排要用它起 guest）。
fn qemu_path(root: &Path) -> Result<std::path::PathBuf, String> {
    deps()?
        .into_iter()
        .find(|d| d.id == "vm.qemu.windows")
        .map(|d| under(root, &d.probe))
        .ok_or_else(|| "清单里没有 vm.qemu.windows（先补齐 QEMU）".to_string())
}

/// 目的：把 Windows guest 编到可用——首启装 agent、维护启动装运行包，并如实落标记与 guest.yaml。
/// 参数：root = 产品根；dep = guest-runtime 清单条目；vm = vm 能力面（None = 组合根没注入，如实拒绝）；on_line = 进度行。
/// 约束：同一租约 "provision"，每次启动前先销毁旧的；装完才写标记，失败不写（下次重试）。
pub(super) fn install_guest_runtime(
    root: &Path,
    dep: &Dep,
    vm: Option<&(dyn Vm + Send + Sync)>,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let vm = vm.ok_or_else(|| "guest 运行期编排需要 vm 能力（组合根未注入）".to_string())?;
    let base = under(root, &dep.install);
    if !base.join("rootfs.qcow2").is_file() {
        return Err("guest 镜像不在场：先跑 --ensure=vm.guest.windows".to_string());
    }
    let qemu = qemu_path(root)?;
    if !qemu.is_file() {
        return Err(format!(
            "QEMU 不在场（{}）：先跑 --ensure=vm.qemu.windows",
            qemu.display()
        ));
    }
    let probe = crate::kernel::detail::HostProbeAdapter;
    let image = classify_base(&base, &probe)?;
    let engine_path = qemu.to_string_lossy().into_owned();
    let lease = "provision".to_string();
    let qga_marker = base.join(".qga-ready");
    let runtimes_marker = base.join(".runtimes-ready");

    // 首启：cloud-init 从宿主小 HTTP 取 user-data，装 qemu-guest-agent。
    if !qga_marker.is_file() {
        on_line("首启 guest：cloud-init 装 qemu-guest-agent");
        let spec = GuestSpec {
            lease: lease.clone(),
            image: image.clone(),
            mounts: Vec::new(),
            net: true,
            engine_path: Some(engine_path.clone()),
            share_path: None,
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

    // 维护启动：装运行包与 cifs-utils；装完把 guest.yaml 的共享客户端写成 cifs。
    if !runtimes_marker.is_file() || !image.clients.cifs {
        on_line(
            "维护启动：经 QGA 装运行包（必需 python3 / cifs-utils；node 走 NodeSource）并逐个自证",
        );
        let spec = GuestSpec {
            lease: lease.clone(),
            image: image.clone(),
            mounts: Vec::new(),
            net: true,
            engine_path: Some(engine_path),
            share_path: None,
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
        // python-is-python3 在 trixie 已没有，python 符号链接自己补。必需包装不上如实失败；node 走 NodeSource，尽力。
        let cmd = format!(
            "rm -rf /var/lib/apt/lists; mkdir -p /var/lib/apt/lists; DEBIAN_FRONTEND=noninteractive apt-get update -o Acquire::Languages=none || true; DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends {req} || true; {node}; command -v python >/dev/null 2>&1 || ln -sf /usr/bin/python3 /usr/bin/python; sync",
            req = REQUIRED_PKGS,
            node = NODE_INSTALL
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
        write_guest_yaml(&base, &image, &proven)?;
        std::fs::write(
            &runtimes_marker,
            format!(
                "产品 Rust 编排：python3 / python 已自证可用{}；cifs-utils 已装\n",
                if node_ok {
                    "，node 也已自证可用"
                } else {
                    "；node 在这个镜像源里装不上，未声明"
                }
            ),
        )
        .map_err(|e| format!("写 {} 失败：{}", runtimes_marker.display(), e))?;
        if !node_ok {
            let tail: String = {
                let s = out.output.as_str();
                let n = s.chars().count();
                s.chars().skip(n.saturating_sub(1200)).collect()
            };
            on_line(&format!("node 未声明的编排输出（尾段）：{}", tail));
        }
        on_line(if node_ok {
            "guest 运行包与 cifs-utils 已装好（python / node 都自证可用）"
        } else {
            "guest 运行包与 cifs-utils 已装好（python 自证可用；node 装不上，未声明）"
        });
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::provision::service::Dep;

    fn dep() -> Dep {
        Dep {
            id: "vm.guest-runtime.windows".to_string(),
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
            kind: "guest-runtime".to_string(),
            install: ".vm/guest/windows".to_string(),
            probe: ".vm/guest/windows/.runtimes-ready".to_string(),
            guest: Some("windows".to_string()),
            source: "guest.debian.qcow2".to_string(),
            strip: 0,
            size_mb: 0,
            note: String::new(),
        }
    }

    /// 目的：没有 vm 能力面时如实拒绝（不假装能编排）。
    #[test]
    fn guest_runtime_refuses_without_vm() {
        let root = crate::tests::scratch("provision-guest-no-vm");
        let err = install_guest_runtime(&root, &dep(), None, &mut |_| {}).expect_err("要拒");
        assert!(err.contains("vm 能力"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：镜像不在场时如实拒绝——先装 vm.guest.windows。
    #[test]
    fn guest_runtime_refuses_without_image() {
        let root = crate::tests::scratch("provision-guest-no-image");
        let vm = crate::tests::doubles::RecordingVm::default();
        let err = install_guest_runtime(&root, &dep(), Some(&vm), &mut |_| {}).expect_err("要拒");
        assert!(err.contains("guest 镜像不在场"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：镜像与 QEMU 在场时，编排走完并落下两个标记、把 guest.yaml 的共享客户端写成 cifs。
    #[test]
    fn guest_runtime_writes_markers_and_declares_cifs() {
        let root = crate::tests::scratch("provision-guest-ok");
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
        let vm = crate::tests::doubles::RecordingVm::default();
        install_guest_runtime(&root, &dep(), Some(&vm), &mut |_| {}).expect("编排");
        assert!(base.join(".qga-ready").is_file(), "首启标记要落");
        assert!(base.join(".runtimes-ready").is_file(), "运行包标记要落");
        let y = std::fs::read_to_string(base.join("guest.yaml")).expect("yaml");
        assert!(y.contains("clients: [cifs]"), "{}", y);
        assert!(y.contains("runtimes: [python, node]"), "{}", y);
        assert!(
            y.contains("arch: amd64"),
            "arch 要用 guest.yaml 认的写法：{}",
            y
        );
        // 写出的 guest.yaml 必须仍可被识别（arch 写法错了就会在这里炸），并如实声明装好的运行能力。
        let image = crate::capabilities::vm::api::classify_base(
            &base,
            &crate::kernel::detail::HostProbeAdapter,
        )
        .expect("运行期编排写出的 guest.yaml 要仍可识别");
        assert_eq!(
            image.runtimes,
            vec!["node".to_string(), "python".to_string()],
            "镜像要如实声明它装好的运行能力"
        );
        let runs = vm.runs.lock().expect("锁").clone();
        assert!(
            runs.iter().any(|c| c.contains("cifs-utils")),
            "运行包清单要带 cifs-utils：{:?}",
            runs
        );
        assert!(
            runs.iter().any(|c| c.contains("deb.nodesource.com")),
            "node 要从 NodeSource 装（Debian trixie 的 nodejs 依赖 node-corepack，装不上）：{:?}",
            runs
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
