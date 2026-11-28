//! 目的：guest 的纯逻辑——镜像布局判定与挂载清单。
//! 管：一份 guest 镜像是不是可用、宿主根到 guest 的挂载清单。
//! 不管：进程的启停与命令通道（detail/ 的引擎与通道）、引擎参数（detail/ 的适配器）。
//! 联动：被 service.rs 与 detail/ 消费；契约见 docs/vm/README.md 与 RUNTIME_SPEC.md。

use crate::kernel::api::fence::{Access, FencePlan, Reach};
use crate::kernel::api::path_bridge::PathBridge;
use crate::units::business::catalog::packages::api::ExecPlan;
use crate::units::business::catalog::packages::api::{KIND_SYSTEM, PlannedPackage};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub use crate::units::business::sandbox::api::{GuestImage, GuestOs};

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
    /// 目的：构建期缓存卷（宿主上的 qcow2）——挂成第二块 virtio-blk 盘，由调用方在 guest 里挂载与对账。
    /// 约束：只有构建 guest 给；会话 guest 与供给编排为 None（它们的盘布局不受影响）。
    pub cache_disk: Option<PathBuf>,
}

/// 目的：一次 guest 执行要用的 per-agent 允许集（已是 guest 写法）。
/// 约束：只由 host 侧从 FencePlan 翻译而来；guest 内 helper 只认这份策略，失败一律 fail-closed。
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
    /// 目的：这次执行是不是**系统级只读诊断**：helper 在 Landlock restrict 之后走诊断模式（不 spawn 命令）。
    /// 约束：诊断是系统动作（不是 per-agent 围栏）——rw / private / cwd 都是空，只给只读根。
    pub diagnose: bool,
}

impl GuestAccess {
    /// 目的：把某 agent 的围栏翻成 guest 路径。挂载表优先，跨方言才走镜像根。
    pub fn from_fence(fence: &FencePlan, os: GuestOs, mounts: &[GuestMount]) -> GuestAccess {
        let map = |paths: &[PathBuf]| -> Vec<String> {
            paths.iter().map(|p| to_guest_path(p, os, mounts)).collect()
        };
        // guest 侧只认"读 / 读写 + 递归与否"这套老口径：host 侧在这里把意图摊平
        // （写授予 → rw，读授予按落点形状分到 ro_tree / ro；私有沙箱 → private）。
        let mut rw: Vec<PathBuf> = fence.paths(Access::Write, Reach::Tree);
        rw.extend(fence.paths(Access::Write, Reach::Dir));
        if !fence.home.as_os_str().is_empty() {
            rw.push(fence.home.clone());
        }
        rw.sort();
        rw.dedup();
        GuestAccess {
            agent: fence.agent.clone(),
            rw: map(&rw),
            ro: map(&fence.paths(Access::Read, Reach::Dir)),
            ro_tree: map(&fence.paths(Access::Read, Reach::Tree)),
            private: if fence.home.as_os_str().is_empty() {
                String::new()
            } else {
                to_guest_path(&fence.home, os, mounts)
            },
            cwd: if fence.cwd.as_os_str().is_empty() {
                String::new()
            } else {
                to_guest_path(&fence.cwd, os, mounts)
            },
            net: fence.net.on(),
            // per-agent 围栏永远不是诊断模式；诊断走 service.rs 自己构造的只读系统允许集。
            diagnose: false,
        }
    }
}

/// 目的：guest 执行输出的截断方式，同时决定回执形状：工具回执只留开头；shell 回执按 shell 契约装配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputLimit {
    /// 目的：只留开头（模块工具沿用）。
    Head,
    /// 目的：保留开头 1/4 与结尾 3/4（构建等内部脚本）。
    HeadAndTail,
    /// 目的：shell 回执——正文 head+tail 截断，且由通道产出与本机同形的状态行；cwd 与耗时由工具层追加。
    Shell,
}

/// 目的：在 guest 里跑一次工具（命令 + 参数 + 环境 + 工作目录 + 按次超时与输出截断）。
#[derive(Debug, Clone)]
pub struct GuestRun {
    pub command: String,
    pub args_json: String,
    pub env: Vec<(String, String)>,
    pub cwd: String,
    /// 目的：fenced 执行的超时秒数（helper 到点杀整棵进程树）；unfenced 供给编排忽略它。
    pub timeout_secs: u64,
    /// 目的：输出超限时的截断方式。
    pub output_limit: OutputLimit,
}

/// 目的：模块入口在 guest 本地副本里的落点（`/usr/local/lib/solomni/modules/<模块>/<入口>`）。
/// 约束：`/usr` 在 guest 内 helper 的只读基线里（含执行位），落这里不用给围栏加新根。
pub fn staged_entry_path(module: &str, entry: &str) -> String {
    let rel = entry.trim_start_matches("./").replace('\\', "/");
    format!(
        "/usr/local/lib/solomni/modules/{}/{}",
        safe_segment(module),
        rel
    )
}

/// 目的：把入口复制进 guest 本地副本的脚本——建目录、复制、给执行位，任何一步失败都非零退出。
pub fn stage_script(src: &str, dest: &str) -> String {
    let dir = dest.rsplit_once('/').map(|(d, _)| d).unwrap_or("/");
    format!(
        "mkdir -p {} && cp -f {} {} && chmod 0755 {}",
        sh_quote(dir),
        sh_quote(src),
        sh_quote(dest),
        sh_quote(dest),
    )
}

/// 目的：路径段的安全化（模块 id 只该是一段；这里再兜一层，不让分隔符进落点）。
fn safe_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
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
        timeout_secs: run.timeout_secs,
        output_limit: run.output_limit,
    }
}

/// 目的：宿主机上的某个文件在 guest 里的写法（挂载表优先）——落本地副本时要用它找源。
pub(crate) fn guest_path(host: &Path, os: GuestOs, mounts: &[GuestMount]) -> String {
    to_guest_path(host, os, mounts)
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
///   共享后端拼挂载 / 同步脚本也走它——外部工作区把宿主路径镜像进 guest 后，guest 路径可能含单引号。
pub(crate) fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 目的：把执行计划与围栏翻成 guest 挂载——prefix 包只读挂到 /<prefix>，围栏根按路径桥给的 guest 路径挂。
/// 约束：围栏根走 PathBridge（同方言时就是宿主原路径，模块命令与参数里的真实路径因此仍然成立；跨方言时是镜像根）；system 叠层不做。
pub fn mounts_from(plan: &ExecPlan, fence: &FencePlan, bridge: &PathBridge) -> Vec<GuestMount> {
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
    // 围栏根：私有沙箱与写授予可写挂，读授予只读挂（意图里的 Dir / Tree 在挂载上没有区别）。
    let mut roots: Vec<(&PathBuf, bool)> = Vec::with_capacity(fence.grants.len() + 1);
    if !fence.home.as_os_str().is_empty() {
        roots.push((&fence.home, false));
    }
    for grant in &fence.grants {
        roots.push((&grant.path, grant.access == Access::Read));
    }
    for (host, read_only) in roots {
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
    out
}
