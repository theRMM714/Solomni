//! 目的：guest 的纯逻辑——镜像布局判定与挂载清单。
//! 管：一份 guest 镜像是不是可用、宿主根到 guest 的挂载清单。
//! 不管：进程的启停与命令通道（detail/ 的引擎与通道）、引擎参数（detail/ 的适配器）。
//! 联动：被 service.rs 与 detail/ 消费；契约见 docs/vm/README.md 与 RUNTIME_SPEC.md。

use crate::capabilities::packages::api::{PlannedPackage, KIND_SYSTEM};
use crate::capabilities::vm::domain::exec::ExecPlan;
use crate::capabilities::vm::domain::paths::{PathBridge, PathDialect};
use crate::kernel::api::FenceSpec;
use crate::kernel::ports::HostProbe;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 目的：guest 的处理器架构（镜像架构要与宿主能虚拟化的架构一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestArch {
    Amd64,
    Arm64,
}

impl GuestArch {
    /// 目的：平台名里用的架构写法（amd64 → x86_64；arm64 → aarch64）。
    pub fn as_str(&self) -> &'static str {
        match self {
            GuestArch::Amd64 => "x86_64",
            GuestArch::Arm64 => "aarch64",
        }
    }

    /// 目的：本机能虚拟化的架构（x86_64 → amd64；aarch64 → arm64）。
    pub fn host() -> GuestArch {
        if cfg!(target_arch = "aarch64") {
            GuestArch::Arm64
        } else {
            GuestArch::Amd64
        }
    }
}

/// 目的：guest 的 OS——决定它认识的路径拼法（路径桥按它选方言），也是以后接非 Linux guest 的钥匙。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestOs {
    Linux,
    Macos,
    Windows,
}

impl GuestOs {
    /// 目的：这个 OS 的路径方言。
    pub fn dialect(&self) -> PathDialect {
        match self {
            GuestOs::Windows => PathDialect::Windows,
            GuestOs::Linux | GuestOs::Macos => PathDialect::Posix,
        }
    }

    /// 目的：平台名里用的 os 写法（也是 guest.yaml 的 os 取值与工作环境册 env_guests 的键）。
    pub fn as_str(&self) -> &'static str {
        match self {
            GuestOs::Linux => "linux",
            GuestOs::Macos => "macos",
            GuestOs::Windows => "windows",
        }
    }

    /// 目的：解析 guest.yaml 的 os；不认识如实拒收。
    pub fn parse(s: &str) -> Result<GuestOs, String> {
        match s {
            "linux" => Ok(GuestOs::Linux),
            "macos" => Ok(GuestOs::Macos),
            "windows" => Ok(GuestOs::Windows),
            other => Err(format!(
                "guest.yaml 里的 os 不认识：{}（认 linux / macos / windows）",
                other
            )),
        }
    }
}

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

/// 目的：guest 里的一处挂载（宿主路径 → guest 路径；只读或可写）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMount {
    pub host: PathBuf,
    pub guest: String,
    pub read_only: bool,
}

/// 目的：首启要经 cloud-init 投喂的种子文档（NoCloud net 数据源：`/user-data` 与 `/meta-data`）。
/// 约束：内容由调用方（供给编排）给，域内不生成；只被支持种子的引擎消费。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedDocs {
    pub user_data: String,
    pub meta_data: String,
}

/// 目的：一次 guest 启动的规格（镜像 + 挂载 + 网络 + 会话租约 + 引擎路径登记 + 首启种子）。
#[derive(Debug, Clone)]
pub struct GuestSpec {
    pub lease: String,
    pub image: GuestImage,
    pub mounts: Vec<GuestMount>,
    pub net: bool,
    /// 目的：设置里登记的**引擎**可执行文件路径（空 = 兜底按 PATH 找，名字由 arch 定）。
    /// 约束：与就绪检测共用同一份登记——检测说"按登记路径找到"，启动就必须走同一个路径。
    pub engine_path: Option<String>,
    /// 目的：设置里登记的**共享后端**要用的外部可执行文件路径（如 virtiofsd；空 = 兜底按 PATH 找）。
    /// 约束：与 qemu_path 同样与就绪检测共用一份登记；不需要外部工具的共享档忽略它。
    pub share_path: Option<String>,
    /// 目的：这次启动**可试**的共享后端 id，按优先序（第一个落得到这台引擎与这份 guest 上的就用它）。
    /// 约束：候选由调用方按"已实现 + 工具在场"排好（vm::api::share_chain）；空表 = 按 service 持有的顺序全试。
    ///   选档因此发生在起 guest 那一刻：补齐作业装完共享工具后，下一次启动就用上，不必重启进程。
    pub shares: Vec<&'static str>,
    /// 目的：首启要投喂的 cloud-init 种子；None = 不投种子（正常会话启动）。
    /// 约束：投种子要求 `net = true`（guest 要能连到宿主的小 HTTP 服务）。
    pub seed: Option<SeedDocs>,
    /// 目的：要不要给这次启动一份**每租约可写层**（会话要：多台并发不能同写一份基础镜像）。
    /// 约束：根永远是磁盘镜像，所以总建得了可写层；供给编排要改的正是基础镜像本身，填 false。
    pub isolated: bool,
}

/// 目的：一次 guest 执行要用的 per-agent 允许集（已是 guest 写法）。
/// 约束：只由 host 侧从 FenceSpec 翻译而来；guest 内 helper 只认这份策略，失败一律 fail-closed。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAccess {
    pub agent: String,
    pub rw: Vec<String>,
    pub ro: Vec<String>,
    pub ro_tree: Vec<String>,
    pub private: String,
    pub cwd: String,
    /// 目的：本次执行要不要保留出站网络；false = guest helper 里 unshare 一个空网络命名空间。
    pub net: bool,
}

impl GuestAccess {
    /// 目的：把某 agent 的围栏翻成 guest 路径。挂载表优先，跨方言才走镜像根。
    pub fn from_fence(fence: &FenceSpec, os: GuestOs, mounts: &[GuestMount]) -> GuestAccess {
        let map = |paths: &[PathBuf]| -> Vec<String> {
            paths.iter().map(|p| to_guest_path(p, os, mounts)).collect()
        };
        GuestAccess {
            agent: fence.agent.clone(),
            rw: map(&fence.rw),
            ro: map(&fence.ro),
            ro_tree: map(&fence.ro_tree),
            private: if fence.private.as_os_str().is_empty() {
                String::new()
            } else {
                to_guest_path(&fence.private, os, mounts)
            },
            cwd: if fence.cwd.as_os_str().is_empty() {
                String::new()
            } else {
                to_guest_path(&fence.cwd, os, mounts)
            },
            net: fence.net,
        }
    }
}

/// 目的：在 guest 里跑一次工具（命令 + 参数 + 环境 + 工作目录）。
#[derive(Debug, Clone)]
pub struct GuestRun {
    pub command: String,
    pub args_json: String,
    pub env: Vec<(String, String)>,
    pub cwd: String,
}

/// 目的：把一次 guest 运行的路径译成 guest 拼法——只译**工作目录**与**本机路径型环境变量**（同方言原样）。
/// 约束：命令与参数不动——模型看到的工作环境块给的就是 guest 拼法（见 prompts/shared/tools.yaml 的 envs）。
pub fn translate_run(run: &GuestRun, os: GuestOs, mounts: &[GuestMount]) -> GuestRun {
    GuestRun {
        command: run.command.clone(),
        args_json: run.args_json.clone(),
        env: run
            .env
            .iter()
            .map(|(k, v)| {
                let is_path = Path::new(v).is_absolute();
                (
                    k.clone(),
                    if is_path {
                        to_guest_path(Path::new(v), os, mounts)
                    } else {
                        v.clone()
                    },
                )
            })
            .collect(),
        cwd: if run.cwd.trim().is_empty() {
            run.cwd.clone()
        } else {
            to_guest_path(Path::new(&run.cwd), os, mounts)
        },
    }
}

/// 目的：一条宿主路径 → guest 路径——**先按挂载表**（包目录挂在 /<prefix>，围栏根挂在它的 guest 点），
/// 不在任何挂载里才用镜像根（跨方言）或原样（同方言）。
fn to_guest_path(host: &Path, os: GuestOs, mounts: &[GuestMount]) -> String {
    for m in mounts {
        if let Ok(rel) = host.strip_prefix(&m.host) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            let base = m.guest.trim_end_matches('/');
            return if rel.is_empty() {
                base.to_string()
            } else {
                format!("{}/{}", base, rel)
            };
        }
    }
    PathBridge::new(std::env::consts::OS, os.dialect()).to_guest(host)
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
    let path = base.join("guest.yaml");
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
            ))
        }
    };
    let agent = match manifest.as_ref().and_then(|m| m.agent.as_deref()) {
        None => GuestAgent::Qga,
        Some("qga") => GuestAgent::Qga,
        Some(other) => {
            return Err(format!(
                "guest.yaml 里的 agent 不认识：{}（当前只认 qga）",
                other
            ))
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
                        ))
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

/// 目的：system 类包在 guest 里的**只读载荷根**——`/<SYSTEM_PKG_MOUNT>/<包 id>`，叠写脚本从这里复制。
pub const SYSTEM_PKG_MOUNT: &str = "solomni-packages";

/// 目的：guest 里"这个包已叠写过"的标记根；标记在（且版本一致）就跳过复制。
const SYSTEM_PKG_STAMP_DIR: &str = "/var/lib/solomni/packages";

/// 目的：已装载包起 guest 后要跑的装配脚本——prefix 包把 `<prefix>/bin/*` 软链进 PATH，
///    system 包把只读载荷按 `provides_paths` **叠写**进系统路径（复制合并，不挂载覆盖）。
/// 参数：plan = 这次执行计划（包由包库扫描给出）。
/// 约束：system 包按"已叠标记"幂等（标记落在 guest 根文件系统里；每会话一份可写层，所以每会话付一次）；载荷里缺的路径由脚本如实跳过。
pub fn package_setup_scripts(plan: &ExecPlan) -> Vec<String> {
    let mut out = Vec::new();
    for p in &plan.packages {
        if p.kind == KIND_SYSTEM {
            if !p.provides_paths.is_empty() {
                out.push(system_apply_script(p));
            }
        } else if !p.prefix.trim().is_empty() {
            let dir = format!("/{}", p.prefix.trim_matches('/'));
            out.push(format!(
                "if [ -d {bin} ]; then ln -sf {bin}/* /usr/local/bin/ 2>/dev/null || true; fi",
                bin = sh_quote(&format!("{dir}/bin"))
            ));
        }
    }
    out
}

/// 目的：把 system 包的载荷叠写进 guest 系统路径（带"已叠"标记，失败不落标记）。
fn system_apply_script(p: &PlannedPackage) -> String {
    let src = format!("/{}/{}", SYSTEM_PKG_MOUNT, p.id);
    let stamp = format!("{}/{}@{}", SYSTEM_PKG_STAMP_DIR, p.id, p.version);
    let mut body = String::new();
    for rel in &p.provides_paths {
        let from = format!("{}/{}", src, rel);
        let to = format!("/{}", rel);
        // 目录：合并进去（`src/.` 不覆盖父目录本身）；文件：照写。缺的载荷如实跳过。
        body.push_str(&format!(
            "if [ -d {from} ]; then mkdir -p {to} && cp -a {from}/. {to}/ || exit 1; elif [ -e {from} ]; then mkdir -p \"$(dirname {to})\" && cp -a {from} {to} || exit 1; fi; ",
            from = sh_quote(&from),
            to = sh_quote(&to)
        ));
    }
    format!(
        "STAMP={stamp}; if [ ! -f \"$STAMP\" ]; then {body} mkdir -p {dir} && : > \"$STAMP\"; fi",
        stamp = sh_quote(&stamp),
        body = body,
        dir = sh_quote(SYSTEM_PKG_STAMP_DIR)
    )
}

/// 目的：把一段字面量安全地放进 guest 的 shell 脚本里（单引号包裹，内部单引号按 POSIX 拆合）。
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 目的：把执行计划与围栏翻成 guest 挂载——prefix 包只读挂到 /<prefix>，围栏根按路径桥给的 guest 路径挂。
/// 约束：围栏根走 PathBridge（同方言时就是宿主原路径，模块命令与参数里的真实路径因此仍然成立；跨方言时是镜像根）；system 叠层不做。
pub fn mounts_from(plan: &ExecPlan, fence: &FenceSpec, bridge: &PathBridge) -> Vec<GuestMount> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<GuestMount> = Vec::new();
    for p in &plan.packages {
        if p.source_dir.as_os_str().is_empty() {
            continue;
        }
        // prefix 包挂到它自己的前缀；system 包的载荷挂到专用只读根，由 guest 侧的叠写脚本复制进系统路径。
        let guest = if p.kind == KIND_SYSTEM {
            format!("/{}/{}", SYSTEM_PKG_MOUNT, p.id)
        } else {
            format!("/{}", p.prefix.trim_matches('/'))
        };
        if !seen.insert(guest.clone()) {
            continue;
        }
        out.push(GuestMount {
            host: p.source_dir.clone(),
            guest,
            read_only: true,
        });
    }
    let roots: [(&[PathBuf], bool); 3] = [
        (&fence.rw, false),
        (&fence.ro, true),
        (&fence.ro_tree, true),
    ];
    for (list, read_only) in roots {
        for host in list {
            let guest = bridge.to_guest(host);
            if !seen.insert(guest.clone()) {
                continue;
            }
            out.push(GuestMount {
                host: host.clone(),
                guest,
                read_only,
            });
        }
    }
    out
}
