//! 目的：虚拟根的运行期编排——把 guest 编到可用，并把它**真有的**运行能力写进画像。
//! 管：启动 / 等 agent / 执行 / 收尾的顺序，首启与维护启动的分工，标记文件（.qga-ready / .runtimes-ready）与 guest.yaml 的客户端 / 运行能力声明。
//! 不管：清单、下载与作业状态（service.rs）；guest 引擎本身（vm 能力）；运行包库（runtimes/ 与包库扫描）。
//! 联动：由 service.rs 的 guest-runtime 分支调用；vm 能力经 ::api 注入（组合根或 --provision 入口装配）；契约见 RUNTIME_SPEC.md。

use super::service::{deps, links, source_url, under, Dep};
use crate::capabilities::vm::api::{
    classify_base, GuestArch, GuestClients, GuestImage, GuestRun, GuestSpec, SeedDocs, Vm,
};
use std::path::Path;
use std::time::Duration;

/// 目的：首启 cloud-init 的 user-data——装 qemu-guest-agent 并让它随 virtio-serial 端口出现而启动。
const USER_DATA: &str = "#cloud-config\nruncmd:\n  - >-\n    dpkg -s qemu-guest-agent >/dev/null 2>&1 || (apt-get update -o Acquire::Languages=none &&\n    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends qemu-guest-agent);\n    systemctl enable qemu-guest-agent >/dev/null 2>&1; systemctl start qemu-guest-agent >/dev/null 2>&1; true\n";

/// 目的：首启 cloud-init 的 meta-data（固定 instance-id，避免每次启动重跑）。
const META_DATA: &str = "instance-id: solomni-provision\nlocal-hostname: solomni-guest\n";

/// 目的：guest 的 Node 用固定版本官方二进制安装。
/// 约束：版本 / URL / sha256 的唯一真相在 links.yaml；升级时同 diff 改那两行与这里的版本常量。
const NODE_TOP: &str = "node-v24.11.0-linux-x64";
const NODE_ROOT: &str = "/opt/solomni/toolchains/node/24.11.0";
const NODE_KEY: &str = "toolchain.node.24.11.0.linux-x86_64";
const NODE_SHA_KEY: &str = "toolchain.node.24.11.0.linux-x86_64.sha256";

/// 目的：Linux 维护启动前的网络预置——Linux 磁盘根不走 cloud-init 首启（agent 打包进镜像），网卡没人配。
/// 约束：QEMU 用户网络（slirp）固定 10.0.2.0/24，网关 `.2`、DNS `.3`；先用镜像自带的 systemd-networkd 走 DHCP，
///   再按 slirp 的固定网段做静态兜底。Windows 虚拟根由首启 cloud-init 把网络写进镜像，不需要这一段。
const NET_UP: &str = "mkdir -p /etc/systemd/network; printf '[Match]\\nName=en* eth*\\n\\n[Network]\\nDHCP=yes\\n' > /etc/systemd/network/10-solomni.network; systemctl enable --now systemd-networkd >/dev/null 2>&1 || true; systemctl enable --now systemd-resolved >/dev/null 2>&1 || true; IF=$(ls /sys/class/net | grep -v '^lo$' | head -1); ip link set \"$IF\" up 2>/dev/null || true; ip addr show \"$IF\" 2>/dev/null | grep -q '10\\.0\\.2\\.' || { ip addr add 10.0.2.15/24 dev \"$IF\" 2>/dev/null || true; ip route add default via 10.0.2.2 dev \"$IF\" 2>/dev/null || true; }; grep -q '10\\.0\\.2\\.3' /etc/resolv.conf 2>/dev/null || { rm -f /etc/resolv.conf; printf 'nameserver 10.0.2.3\\n' > /etc/resolv.conf; }; ";

/// 目的：让 apt 先取压缩索引（xz / gz）——Debian 的 InRelease 里也列着未压缩的 Packages / Sources，
///   不指定顺序时 apt 会抓那份（几十 MB，还常在 CDN 上缺文件），既费空间又可能 404。
const APT_COMPRESSED: &str = "echo 'Acquire::CompressionTypes::Order { \"xz\"; \"gz\"; };' > /etc/apt/apt.conf.d/99solomni-compressed; ";

/// 目的：错误回执只留尾段，避免把整段 apt / curl 输出塞进错误消息。
fn tail_text(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

/// 目的：一次 guest 运行（命令交给 guest 的 /bin/sh；带标准 PATH，供给编排里的 g++ / ld / curl 才找得到）。
/// 约束：PATH 写在 shell 命令里，不放进 QGA env（guest-exec 的 env 由 QGA 处理，空 env 最稳）。
fn sh(command: &str) -> GuestRun {
    GuestRun {
        command: format!(
            "export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin; {command}"
        ),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::api::OutputLimit::Head,
    }
}

/// 目的：这次维护启动要装的运行包——Windows 虚拟根的共享客户端是 SMB，要 cifs-utils；Linux 虚拟根走 9p/virtiofs，不需要。
/// 约束：C/C++ 用 Debian 原生 GCC（与 guest 的 libc / libstdc++ 匹配）；Node 走下面的官方二进制。
fn required_pkgs(is_windows: bool) -> &'static str {
    if is_windows {
        "python3 cifs-utils gcc g++ make libc6-dev xz-utils"
    } else {
        "python3 gcc g++ make libc6-dev xz-utils"
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

/// 目的：guest 内安装固定版本 Node 的 shell 片段（下载 / 校验 / 解包 / 软链到 /usr/local/bin）。
/// 错误：links.yaml 缺 URL 或 sha256 时如实拒绝（不静默换版本 / 不跳过校验）。
fn node_install_script() -> Result<String, String> {
    let all = links()?;
    let url = source_url(NODE_KEY, &all)?;
    let sha = all
        .get(NODE_SHA_KEY)
        .ok_or_else(|| format!("links.yaml 缺 {}", NODE_SHA_KEY))?;
    Ok(format!(
        "set -e; \
         rm -rf {root}; mkdir -p {root} /usr/local/bin; \
         curl -fsSL --retry 3 --retry-all-errors -o /tmp/solomni-node.tar.xz '{url}'; \
         echo '{sha}  /tmp/solomni-node.tar.xz' | sha256sum -c -; \
         tar -xJf /tmp/solomni-node.tar.xz -C {root}; \
         ln -sfn {root}/{top}/bin/node /usr/local/bin/node; \
         ln -sfn {root}/{top}/bin/npm /usr/local/bin/npm; \
         ln -sfn {root}/{top}/bin/npx /usr/local/bin/npx; \
         rm -f /tmp/solomni-node.tar.xz; sync",
        root = NODE_ROOT,
        top = NODE_TOP,
        url = url,
        sha = sha
    ))
}

/// 目的：把虚拟根编到可用——先确保 agent 在场（Windows 首启装、Linux 随镜像打包），随后维护启动装运行包。
/// 参数：root = 产品根；dep = guest-runtime 清单条目；vm = vm 能力面（None = 组合根没注入，如实拒绝）；
///   force = 用户显式点名这一条（重跑维护启动，不再被 `.runtimes-ready` 挡住）；on_line = 进度行。
/// 约束：同一租约 "provision"，每次启动前先销毁旧的；装完才写标记，失败不写（下次重试）。
/// 约束：只声明**自证过**的能力——python / node / cc 三者都通才写进画像；失败不写标记（下次重试）。
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
    // Node 预装用官方 linux-x86_64 二进制；arm64 guest 暂不伪装成支持。
    if image.arch != GuestArch::Amd64 {
        return Err("基础工具链预装目前只支持 amd64 guest（Node 官方包只有 x86_64）；arm64 请走 runtimes/ 扩展".to_string());
    }
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
            // 编排不要挂载：只经 QGA 在 guest 里装东西，所以固定用内部 none 档（无挂载、无同步）。
            shares: vec!["none"],
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
    // 已有运行标记但 Windows 画像缺 cifs = 不是当前基线；只拒绝，不自动修补旧资产。
    if clients_pending && runtimes_marker.is_file() {
        return Err(format!(
            "guest 画像不是当前 Windows 基线（缺 cifs 客户端）：删除 {} 后重新 --provision --ensure --yes；不自动修补",
            base.display()
        ));
    }
    if force || !runtimes_marker.is_file() {
        on_line(&format!(
            "维护启动：经 QGA 装基础工具链（apt {}；Node 用固定版本官方二进制）并逐个自证",
            required_pkgs(is_windows)
        ));
        let spec = GuestSpec {
            lease: lease.clone(),
            image: image.clone(),
            mounts: Vec::new(),
            net: true,
            engine_path: Some(engine_path),
            share_path: None,
            shares: vec!["none"],
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
        // Linux 磁盘根没有 cloud-init 首启（agent 打包进镜像），网卡没人配：先按 QEMU 用户网络配网（Windows 不需要）。
        let net = if is_windows { "" } else { NET_UP };
        let py_probe =
            "mkdir -p /usr/local/bin; command -v python >/dev/null 2>&1 || ln -sf /usr/bin/python3 /usr/local/bin/python; python -V >/dev/null 2>&1";
        let mut python_ok = vm.guest_run(&lease, &sh(py_probe)).ok;
        // 索引与安装分两步：各给一条进度行——一步跑几分钟时界面不会停在同一条上。
        on_line("刷新 apt 索引…");
        let index_cmd = format!(
            "{net}{apt} sed -i 's/^Types: deb deb-src/Types: deb/' /etc/apt/sources.list.d/debian.sources 2>/dev/null || true; \
             rm -rf /var/lib/apt/lists /var/cache/apt/archives; mkdir -p /var/lib/apt/lists; \
             DEBIAN_FRONTEND=noninteractive apt-get update -o Acquire::Languages=none || true",
            apt = APT_COMPRESSED,
            net = net
        );
        let index_out = vm.guest_run(&lease, &sh(&index_cmd));
        on_line(&format!(
            "安装基础工具链（apt {}）…",
            required_pkgs(is_windows)
        ));
        // C/C++ 用 Debian 原生 GCC / G++；Node 从官方二进制装。
        let install_cmd = format!(
            "DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends {req} curl ca-certificates; \
             apt-get clean; rm -rf /var/cache/apt/archives/*; sync",
            req = required_pkgs(is_windows)
        );
        let install_out = vm.guest_run(&lease, &sh(&install_cmd));
        // 失败回执同时带索引与安装两段；判据仍是安装命令（索引段沿用本地缓存也算成功）。
        let apt_out = crate::kernel::api::ToolOutcome {
            ok: install_out.ok,
            output: format!("{}\n{}", index_out.output, install_out.output),
        };
        if !python_ok {
            python_ok = apt_out.ok && vm.guest_run(&lease, &sh(py_probe)).ok;
        }
        // Node：固定版本官方二进制，校验 sha256 后解到 /opt/solomni/toolchains；失败就如实失败（不静默降级）。
        on_line("安装 Node（固定版本官方二进制）…");
        let node_script = node_install_script()?;
        let node_run = if apt_out.ok {
            vm.guest_run(&lease, &sh(&node_script))
        } else {
            crate::kernel::api::ToolOutcome {
                ok: false,
                output: apt_out.output.clone(),
            }
        };
        on_line("自证 python / node / C-C++…");
        let node_ok = node_run.ok
            && vm
                .guest_run(&lease, &sh("node --version >/dev/null 2>&1"))
                .ok;
        // Debian 的 gcc/g++ 以裸名调用时会把 libexec 按相对路径找（QGA 环境尤其明显）：
        // 用 /usr/local/bin 的包装固定成绝对路径，并让 make / cc 也走同一份。
        let wrapper_cmd = "mkdir -p /usr/local/bin; \
             printf '#!/bin/sh\\nexec /usr/bin/gcc -B/usr/bin \"$@\"\\n' > /usr/local/bin/gcc; \
             printf '#!/bin/sh\\nexec /usr/bin/g++ -B/usr/bin \"$@\"\\n' > /usr/local/bin/g++; \
             printf '#!/bin/sh\\nexec /usr/bin/gcc -B/usr/bin \"$@\"\\n' > /usr/local/bin/cc; \
             printf '#!/bin/sh\\nexec /usr/bin/g++ -B/usr/bin \"$@\"\\n' > /usr/local/bin/c++; \
             chmod 0755 /usr/local/bin/gcc /usr/local/bin/g++ /usr/local/bin/cc /usr/local/bin/c++; sync";
        let wrapper_run = if apt_out.ok {
            vm.guest_run(&lease, &sh(wrapper_cmd))
        } else {
            crate::kernel::api::ToolOutcome {
                ok: false,
                output: apt_out.output.clone(),
            }
        };
        let cc_run = vm.guest_run(
            &lease,
            &sh("set -e; command -v gcc; command -v g++; gcc --version | head -1; g++ --version | head -1; g++ -print-prog-name=cc1plus; printf '#include <iostream>\nint main(){return 0;}\n' > /tmp/solomni-cc.cpp; g++ -std=c++17 /tmp/solomni-cc.cpp -o /tmp/solomni-cc; /tmp/solomni-cc"),
        );
        let cc_ok = apt_out.ok && wrapper_run.ok && cc_run.ok;
        vm.guest_stop(&lease);
        if !python_ok || !node_ok || !cc_ok {
            return Err(format!(
                "guest 基础工具链没装成（python={python_ok} node={node_ok} cc={cc_ok}）\napt 尾段：{}\nnode 尾段：{}\nwrapper 尾段：{}\ncc 尾段：{}",
                tail_text(&apt_out.output, 1200),
                tail_text(&node_run.output, 1200),
                tail_text(&wrapper_run.output, 1200),
                tail_text(&cc_run.output, 1200)
            ));
        }
        // cc 是模块能力名；gcc 是 build.needs 使用的可执行名（两者都如实声明，避免构建期误判缺包）。
        let proven = vec![
            "python".to_string(),
            "node".to_string(),
            "cc".to_string(),
            "gcc".to_string(),
        ];
        write_guest_yaml(&base, &image, &proven, &clients)?;
        std::fs::write(
            &runtimes_marker,
            format!(
                "产品 Rust 编排：python / node（固定版本）/ cc + gcc（Debian GCC）已逐个自证可用；共享客户端 [{}]\n",
                clients.join(", ")
            ),
        )
        .map_err(|e| format!("写 {} 失败：{}", runtimes_marker.display(), e))?;
        if !qga_marker.is_file() {
            // 用户自备的虚拟根没有 .qga-ready：维护启动能通就说明 agent 在场——如实落这条事实。
            let _ = std::fs::write(&qga_marker, "维护启动已确认 qemu-guest-agent 在场\n");
        }
        on_line("guest 基础工具链已装好（python / node / cc + gcc 都自证可用）");
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
            family: None,
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
        assert!(y.contains("runtimes: [python, node, cc, gcc]"), "{}", y);
        assert!(y.contains("arch: amd64"), "arch 要用画像认的写法：{}", y);
        // 写出的画像必须仍可被识别（arch 写法错了就会在这里炸），并如实声明装好的运行能力。
        let image = crate::capabilities::vm::api::classify_base(
            &base,
            &crate::kernel::detail::HostProbeAdapter,
        )
        .expect("运行期编排写出的画像要仍可识别");
        assert_eq!(
            image.runtimes,
            vec![
                "cc".to_string(),
                "gcc".to_string(),
                "node".to_string(),
                "python".to_string()
            ],
            "镜像要如实声明它装好的运行能力"
        );
        let runs = vm.runs.lock().expect("锁").clone();
        assert!(
            runs.iter().any(|c| c.contains("cifs-utils")),
            "磁盘根的运行包清单要带 cifs-utils：{:?}",
            runs
        );
        assert!(
            runs.iter().any(|c| c.contains("nodejs.org/dist/v24.11.0")),
            "node 要从固定版本官方二进制装：{:?}",
            runs
        );
        assert!(
            runs.iter().any(|c| c.contains("gcc") && c.contains("g++")),
            "C/C++ 要随 apt 装进 guest：{:?}",
            runs
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：已有运行标记但 Windows 画像缺 cifs 的旧资产只拒绝，不自动修补。
    #[test]
    fn stale_windows_guest_is_refused_instead_of_repaired() {
        let root = crate::tests::scratch("provision-guest-stale");
        let base = seed_disk_root(&root);
        std::fs::write(base.join(".runtimes-ready"), "old\n").expect("marker");
        let vm = crate::tests::doubles::RecordingVm::default();
        let err = install_guest_runtime(&root, &disk_dep(), Some(&vm), false, &mut |_| {})
            .expect_err("旧画像要拒");
        assert!(err.contains("不是当前 Windows 基线"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目的：Linux 磁盘根的维护启动必须先配网——磁盘根没有 cloud-init 首启，网卡没人配，
    ///   apt 与 Node 下载只能解析失败（真机验收就是这么露的）。
    #[test]
    fn linux_maintenance_brings_up_the_network_first() {
        use std::fs;
        let root = crate::tests::scratch("provision-guest-net");
        let base = root.join(".vm").join("guest").join("debian");
        fs::create_dir_all(&base).expect("建虚拟根");
        fs::write(base.join("rootfs.ext4"), b"img").expect("磁盘镜像");
        fs::write(
            base.join("guest.yaml"),
            "id: t\nos: linux\narch: amd64\nagent: qga\nclients: [9p, virtiofs]\n",
        )
        .expect("画像");
        let qemu = root
            .join(".vm")
            .join("qemu")
            .join("bin")
            .join("qemu-system-x86_64");
        fs::create_dir_all(qemu.parent().expect("父目录")).expect("建 qemu 目录");
        fs::write(&qemu, b"exe").expect("qemu");
        let vm = crate::tests::doubles::RecordingVm::default();
        let d = dep(
            "vm.guest-runtime.linux",
            "linux",
            "debian",
            ".vm/guest/debian",
            ".vm/guest/debian/.runtimes-ready",
        );
        let mut lines: Vec<String> = Vec::new();
        install_guest_runtime(&root, &d, Some(&vm), true, &mut |l| {
            lines.push(l.to_string())
        })
        .expect("编排");
        let runs = vm.runs.lock().expect("锁").clone();
        for want in [
            "刷新 apt 索引",
            "安装基础工具链",
            "安装 Node",
            "自证 python / node / C-C++",
        ] {
            assert!(
                lines.iter().any(|l| l.contains(want)),
                "进度行缺「{}」：{:?}",
                want,
                lines
            );
        }
        assert!(
            runs.iter().filter(|c| c.contains("apt-get update")).count() == 1
                && runs
                    .iter()
                    .filter(|c| c.contains("apt-get install"))
                    .count()
                    == 1,
            "索引与安装要分两条命令：{:?}",
            runs
        );
        assert!(
            runs.iter()
                .any(|c| c.contains("systemd-networkd") && c.contains("10.0.2.15")),
            "Linux 维护启动要先配网：{:?}",
            runs
        );
        assert!(
            !runs.iter().any(|c| c.contains("cifs-utils")),
            "Linux 不需要 cifs-utils：{:?}",
            runs
        );
        let _ = fs::remove_dir_all(&root);
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
            std::sync::Arc::new(crate::capabilities::vm::detail::QemuEngine::new()),
            vec![std::sync::Arc::new(
                crate::capabilities::vm::detail::NoneShare::new(),
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
