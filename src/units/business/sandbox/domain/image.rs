//! 目的：客机镜像的契约——一份虚拟根里有什么、怎么判定、画像（guest.yaml）怎么读写。
//! 管：`GuestImage` / `GuestClients` / `GuestAgent` 形状、`classify_base` 判定、`write_image` 写画像、`guest_platform` 平台名，
//!   以及按执行规格派生的 `guest_image_of` / `guest_platform_of` / `guest_os_of` / `guest_runtimes_of` / `path_codec_of`。
//! 不管：镜像怎么造出来（`provision`）、客机怎么跑（`vm`）、虚拟根名到目录的解析（`vmstore` 的 `resolve_guest`）。
//! 联动：`vm` 起客机前读它的事实；`provision` 装完按它写画像；`maintenance::builds` 按平台名选运行包。

use crate::kernel::api::layout;
use crate::kernel::api::path_bridge::PathBridge;
use crate::kernel::ports::HostProbe;
use crate::kernel::ports::PathCodec;
use crate::units::business::sandbox::domain::guest::{GuestArch, GuestOs};
use crate::units::pure::exec_model::api::{Side, TargetSpec};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
/// 目的：guest 镜像里装的命令通道 agent（命令通道按它选实现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAgent {
    /// 目的：QEMU guest agent（QGA），走 virtio-serial。
    Qga,
}

/// 目的：guest 里具备哪些共享客户端——决定哪个共享后端能落上去（缺客户端就是缺，不假装）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestClients {
    /// 目的：9p2000.L 客户端（Linux 内核自带）。
    pub nine_p: bool,
    /// 目的：virtio-fs 客户端（Linux 内核 ≥5.4 自带）。
    pub virtio_fs: bool,
    /// 目的：CIFS 客户端（要镜像装 cifs-utils）。
    pub cifs: bool,
}

impl GuestClients {
    /// 目的：默认 Debian guest 具备的客户端（9p + virtio-fs；CIFS 要镜像显式装 cifs-utils）。
    pub const fn debian() -> GuestClients {
        GuestClients {
            nine_p: true,
            virtio_fs: true,
            cifs: false,
        }
    }
}

/// 目的：一份 guest 镜像（四轴里的 guest 轴）——磁盘镜像（可选内核 + initrd）+ agent 与共享客户端能力。
/// 约束：根**只能是磁盘镜像**：会话每次都叠一份每租约可写层，谁也不写基础镜像（见 docs/vm/README.md 的「多平台组合」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestImage {
    /// 目的：镜像名（诊断与日志用；默认取基础根目录名）。
    pub id: String,
    pub arch: GuestArch,
    /// 目的：guest 的 OS（路径桥按它选方言；缺省 linux）。
    pub os: GuestOs,
    /// 目的：基础磁盘镜像（qcow2 / raw ext4）：宿主只是普通文件 I/O，三平台与各引擎通用。
    pub disk: PathBuf,
    pub kernel: Option<PathBuf>,
    pub initrd: Option<PathBuf>,
    pub agent: GuestAgent,
    pub clients: GuestClients,
    /// 目的：guest 镜像**自带**的运行能力名（guest.yaml 的 `runtimes`，如 python / node）。
    /// 约束：虚拟机档的选型诊断把它并进包库一起看——镜像里已经有的解释器不必再放 `runtimes/` 包。缺省空表。
    pub runtimes: Vec<String>,
}

impl GuestImage {
    /// 目的：一份 Linux guest 镜像——默认 QGA 命令通道与 Debian 的共享客户端能力。
    pub fn linux(
        id: &str,
        disk: PathBuf,
        kernel: Option<PathBuf>,
        initrd: Option<PathBuf>,
    ) -> GuestImage {
        GuestImage {
            id: id.to_string(),
            arch: GuestArch::host(),
            os: GuestOs::Linux,
            disk,
            kernel,
            initrd,
            agent: GuestAgent::Qga,
            clients: GuestClients::debian(),
            runtimes: Vec::new(),
        }
    }

    /// 目的：这份镜像在本机（宿主）上的路径桥——宿主路径与 guest 路径的换算只此一座。
    pub fn bridge(&self) -> PathBridge {
        PathBridge::new(std::env::consts::OS, self.os.dialect())
    }
}
impl GuestClients {
    /// 目的：这份能力按 guest.yaml 认的写法列出来（9p / virtiofs / cifs，固定顺序）。
    pub fn names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.nine_p {
            out.push("9p");
        }
        if self.virtio_fs {
            out.push("virtiofs");
        }
        if self.cifs {
            out.push("cifs");
        }
        out
    }
}

/// 目的：guest 的平台名（os-arch，如 linux-x86_64）——运行包按它选分平台子目录。
pub fn guest_platform(os: GuestOs, arch: GuestArch) -> String {
    format!("{}-{}", os.as_str(), arch.as_str())
}

/// 目的：guest 清单（guest.yaml，可选）——用户自备镜像对世界的自我介绍；不认识的值如实拒收。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestManifest {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    arch: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    clients: Option<Vec<String>>,
    /// 目的：镜像自带的运行能力名（如 python / node）；进虚拟机档选型诊断。
    #[serde(default)]
    runtimes: Option<Vec<String>>,
}

/// 目的：读基础根目录里的 guest.yaml（可选）。
fn read_manifest(base: &Path, probe: &dyn HostProbe) -> Result<Option<GuestManifest>, String> {
    let path = layout::guest_spec(base);
    if !probe.is_file(&path) {
        return Ok(None);
    }
    let text = probe
        .read_text(&path)
        .ok_or_else(|| "guest.yaml 读不了".to_string())?;
    yaml_serde::from_str::<GuestManifest>(&text)
        .map(Some)
        .map_err(|e| format!("guest.yaml 非法（{}）", e))
}

/// 目的：把基础根目录判成一份可用的 guest 镜像（磁盘镜像 + 可选 vmlinuz/bzImage 与 initrd + 可选 guest.yaml）。
/// 返回：判定出来的镜像；不到一处可用的布局时给一句缺什么。
pub fn classify_base(base: &Path, probe: &dyn HostProbe) -> Result<GuestImage, String> {
    if !probe.is_dir(base) {
        return Err("基础根不在场（不是一个目录）".to_string());
    }
    let kernel = ["vmlinuz", "bzImage"]
        .iter()
        .map(|n| base.join(n))
        .find(|p| probe.is_file(p));
    let initrd = ["initrd.img", "initrd", "initramfs.img"]
        .iter()
        .map(|n| base.join(n))
        .find(|p| probe.is_file(p));
    // 虚拟根里必须有一份磁盘镜像（会话的可写层就叠在它上面）；没有就是没装好。
    let disk = [
        "rootfs.ext4",
        "rootfs.qcow2",
        "disk.qcow2",
        "disk.raw",
        "rootdisk.qcow2",
    ]
    .iter()
    .map(|n| base.join(n))
    .find(|p| probe.is_file(p))
    .ok_or_else(|| {
        "基础根里没有磁盘镜像（rootfs.ext4 / rootfs.qcow2 / disk.qcow2 / disk.raw / rootdisk.qcow2）：先跑一次补齐把虚拟根装好"
            .to_string()
    })?;
    let manifest = read_manifest(base, probe)?;
    let id = manifest
        .as_ref()
        .and_then(|m| m.id.clone())
        .unwrap_or_else(|| {
            base.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "guest".to_string())
        });
    let arch = match manifest.as_ref().and_then(|m| m.arch.as_deref()) {
        None => GuestArch::host(),
        Some("amd64") => GuestArch::Amd64,
        Some("arm64") => GuestArch::Arm64,
        Some(other) => {
            return Err(format!(
                "guest.yaml 里的 arch 不认识：{}（认 amd64 / arm64）",
                other
            ));
        }
    };
    let agent = match manifest.as_ref().and_then(|m| m.agent.as_deref()) {
        None => GuestAgent::Qga,
        Some("qga") => GuestAgent::Qga,
        Some(other) => {
            return Err(format!(
                "guest.yaml 里的 agent 不认识：{}（当前只认 qga）",
                other
            ));
        }
    };
    let clients = match manifest.as_ref().and_then(|m| m.clients.as_deref()) {
        None => GuestClients::debian(),
        Some(list) => {
            let mut c = GuestClients {
                nine_p: false,
                virtio_fs: false,
                cifs: false,
            };
            for name in list {
                match name.as_str() {
                    "9p" => c.nine_p = true,
                    "virtiofs" => c.virtio_fs = true,
                    "cifs" => c.cifs = true,
                    other => {
                        return Err(format!(
                            "guest.yaml 里的共享客户端不认识：{}（认 9p / virtiofs / cifs）",
                            other
                        ));
                    }
                }
            }
            c
        }
    };
    let os = match manifest.as_ref().and_then(|m| m.os.as_deref()) {
        None => GuestOs::Linux,
        Some(s) => GuestOs::parse(s)?,
    };
    let mut image = GuestImage::linux(&id, disk, kernel, initrd);
    image.arch = arch;
    image.os = os;
    image.agent = agent;
    image.clients = clients;
    image.runtimes = manifest
        .as_ref()
        .and_then(|m| m.runtimes.clone())
        .unwrap_or_default();
    image.runtimes.sort();
    image.runtimes.dedup();
    Ok(image)
}
/// 目的：按镜像事实重写 guest.yaml——客户端按这次编排确认的写，运行能力只列自证过的那些。
/// 参数：base = 虚拟根目录；image = 判定出来的镜像；runtimes = 这轮自证可用的能力名；clients = 这轮确认可用的共享客户端；
/// 错误：写不进去如实报（带内核给的原因）。
pub fn write_image(
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
    crate::kernel::api::fs::write(
        None,
        &layout::guest_spec(base),
        text.as_bytes(),
        crate::kernel::api::fs::Expect::Any,
    )
    .map(|_| ())
    .map_err(|fault| format!("写 guest.yaml 失败：{}", fault.message))
}

/// 目的：这次会话的 guest 镜像（本机档或选定虚拟根判不出来 = None）——平台名与 guest OS 都从它派生。
/// 约束：虚拟根名先经 `vmstore` 的 `resolve_guest` 解析成那一格目录，再按布局判定；两者任一失败都给 None。
pub fn guest_image_of(root: &Path, probe: &dyn HostProbe, spec: &TargetSpec) -> Option<GuestImage> {
    if spec.side != Side::Guest {
        return None;
    }
    let name = spec.guest.as_deref()?;
    let dir = crate::units::resource::vmstore::api::resolve_guest(root, name).ok()?;
    classify_base(&dir, probe).ok()
}

/// 目的：这次虚拟机档会话的 guest 平台名（os-arch）；本机档或判不出来给空串。
pub fn guest_platform_of(root: &Path, probe: &dyn HostProbe, spec: &TargetSpec) -> String {
    guest_image_of(root, probe, spec)
        .map(|image| guest_platform(image.os, image.arch))
        .unwrap_or_default()
}

/// 目的：这次虚拟机档会话的 guest OS（本机档或判不出来 = None）——工作环境块按它选 guest 档。
pub fn guest_os_of(root: &Path, probe: &dyn HostProbe, spec: &TargetSpec) -> Option<GuestOs> {
    guest_image_of(root, probe, spec).map(|image| image.os)
}

/// 目的：这场会话的路径翻译器（虚拟机档 guest↔宿主；本机档视图盘打开时宿主↔视图，否则 None = 恒等）。
/// 约束：它只把模型给的写法译成宿主路径**再送寻址**；安全判断仍在 `workbench` 的寻址里。
pub fn path_codec_of(
    root: &Path,
    probe: &dyn HostProbe,
    spec: &TargetSpec,
) -> Option<Arc<dyn PathCodec>> {
    let Some(guest) = guest_os_of(root, probe, spec) else {
        return crate::kernel::api::view_drive::view_codec();
    };
    Some(Arc::new(PathBridge::for_session(
        std::env::consts::OS,
        Some(guest.dialect()),
    )))
}

/// 目的：这份规格的 guest 镜像自带哪些运行能力（本机档或判不出来 = 空）。
pub fn guest_runtimes_of(root: &Path, probe: &dyn HostProbe, spec: &TargetSpec) -> Vec<String> {
    guest_image_of(root, probe, spec)
        .map(|image| image.runtimes)
        .unwrap_or_default()
}
