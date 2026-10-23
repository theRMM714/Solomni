//! 目的：执行档位与执行计划的纯逻辑——档位诊断、计划派生、承载判定与就绪检测。
//! 管：ExecSpec 的档位诊断、ExecPlan 的装载顺序、本机能否承载虚拟机档。
//! 不管：包库与选包（packages）、目录遍历与 guest 启动（在 detail/ 与 guest 后端）、会话档案（session）、实际的工具进程执行（tools）。
//! 联动：契约见 docs/vm/README.md 与 RUNTIME_SPEC.md；ExecSpec 是 kernel 的共享事实；被 conductor 与呈现层经 api.rs 消费。

use crate::capabilities::modules::api::Module;
use crate::capabilities::packages::api::PlannedPackage;
use crate::capabilities::vm::domain::guest::{GuestAgent, GuestImage, GuestOs};
use crate::kernel::api::{ExecSpec, Tier};
use crate::kernel::ports::HostProbe;
use serde::Serialize;
use std::path::Path;

/// 目的：一条**模块入口**诊断：结构化事实（界面文案由呈现层渲染；模型侧文案在提示词册）。
/// 约束：包选择问题归 `packages::api::Problem`，不混进这里。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum EntryProblem {
    /// 模块工具的入口（模块内产物）不在场——命令是相对路径，模块根下却没有这个能直接跑的文件。
    MissingEntry {
        module: String,
        tool: String,
        command: String,
        /// 目的：这个模块声明了 `build:`（系统可以照它在目标环境里编出来）——拒绝理由据此指路。
        buildable: bool,
    },
}

/// 目的：一条"模块工具的入口不在场"的**事实**（谁、哪个工具、命令里写的那个入口）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingEntry {
    pub module: String,
    pub tool: String,
    /// 目的：命令里写的相对入口（如 `build/indexer`）。
    pub command: String,
    /// 目的：这个模块声明了 `build:`（见 MODULE_SPEC）——拒绝理由里就能给出"怎么编"的路。
    pub buildable: bool,
}

/// 目的：挑出"入口不在场"的模块工具——相对路径入口必须在**目标环境**里存在且能直接执行。
/// 参数：os = 目标系统（本机档 = 宿主，虚拟机档 = guest 的 os）；is_executable = "这个路径在目标环境里能不能直接跑"的事实。
/// 约束：裸名（`python` / `node` / `indexer`）走 PATH 与运行能力，不在本判据里；绝对路径同样不在模块目录里找。
/// 约束：名字按目标环境的壳解析（见 `entry_candidates`），跨平台时调用方还要按文件形式筛（见 `runs_on_target`）。
pub fn missing_entries_for(
    modules: &[Module],
    os: &str,
    is_executable: &dyn Fn(&Path) -> bool,
) -> Vec<MissingEntry> {
    let mut out = Vec::new();
    for m in modules {
        for (tool, decl) in &m.manifest.tools {
            let Some(entry) = crate::capabilities::modules::api::local_entry(&decl.command) else {
                continue;
            };
            // 壳怎么解析这个名字，就按它的候选去验（Windows 上 `build/indexer` 实际落在 `.exe`）。
            let found = crate::capabilities::modules::api::entry_candidates(entry, os)
                .iter()
                .any(|c| is_executable(&m.root.join(c)));
            if !found {
                out.push(MissingEntry {
                    module: m.manifest.id.clone(),
                    tool: tool.clone(),
                    command: entry.to_string(),
                    buildable: m.manifest.build.is_some(),
                });
            }
        }
    }
    out
}

/// 目的：执行计划：运行时派生，从不落盘（装配阶段按 (id, version) 去包里定位内容）。
#[derive(Debug, Clone)]
pub struct ExecPlan {
    pub tier: Tier,
    pub net: bool,
    /// 目的：这次执行用的虚拟根名（本机档为空）——装配阶段按它解析出那一格目录。
    pub guest: Option<String>,
    /// 目的：装载顺序：先独立前缀（互不干扰），后写进系统路径的包（叠层）。
    pub packages: Vec<PlannedPackage>,
}

/// 目的：执行档位的能力前置条件（本机档没有额外前置）。
/// 约束：`RUNTIME_SPEC.md` 把「选型」与「本机能不能承载」分开：定版/缺包/冲突是选型，这里是承载。
/// 约束：**逐项清单**（而不是几个布尔）：用户要看到的是"缺哪几项、每项怎么补"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierReadiness {
    /// 目的：虚拟机档的全部前置要求；本机档为空（没有额外前置）。
    pub requirements: Vec<VmRequirement>,
}

impl TierReadiness {
    pub fn ready(&self) -> bool {
        self.requirements.iter().all(|r| r.met)
    }

    /// 目的：缺哪几项（空 = 齐了）：呈现层按它如实说明，不猜。
    pub fn unmet(&self) -> Vec<&VmRequirement> {
        self.requirements.iter().filter(|r| !r.met).collect()
    }

    /// 目的：缺什么（每项一句"现状"）：沿用给日志与拒绝理由用。
    pub fn missing(&self) -> Vec<&str> {
        self.unmet().iter().map(|r| r.detail.as_str()).collect()
    }
}

/// 目的：虚拟机监视器不可用时该怎么提示（各平台如实说各自的前置条件）。
pub fn hypervisor_hint() -> &'static str {
    if cfg!(windows) {
        "本机虚拟机监视器不可用（要启用「虚拟机平台」组件）"
    } else if cfg!(target_os = "linux") {
        "本机虚拟机监视器不可用（要有 /dev/kvm）"
    } else if cfg!(target_os = "macos") {
        "本机虚拟机监视器不可用（需要 macOS 11 及以上）"
    } else {
        "本平台没有接入虚拟机档"
    }
}

/// 目的：虚拟机档的一项前置要求：**事实 + 怎么解决**（界面与 --doctor 照抄，不各自编话）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VmRequirement {
    /// 目的：要求名（机器可读的稳定短名，界面/日志按它分支）。
    pub id: &'static str,
    /// 目的：满没满足。
    pub met: bool,
    /// 目的：现状一句话（满足与不满足都要能读）。
    pub detail: String,
    /// 目的：没满足时怎么解决（满足了就是空）。
    pub how: String,
}

/// 目的：虚拟机档检测的入参：会话选型 + 设置里登记的路径（None = 没登记，兜底看 PATH）+ 宿主探测端口。
pub struct VmInputs<'a> {
    pub qemu: Option<&'a str>,
    /// 目的：设置里登记的**共享后端外部工具**路径（如 virtiofsd；None = 兜底看 PATH）。
    pub share_tool: Option<&'a str>,
    /// 目的：本机事实探测（读环境、查路径存在性都在它后面，见 ports::HostProbe）。
    pub probe: &'a dyn HostProbe,
}

/// QEMU 可执行文件名（各平台同名；平台扩展名由适配层的探测实现处理）。
const QEMU_BIN: &str = "qemu-system-x86_64";

/// 目的：虚拟机档的**机器能力**清单（**只读事实，不起任何虚拟机**）——与某条会话无关。
/// 约束：为什么做成清单：用户看到的必须是"缺哪几项、每项怎么补"，而不是一句笼统的"前置条件不具备"。
/// 约束：虚拟根属于**会话选型**，不在这里；含虚拟根的严格判据见 tier_readiness。
pub fn vm_requirements(vm: &VmInputs<'_>) -> Vec<VmRequirement> {
    let hyper = vm.probe.hypervisor_available();
    let combo = effective_combo_label(vm.share_tool, vm.probe);
    vec![
        VmRequirement {
            id: "hypervisor",
            met: hyper,
            detail: if hyper {
                format!("本机虚拟机监视器可用（{}）", hypervisor_kind())
            } else {
                hypervisor_hint().to_string()
            },
            how: if hyper {
                String::new()
            } else {
                hypervisor_how().to_string()
            },
        },
        // guest 本体：产品侧的后端与执行分派已就位；**逐平台**真机验收通过后才放开。
        // 约束：这是"验过没有"的事实，不是环境探测——环境项是上面的 hypervisor / qemu / 虚拟根。
        VmRequirement {
            id: "guest",
            met: guest_backend_verified(),
            detail: if guest_backend_verified() {
                if copy_share_enabled() {
                    "guest 后端已就位，本平台已通过真机验收（当前共享档为开发/应急 copy）"
                        .to_string()
                } else if combo == "未定" {
                    "guest 后端已就位，本平台已通过真机验收；用户共享档当前不可用".to_string()
                } else {
                    format!("guest 后端已就位，本平台默认组合 {} 已通过真机验收", combo)
                }
            } else if copy_share_enabled() {
                "guest 后端已就位，但本平台尚未真机验收；当前共享档为开发/应急 copy（就绪项判否）"
                    .to_string()
            } else if combo == "未定" {
                "guest 后端已就位，但本平台尚未真机验收（就绪项判否）".to_string()
            } else {
                format!(
                    "guest 后端已就位，但本平台默认组合 {} 尚未真机验收（就绪项判否）",
                    combo
                )
            },
            how: if guest_backend_verified() {
                String::new()
            } else if copy_share_enabled() || combo == "未定" {
                "在该平台真机上跑通一次 vm 档工具后再放开（现状与验收见 tests/macos/gaps.yaml）"
                    .to_string()
            } else {
                format!(
                    "在该平台真机上跑通一次 vm 档工具（默认组合 {}）后再放开（现状与验收见 tests/macos/gaps.yaml）",
                    combo
                )
            },
        },
        // 共享档：**硬前置**——用户链为空就判否；copy 只在开发/应急开关下出现。
        share_requirement(vm.share_tool, vm.probe),
        // QEMU：不预置、不分发（可由 setup 脚本装进项目内）。没登记就看 PATH——检测那件东西在不在。
        qemu_requirement(vm.qemu, vm.probe),
    ]
}

/// 本机虚拟机监视器的类型（如实说清是哪一种，不笼统说"虚拟化可用"）。
fn hypervisor_kind() -> &'static str {
    if cfg!(windows) {
        "Windows 虚拟机平台"
    } else if cfg!(target_os = "linux") {
        "Linux KVM（/dev/kvm）"
    } else if cfg!(target_os = "macos") {
        "macOS 虚拟化框架"
    } else {
        "未知"
    }
}

/// 目的：已通过真机验收的宿主平台（**数据，不是平台 cfg**）——新平台验收后在这里加一行，并同步对应平台的缺口账（tests/<平台>/gaps.yaml）。
const VERIFIED_HOSTS: &[&str] = &["linux", "windows"];

/// 目的：给定宿主平台名（std::env::consts::OS），这个平台的 guest 后端是否已真机验收。
pub fn host_verified(host: &str) -> bool {
    VERIFIED_HOSTS.contains(&host)
}

/// 目的：一个可选的后端组合（引擎 + 共享后端）——按平台排成回退链，前面的排前。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub engine: &'static str,
    pub share: &'static str,
}

const LINUX_COMBOS: &[Combo] = &[
    Combo {
        engine: "qemu",
        share: "virtiofs",
    },
    Combo {
        engine: "qemu",
        share: "9p",
    },
];
const MACOS_COMBOS: &[Combo] = &[
    Combo {
        engine: "vz",
        share: "virtiofs",
    },
    Combo {
        engine: "qemu",
        share: "9p",
    },
];
const WINDOWS_COMBOS: &[Combo] = &[Combo {
    engine: "qemu",
    share: "smb",
}];
/// 未列出的平台没有已实现的用户共享档——虚拟机档如实不可用，不拿 emergency 档兜底。
const FALLBACK_COMBOS: &[Combo] = &[];

/// 目的：某个宿主平台上按"性能优先"排好的组合顺序（回退在后面）；未列出的平台用兜底表。
pub fn default_combos(host: &str) -> &'static [Combo] {
    match host {
        "linux" => LINUX_COMBOS,
        "macos" => MACOS_COMBOS,
        "windows" => WINDOWS_COMBOS,
        _ => FALLBACK_COMBOS,
    }
}

/// 目的：guest 的平台名（os-arch，如 linux-x86_64）——运行包按它选分平台子目录。
pub fn guest_platform(
    os: crate::capabilities::vm::domain::guest::GuestOs,
    arch: crate::capabilities::vm::domain::guest::GuestArch,
) -> String {
    format!("{}-{}", os.as_str(), arch.as_str())
}

/// 目的：本平台的 guest 后端是否已经过真机验收。
/// 约束：记录的是"验过没有"的事实，不是环境探测。
fn guest_backend_verified() -> bool {
    host_verified(std::env::consts::OS)
}

/// 目的：用户可选的共享后端（组合选择与就绪说明**共用同一份**，避免"说要验 A、实际跑 B"）。
/// 约束：copy 不在此列——它是开发/应急通道，只有进程级隐藏开关开启时才参与选档。
pub const USER_SHARES: &[&str] = &["9p", "virtiofs", "smb"];

/// 目的：进程级开发/应急开关的环境变量；取值为 `copy`（不区分大小写）时才启用 copy。
pub const COPY_SHARE_ENV: &str = "SOLOMNI_VM_SHARE";

/// 目的：这个进程是否显式打开了 copy 应急通道。
/// 约束：只在入口层读一次；普通用户进程不设置它，copy 因此不装配、不参与选档。
pub fn copy_share_enabled() -> bool {
    let raw = std::env::var(COPY_SHARE_ENV).ok();
    copy_share_setting(raw.as_deref())
}

/// 目的：把环境变量的原始值翻成"开没开"（纯函数，测试不用改进程环境）。
pub(crate) fn copy_share_setting(raw: Option<&str>) -> bool {
    raw.map(|v| v.trim().eq_ignore_ascii_case("copy"))
        .unwrap_or(false)
}

/// 目的：共享档 virtiofs 要用的外部守护进程名（探测 PATH 与设置登记都用它；只支持 Rust 版 CLI）。
pub const VIRTIOFSD_BIN: &str = "virtiofsd";

/// 目的：共享档 smb 要用的外部 helper 名（探测 PATH 与设置登记都用它；Windows 上补 .exe 由宿主探测处理）。
pub const SMBD_BIN: &str = "smbd";

/// 目的：某个共享档要用的外部工具名（None = 不需要外部工具）。
pub fn share_tool_of(share: &str) -> Option<&'static str> {
    match share {
        "virtiofs" => Some(VIRTIOFSD_BIN),
        "smb" => Some(SMBD_BIN),
        _ => None,
    }
}

/// 目的：这个共享档能不能承载这台引擎上的这份 guest（纯事实）。
/// 约束：适配器的 `supports` 与起 guest 前的就绪判据共用这一份，避免"说 A 跑 B"。
pub fn share_carries(share: &str, engine: &str, guest: &GuestImage) -> bool {
    match share {
        "virtiofs" => engine == "qemu" && guest.clients.virtio_fs,
        "9p" => engine == "qemu" && guest.clients.nine_p,
        "smb" => engine == "qemu" && guest.clients.cifs,
        // copy 是开发/应急通道：脚本是 POSIX（/bin/sh、/tmp、tar -C），只对 Linux guest 成立；
        // 它不参与用户链，只有 SOLOMNI_VM_SHARE=copy 时才进候选。
        "copy" => {
            engine == "qemu" && matches!(guest.agent, GuestAgent::Qga) && guest.os == GuestOs::Linux
        }
        _ => false,
    }
}

/// 目的：一个外部工具在不在场——登记路径优先（登记了就必须是那个文件），否则看 PATH。
fn tool_available(name: &str, registered: Option<&str>, probe: &dyn HostProbe) -> bool {
    match registered.map(str::trim) {
        // 登记了就以登记的为准，而且得**能直接执行**——只在场（比如权限位丢了）不算可用，
        // 否则组合会选它、到启动那一步才炸；这里如实判"不在场"，让就绪文案说真话。
        Some(p) if !p.is_empty() => probe.is_executable(Path::new(p)),
        _ => probe.has_exe(name),
    }
}

/// 目的：这次启动**可试**的用户共享后端 id，按本平台"性能优先"的顺序；工具不在场的档不列。
/// 参数：host = 宿主平台（std::env::consts::OS 的取值）；share_tool = 本平台共享工具路径（None = 兜底 PATH）；probe = 宿主事实。
/// 约束：只判"用户档 + 工具在场"（share_available）；copy 不在用户链里，由 `copy_share_enabled` 单独决定。
/// 约束：落不落到这台引擎 / 这份 guest 上由 `share_carries` 判——选档发生在起 guest 那一刻，不是装配期定一次。
/// 约束：引擎恒按 qemu 筛（当前唯一已实现引擎）；返回空表 = 本平台没有可试的用户共享档。
pub fn share_chain(
    host: &str,
    share_tool: Option<&str>,
    probe: &dyn HostProbe,
) -> Vec<&'static str> {
    default_combos(host)
        .iter()
        .filter(|c| c.engine == "qemu" && share_available(c.share, share_tool, probe))
        .map(|c| c.share)
        .collect()
}

/// 目的：这个文件在**目标平台**上能直接跑吗——看形式（ELF / Mach-O / PE / 脚本头 / Windows 的批处理扩展名）。
/// 参数：path = 模块目录里的入口文件；target_os = 目标平台（`std::env::consts::OS` 的取值）。
/// 约束：名字层面的解析由 `entry_candidates` 管，这里只管**内容**；跨平台时"宿主编出来的产物"与目标格式不符，
///   必须在预检就判否，而不是等 guest 里报 `not found` / `Exec format error`。
pub fn runs_on_target(path: &Path, target_os: &str) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 4];
    let n = f.read(&mut head).unwrap_or(0);
    let head = &head[..n];
    let elf = head == b"\x7fELF";
    let pe = head.starts_with(b"MZ");
    let script = head.starts_with(b"#!");
    let mach = matches!(
        head,
        [0xfe, 0xed, 0xfa, 0xce]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
    );
    match target_os {
        // Windows 的壳按扩展名认批处理与命令脚本；可执行体是 PE。
        "windows" => {
            pe || script
                || path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| matches!(e.to_ascii_lowercase().as_str(), "cmd" | "bat" | "com"))
                    .unwrap_or(false)
        }
        "macos" => mach || script,
        _ => elf || script,
    }
}

/// 目的：某个用户共享档在不在场：先要是用户档，再要它要的外部工具在场（不需要工具的档恒在场）。
/// 约束：组合选择（组合根）与就绪说明共用这一份，避免"说要验 A、实际跑 B"；copy 不在用户档里。
pub fn share_available(share: &str, share_tool: Option<&str>, probe: &dyn HostProbe) -> bool {
    if !USER_SHARES.contains(&share) {
        return false;
    }
    match share_tool_of(share) {
        None => true,
        Some(name) => tool_available(name, share_tool, probe),
    }
}

/// 目的：本平台用户链**实际会选**的组合标签（"引擎/共享"）；开发/应急开关打开时标出 copy。
fn effective_combo_label(share_tool: Option<&str>, probe: &dyn HostProbe) -> String {
    if copy_share_enabled() {
        return "qemu/copy（应急）".to_string();
    }
    share_chain(std::env::consts::OS, share_tool, probe)
        .first()
        .map(|share| format!("qemu/{share}"))
        .unwrap_or_else(|| "未定".to_string())
}

/// 目的：共享档的现状——本平台用户链有没有能用的挂载式后端；没有就如实判否。
/// 约束：这是**硬前置**，不再回退 copy；Linux 缺 virtiofsd 时 9p 仍算可用。
///   "怎么补"写在 how 里；copy 只由开发/应急开关启用，并在 detail 里明确标出。
fn share_requirement(share_tool: Option<&str>, probe: &dyn HostProbe) -> VmRequirement {
    if copy_share_enabled() {
        return VmRequirement {
            id: "share",
            met: true,
            detail: format!(
                "共享档 copy 已由开发/应急开关启用（{}={}）：性能不可用，不参与用户档",
                COPY_SHARE_ENV, "copy"
            ),
            how: String::new(),
        };
    }
    let chain = share_chain(std::env::consts::OS, share_tool, probe);
    let top = top_user_combo();
    if chain.is_empty() {
        let (detail, how) = match top {
            None => (
                "本平台还没有已实现的用户共享档".to_string(),
                "换本机档；虚拟机档需要挂载式共享（Linux 9p/virtiofs、Windows smb）".to_string(),
            ),
            Some(c) => {
                let detail = match share_tool_of(c.share) {
                    None => format!("共享档 {} 不可用：本平台用户链为空", c.share),
                    Some(name) => match share_tool.map(str::trim) {
                        Some(p)
                            if !p.is_empty()
                                && probe.is_file(Path::new(p))
                                && !probe.is_executable(Path::new(p)) =>
                        {
                            format!(
                                "共享档 {} 登记的那个不能执行（{}）：虚拟机档不可用",
                                c.share, p
                            )
                        }
                        _ => format!(
                            "共享档 {} 不在场（没找到 {}）：虚拟机档不可用",
                            c.share, name
                        ),
                    },
                };
                (detail, share_how(c.share))
            }
        };
        return VmRequirement {
            id: "share",
            met: false,
            detail,
            how,
        };
    }
    let best = chain[0];
    let detail = match top {
        Some(c) if c.share == best => match share_tool_of(best) {
            Some(name) => format!("共享档 {} 可用（找到 {}）", best, name),
            None => format!("共享档 {}（不需要外部工具）", best),
        },
        Some(c) => match share_tool_of(c.share) {
            Some(name) => match share_tool.map(str::trim) {
                Some(p)
                    if !p.is_empty()
                        && probe.is_file(Path::new(p))
                        && !probe.is_executable(Path::new(p)) =>
                {
                    format!(
                        "共享档 {} 登记的那个不能执行（{}）：使用 {}；给它执行位，或换成 PATH 上的 {} 即可启用",
                        c.share, p, best, name
                    )
                }
                _ => format!(
                    "共享档 {} 不在场（没找到 {}）：使用 {}；把 {} 放进 PATH，或在设置里登记它的完整路径即可启用",
                    c.share, name, best, name
                ),
            },
            None => format!("共享档 {}（不需要外部工具）", best),
        },
        None => format!("共享档 {} 可用", best),
    };
    VmRequirement {
        id: "share",
        met: true,
        detail,
        how: String::new(),
    }
}

/// 目的：本平台用户链里性能最高的那一档（copy 不在其中）。
fn top_user_combo() -> Option<&'static Combo> {
    default_combos(std::env::consts::OS)
        .iter()
        .find(|c| c.engine == "qemu" && USER_SHARES.contains(&c.share))
}

/// 目的：用户共享档缺失时怎么补（按平台给可操作的说法，不说"回退 copy"）。
fn share_how(share: &str) -> String {
    match share {
        "smb" => "点「补齐虚拟机依赖」把 smbd 装进项目内（只写项目内，需用户同意），或自行安装 smbd 并放进 PATH / 在设置里登记完整路径；集成环境可用 solomni --provision --ensure --yes。".to_string(),
        "virtiofs" => "点「补齐虚拟机依赖」把 virtiofsd 装进项目内（只写项目内，需用户同意），或自行安装 virtiofsd 并放进 PATH / 在设置里登记完整路径。Linux 没有 virtiofsd 时仍可用 9p；9p 也不可用才禁用 VM 档。".to_string(),
        _ => "点「补齐虚拟机依赖」把所需的共享档装进项目内（只写项目内，需用户同意），或换本机档。".to_string(),
    }
}

/// 虚拟机监视器不可用时的解决指引（各平台说各自的办法）。
fn hypervisor_how() -> &'static str {
    if cfg!(windows) {
        "启用「虚拟机平台」组件（设置 → 应用 → 可选功能；或管理员 PowerShell 执行 dism /online /enable-feature /featurename:VirtualMachinePlatform /all），然后重启"
    } else if cfg!(target_os = "linux") {
        "确认 CPU 虚拟化已在 BIOS/UEFI 打开，且宿主内核提供 /dev/kvm（ls -l /dev/kvm 确认；容器里通常要 --device /dev/kvm）"
    } else if cfg!(target_os = "macos") {
        "升级到 macOS 11 及以上（本机不需要额外安装）"
    } else {
        "本平台未接入虚拟机档"
    }
}

/// QEMU 检测：登记了就用登记的路径，没登记就看 PATH（依赖不预置、不分发，可由 setup 脚本装进项目内）。
fn qemu_requirement(registered: Option<&str>, probe: &dyn HostProbe) -> VmRequirement {
    // "登记的路径" 也包含核心在没登记时自动采用的那一份（项目内补齐的包）：对用户是同一件事——按路径找到了。
    let (found, where_from) = match registered.map(str::trim) {
        Some(p) if !p.is_empty() => (probe.is_file(Path::new(p)), "设置登记或项目内补齐的路径"),
        _ => (probe.has_exe(QEMU_BIN), "PATH"),
    };
    VmRequirement {
        id: "qemu",
        met: found,
        detail: if found {
            format!("QEMU 可用（按{}找到 {}）", where_from, QEMU_BIN)
        } else {
            format!("没有找到 {}（按{}找过）", QEMU_BIN, where_from)
        },
        how: if found {
            String::new()
        } else {
            "在创建向导或配置界面点「补齐虚拟机依赖」把它下载进项目内（只写项目内，需用户同意）；也可自行安装 QEMU 并放进 PATH，或在设置里登记它的完整路径。集成环境里没有界面时可用 solomni --provision --ensure --yes"
                .to_string()
        },
    }
}

/// 目的：虚拟根的安装父目录（相对产品根）——清单里每条 guest 条目的 `install` 都落在它下面。
const GUEST_ROOT: &str = ".vm/guest";

/// 目的：虚拟根名 → 那一格目录的绝对路径（产品根 + `.vm/guest/<名>`）。
/// 约束：名字必须是单纯的一段（不许路径分隔符与 `.` / `..`）——它来自清单，也是会话里存的值。
pub fn resolve_guest(root: &Path, guest: &str) -> Result<std::path::PathBuf, String> {
    let name = guest.trim();
    if name.is_empty() {
        return Err("没有指定虚拟根".to_string());
    }
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(format!(
            "虚拟根名不合法：{}（应是清单里的一个名字，不是路径）",
            guest
        ));
    }
    let mut dir = root.to_path_buf();
    for seg in GUEST_ROOT.split('/').filter(|s| !s.is_empty()) {
        dir.push(seg);
    }
    Ok(dir.join(name))
}

/// 目的：虚拟根就绪检测——名字认得、那一格目录在场、布局可用（rootfs + 内核或磁盘镜像）。
/// 约束：检测顺序 = 名字合法性 → 在场 → 布局；三种失败给三种可操作的说明，不混成一句。
fn guest_requirement(root: &Path, guest: Option<&str>, probe: &dyn HostProbe) -> VmRequirement {
    let how = "在创建向导或配置界面点「补齐虚拟机依赖」把虚拟根装进项目内（只写项目内，需用户同意），或换一个已装好的虚拟根；集成环境里没有界面时可用 solomni --provision --ensure --yes";
    let name = guest.map(str::trim).filter(|g| !g.is_empty());
    let Some(name) = name else {
        return VmRequirement {
            id: "base",
            met: false,
            detail: "没有选定虚拟根".to_string(),
            how: how.to_string(),
        };
    };
    let dir = match resolve_guest(root, name) {
        Ok(d) => d,
        Err(why) => {
            return VmRequirement {
                id: "base",
                met: false,
                detail: why,
                how: how.to_string(),
            }
        }
    };
    if !probe.is_dir(&dir) {
        return VmRequirement {
            id: "base",
            met: false,
            detail: format!("虚拟根「{}」不在场：{}", name, dir.display()),
            how: how.to_string(),
        };
    }
    match crate::capabilities::vm::domain::guest::classify_base(&dir, probe) {
        Ok(b) => {
            let root_kind = "磁盘镜像 rootfs";
            VmRequirement {
                id: "base",
                met: true,
                detail: format!(
                    "虚拟根「{}」已就位（{}，{}，内核 {}{}）",
                    name,
                    b.id,
                    root_kind,
                    b.kernel
                        .as_deref()
                        .and_then(|k| k.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "自带引导".to_string()),
                    if b.initrd.is_some() { " + initrd" } else { "" }
                ),
                how: String::new(),
            }
        }
        Err(why) => VmRequirement {
            id: "base",
            met: false,
            detail: format!("虚拟根「{}」不可用：{}", name, why),
            how: how.to_string(),
        },
    }
}

/// 目的：会话级共享判据——这份 guest 的客户端里，有没有本平台用户链能承载的一档。
/// 返回 None = 虚拟根不在场/布局不可用（由 guest 项报错）；不重复报同一件事。
fn guest_share_requirement(
    root: &Path,
    guest: Option<&str>,
    share_tool: Option<&str>,
    probe: &dyn HostProbe,
) -> Option<VmRequirement> {
    let name = guest.map(str::trim).filter(|g| !g.is_empty())?;
    let dir = resolve_guest(root, name).ok()?;
    if !probe.is_dir(&dir) {
        return None;
    }
    let image = crate::capabilities::vm::domain::guest::classify_base(&dir, probe).ok()?;
    let chain: Vec<&'static str> = if copy_share_enabled() {
        vec!["copy"]
    } else {
        share_chain(std::env::consts::OS, share_tool, probe)
    };
    let usable: Vec<&'static str> = chain
        .into_iter()
        .filter(|share| share_carries(share, "qemu", &image))
        .collect();
    if usable.is_empty() {
        let mut declared: Vec<&str> = Vec::new();
        if image.clients.nine_p {
            declared.push("9p");
        }
        if image.clients.virtio_fs {
            declared.push("virtiofs");
        }
        if image.clients.cifs {
            declared.push("cifs");
        }
        let declared = if declared.is_empty() {
            "未声明任何共享客户端".to_string()
        } else {
            declared.join("、")
        };
        return Some(VmRequirement {
            id: "share-guest",
            met: false,
            detail: format!(
                "这份 guest 没有本平台用户共享档能承载的客户端（已声明：{}）；虚拟机档无法挂载工作区",
                declared
            ),
            how: if copy_share_enabled() {
                "开发/应急 copy 只支持 Linux guest；换 Linux 虚拟根，或关掉 SOLOMNI_VM_SHARE=copy 后用用户档。"
                    .to_string()
            } else {
                "在 guest.yaml 的 clients 里声明 9p 或 virtiofs（Windows 宿主用 cifs），或换默认虚拟根。"
                    .to_string()
            },
        });
    }
    Some(VmRequirement {
        id: "share-guest",
        met: true,
        detail: format!("guest 可用共享档：{}", usable.join("、")),
        how: String::new(),
    })
}

/// 目的：机器能力的就绪视图（不含会话级虚拟根）：界面据此决定虚拟机档能不能**选**。
pub fn vm_capability(vm: &VmInputs<'_>) -> TierReadiness {
    TierReadiness {
        requirements: vm_requirements(vm),
    }
}

/// 目的：机器能力不足时的可读拒绝理由（与 tier_refusal 同一套文案，判据只到机器能力）。
pub fn capability_refusal(vm: &VmInputs<'_>) -> Option<String> {
    let cap = vm_capability(vm);
    if cap.ready() {
        return None;
    }
    Some(format!(
        "虚拟机档现在不可用（{}）：请换本机档，或在创建向导 / 配置界面的「虚拟机档前置」里点「补齐虚拟机依赖」把缺的装进项目内",
        cap.missing().join("；")
    ))
}

/// 目的：本机能不能承载这个档位（**只读事实，不碰任何东西**）。
/// 约束：虚拟机档的前置条件不具备时，虚拟机档**不允许创建或改入**——这是用户环境问题，不是选型问题；
/// 约束：界面的"能不能点"与「开始」的校验走同一个函数，两处不会各说各话。
/// 约束：判据**只有一份**：机器能力清单（vm_requirements）+ 这条会话选的虚拟根；逐项都满足才算承载得了。
pub fn tier_readiness(
    root: &Path,
    spec: &ExecSpec,
    qemu: Option<&str>,
    share_tool: Option<&str>,
    probe: &dyn HostProbe,
) -> TierReadiness {
    if spec.tier != Tier::Vm {
        return TierReadiness {
            requirements: Vec::new(),
        };
    }
    let mut requirements = vm_requirements(&VmInputs {
        qemu,
        share_tool,
        probe,
    });
    // 虚拟根是这条会话的选型，不是机器能力：保存 /「开始」时才要求它。
    requirements.push(guest_requirement(root, spec.guest.as_deref(), probe));
    // 共享档也按这台 guest 再判一次：用户链里有档可用，不代表这份 guest 声明了对应客户端。
    if let Some(share) = guest_share_requirement(root, spec.guest.as_deref(), share_tool, probe) {
        requirements.push(share);
    }
    TierReadiness { requirements }
}

/// 目的：虚拟机档的可读拒绝理由（创建与编辑共用同一把尺子）。
pub fn tier_refusal(
    root: &Path,
    spec: &ExecSpec,
    qemu: Option<&str>,
    share_tool: Option<&str>,
    probe: &dyn HostProbe,
) -> Option<String> {
    let readiness = tier_readiness(root, spec, qemu, share_tool, probe);
    if readiness.ready() {
        return None;
    }
    Some(format!(
        "虚拟机档现在不可用（{}）：请换本机档，或在创建向导 / 配置界面的「虚拟机档前置」里点「补齐虚拟机依赖」把缺的装进项目内",
        readiness.missing().join("；")
    ))
}

/// 目的：虚拟机档的并发准入——每台 guest 都是实打实的资源（固定 `-m 2048 -smp 2`），
///   到上限就如实拒绝；**同一租约重启不占新名额**；上限 0 = 不限制。
pub fn admit_guest(running: &[String], lease: &str, max: u32) -> Result<(), String> {
    if max == 0 || running.iter().any(|l| l == lease) {
        return Ok(());
    }
    if running.len() >= max as usize {
        return Err(format!(
            "虚拟机档同时最多 {} 台（现在 {} 台）：先删掉、或把别的虚拟机档会话切回本机档",
            max,
            running.len()
        ));
    }
    Ok(())
}

/// 目的：派生执行计划：入口判据与档位无关；运行包由 packages 能力选好，这里只负责落进 ExecPlan。
/// 约束：本机档的 `packages` 为空（运行包由 packages 注入本次 PATH，不进 guest 装配）；虚拟机档的包选择与诊断在 `packages::api::plan_guest`。
pub fn plan(
    spec: &ExecSpec,
    packages: Vec<PlannedPackage>,
    missing: &[MissingEntry],
    net: bool,
) -> Result<ExecPlan, Vec<EntryProblem>> {
    // 入口判据与档位无关：模块根下没有那个能直接跑的产物，两档都当场拒绝（一次把话说全）。
    let entries: Vec<EntryProblem> = missing
        .iter()
        .map(|m| EntryProblem::MissingEntry {
            module: m.module.clone(),
            tool: m.tool.clone(),
            command: m.command.clone(),
            buildable: m.buildable,
        })
        .collect();
    if !entries.is_empty() {
        return Err(entries);
    }
    match spec.tier {
        Tier::Host => Ok(ExecPlan {
            tier: Tier::Host,
            net,
            guest: None,
            packages: Vec::new(),
        }),
        Tier::Vm => Ok(ExecPlan {
            tier: Tier::Vm,
            net,
            guest: spec.guest.clone(),
            packages,
        }),
    }
}

/// 目的：同上，但"怎么补"按**目标环境**说：虚拟机档的入口要在那个虚拟根里编（宿主上编出来的产物在 guest 里跑不了）。
/// 参数：guest = 这次会话的虚拟根名（None = 本机档，在宿主编）。
pub fn diagnose_text_for(diags: &[EntryProblem], guest: Option<&str>) -> String {
    let lines: Vec<String> = diags
        .iter()
        .map(|d| match d {
            EntryProblem::MissingEntry {
                module,
                tool,
                command,
                buildable,
            } => {
                let how = match (buildable, guest) {
                    (true, Some(g)) => format!(
                        "跑 solomni --build={} --guest={} 让系统在虚拟根里照模块声明编一次（宿主上编出来的产物在 guest 里跑不了）",
                        module, g
                    ),
                    (true, None) => {
                        format!("跑 solomni --build={} 让系统照模块声明的构建方法编一次", module)
                    }
                    (false, _) => {
                        "模块要自带这个产物，或先按模块自己的 README 构建一次".to_string()
                    }
                };
                format!("模块 {} 的工具 {} 入口不在场：{}（{}）", module, tool, command, how)
            }
        })
        .collect();
    lines.join("；")
}

/// 目的：计划的一句话摘要（日志用）：只报事实，不含真实路径。
pub fn plan_summary(plan: &ExecPlan) -> String {
    let net = if plan.net { "放行" } else { "不放行" };
    if plan.tier == Tier::Host {
        return format!("执行档位 本机（不装载运行包）；网络 {}", net);
    }
    let pkgs: Vec<String> = plan
        .packages
        .iter()
        .map(|p| format!("{}@{}（{}）", p.id, p.version, p.kind))
        .collect();
    format!(
        "执行档位 虚拟机；虚拟根 {}；装载 {} 个包：{}；网络 {}",
        plan.guest.as_deref().unwrap_or("（未指定）"),
        plan.packages.len(),
        pkgs.join("、"),
        net
    )
}
