//! 目的：执行档位与执行计划的纯逻辑——档位诊断、计划派生、承载判定与就绪检测。
//! 管：ExecSpec 的选型诊断（缺包 / 歧义 / 定版不存在 / 路径冲突）、ExecPlan 的装载顺序、本机能否承载虚拟机档。
//! 不管：目录遍历与 guest 启动（在 detail/ 与 guest 后端）、会话档案（session）、实际的工具进程执行（tools）。
//! 联动：契约见 docs/vm/README.md 与 RUNTIME_SPEC.md；ExecSpec 是 kernel 的共享事实；被 conductor 与呈现层经 api.rs 消费。

use crate::capabilities::modules::api::Module;
use crate::capabilities::vm::domain::packages::{Library, PackageManifest, KIND_SYSTEM};
use crate::kernel::api::{ExecSpec, Tier};
use crate::kernel::ports::HostProbe;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 目的：一条诊断：结构化事实（界面文案由呈现层渲染；模型侧文案在提示词册）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Diagnosis {
    /// 模块（含它的包的传递依赖）需要的能力，包库里没有任何包提供。
    Missing { module: String, capability: String },
    /// 同一能力有多个版本，需用户定版。
    Ambiguous {
        capability: String,
        versions: Vec<String>,
    },
    /// exec 段定的版本在库里不存在。
    UnknownPin { capability: String, version: String },
    /// 两个包会写进同一处路径。
    Conflict { path: String, a: String, b: String },
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

/// 目的：按宿主事实挑出"入口不在场"的模块工具——相对路径入口必须存在且**能直接执行**。
/// 参数：is_executable = 宿主侧"这个路径能不能直接跑"的事实（由端口给；本函数不碰文件系统）。
/// 约束：裸名（`python` / `node` / `indexer`）走 PATH 与运行能力，不在本判据里；绝对路径同样不在模块目录里找。
pub fn missing_entries(
    modules: &[Module],
    is_executable: &dyn Fn(&Path) -> bool,
) -> Vec<MissingEntry> {
    missing_entries_for(modules, std::env::consts::OS, is_executable)
}

/// 目的：同上，但**目标系统**由调用方给——判据跟着那台机器的壳解析走（用例据此在别的平台上验 Windows 语义）。
/// 参数：os = 目标系统（`std::env::consts::OS` 的取值）；其余同上。
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

/// 目的：计划里的一项：已定版的包。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPackage {
    pub id: String,
    pub version: String,
    pub kind: String,
    pub prefix: String,
    /// 目的：包内容所在目录（清单扫描时记下）；装配阶段用它把包挂进 guest。
    pub source_dir: std::path::PathBuf,
    /// 目的：kind = system 时要叠写进 guest 的**相对系统路径**（prefix 包为空）。
    pub provides_paths: Vec<String>,
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

/// 目的：模块声明的能力（模块 id → 能力名，升序；去重）。
pub fn declared(modules: &[Module]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in modules {
        if m.manifest.runtimes.is_empty() {
            continue;
        }
        let mut caps = m.manifest.runtimes.clone();
        caps.sort();
        caps.dedup();
        out.insert(m.manifest.id.clone(), caps);
    }
    out
}

/// 目的：哪些模块声明的能力包库里没有（模块 id → 缺失的能力名）——档位无关的事实。
pub fn absent(modules: &[Module], lib: &Library) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in modules {
        let miss: Vec<String> = m
            .manifest
            .runtimes
            .iter()
            .filter(|c| lib.versions_of(c).is_empty())
            .cloned()
            .collect();
        if !miss.is_empty() {
            out.insert(m.manifest.id.clone(), miss);
        }
    }
    out
}

/// 目的：虚拟机档的诊断（空 = 可以成立）：缺包 / 多版本歧义 / 定版不存在 / 系统路径冲突。
/// 参数：guest_runtimes = guest 镜像**自带**的运行能力名——镜像里已经有的不必再放 `runtimes/` 包，不算缺。
/// 约束：传递依赖也在这里走完：包 requires 的能力缺失时，归到触发出它的模块上。
pub fn vm_diagnoses(
    modules: &[Module],
    lib: &Library,
    spec: &ExecSpec,
    guest_runtimes: &[String],
) -> Vec<Diagnosis> {
    let mut out: Vec<Diagnosis> = Vec::new();
    let mut queue: Vec<(String, String)> = Vec::new();
    for m in modules {
        for c in &m.manifest.runtimes {
            queue.push((c.clone(), m.manifest.id.clone()));
        }
    }
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut chosen: BTreeMap<String, &PackageManifest> = BTreeMap::new();
    while let Some((cap, module)) = queue.pop() {
        if !seen.insert((cap.clone(), module.clone())) {
            continue;
        }
        let versions = lib.versions_of(&cap);
        if versions.is_empty() {
            if guest_runtimes.iter().any(|g| g == &cap) {
                // guest 镜像自带这个能力：不必再放 runtimes/ 包，也不算缺。
                continue;
            }
            out.push(Diagnosis::Missing {
                module,
                capability: cap,
            });
            continue;
        }
        let picked: Option<&PackageManifest> = match spec.pins.get(&cap) {
            Some(v) => match lib.pick(&cap, v) {
                Some(p) => Some(p),
                None => {
                    out.push(Diagnosis::UnknownPin {
                        capability: cap.clone(),
                        version: v.clone(),
                    });
                    None
                }
            },
            None if versions.len() == 1 => Some(versions[0]),
            None => {
                out.push(Diagnosis::Ambiguous {
                    capability: cap.clone(),
                    versions: versions.iter().map(|p| p.version.clone()).collect(),
                });
                None
            }
        };
        if let Some(p) = picked {
            for r in &p.requires {
                queue.push((r.clone(), module.clone()));
            }
            chosen.insert(cap, p);
        }
    }
    let refs: Vec<&PackageManifest> = chosen.values().copied().collect();
    for (path, a, b) in crate::capabilities::vm::domain::packages::conflicts(&refs) {
        out.push(Diagnosis::Conflict { path, a, b });
    }
    out.sort();
    out.dedup();
    out
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
                format!("guest 后端已就位，本平台默认组合 {} 已通过真机验收", combo)
            } else {
                format!(
                    "guest 后端已就位，但本平台默认组合 {} 尚未真机验收（就绪项判否）",
                    combo
                )
            },
            how: if guest_backend_verified() {
                String::new()
            } else {
                format!(
                    "在该平台真机上跑通一次 vm 档工具（默认组合 {}）后再放开（现状与验收见 tests/gaps.yaml）",
                    combo
                )
            },
        },
        // 共享档：只作事实呈现（**不参与 ready**）——缺 virtiofsd 时组合自动回退 9p，虚拟机档仍可用。
        share_requirement(vm.share_tool, vm.probe, &combo),
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

/// 目的：已通过真机验收的宿主平台（**数据，不是平台 cfg**）——新平台验收后在这里加一行，并同步 tests/gaps.yaml。
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
    Combo {
        engine: "qemu",
        share: "copy",
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
    Combo {
        engine: "qemu",
        share: "copy",
    },
];
const WINDOWS_COMBOS: &[Combo] = &[
    Combo {
        engine: "qemu",
        share: "smb",
    },
    Combo {
        engine: "qemu",
        share: "copy",
    },
];
const FALLBACK_COMBOS: &[Combo] = &[Combo {
    engine: "qemu",
    share: "copy",
}];

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

/// 目的：已经实现的共享后端（组合选择与就绪说明**共用同一份**，避免"说要验 A、实际跑 B"）。
pub const IMPLEMENTED_SHARES: &[&str] = &["9p", "copy", "virtiofs", "smb"];

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

/// 目的：一个外部工具在不在场——登记路径优先（登记了就必须是那个文件），否则看 PATH。
fn tool_available(name: &str, registered: Option<&str>, probe: &dyn HostProbe) -> bool {
    match registered.map(str::trim) {
        // 登记了就以登记的为准，而且得**能直接执行**——只在场（比如权限位丢了）不算可用，
        // 否则组合会选它、到启动那一步才炸；这里如实判"不在场"，让回退链与就绪文案说真话。
        Some(p) if !p.is_empty() => probe.is_executable(Path::new(p)),
        _ => probe.has_exe(name),
    }
}

/// 目的：这次启动**可试**的共享后端 id，按本平台"性能优先"的已实现顺序；工具不在场的档不列。
/// 参数：host = 宿主平台（std::env::consts::OS 的取值）；share_tool = 本平台共享工具路径（None = 兜底 PATH）；probe = 宿主事实。
/// 约束：只判"已实现 + 工具在场"（share_available）；落不落到这台引擎 / 这份 guest 上由共享后端自己判
///   （ShareBackend::supports）——所以**选档发生在起 guest 那一刻**，不是装配期定一次。
/// 约束：引擎恒按 qemu 筛（当前唯一已实现引擎）；返回空表 = 本平台这一份没有可试的共享档。
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

/// 目的：某个共享档在不在场：先要是已实现的档，再要它要的外部工具在场（不需要工具的档恒在场）。
/// 约束：组合选择（组合根）与就绪说明共用这一份，避免"说要验 A、实际跑 B"。
pub fn share_available(share: &str, share_tool: Option<&str>, probe: &dyn HostProbe) -> bool {
    if !IMPLEMENTED_SHARES.contains(&share) {
        return false;
    }
    match share_tool_of(share) {
        None => true,
        Some(name) => tool_available(name, share_tool, probe),
    }
}

/// 目的：本平台**实际会选**的已实现组合标签（"引擎/共享"）：引擎 qemu 在、共享档在场，按回退链取第一个。
fn effective_combo_label(share_tool: Option<&str>, probe: &dyn HostProbe) -> String {
    default_combos(std::env::consts::OS)
        .iter()
        .find(|c| c.engine == "qemu" && share_available(c.share, share_tool, probe))
        .map(|c| format!("{}/{}", c.engine, c.share))
        .unwrap_or_else(|| "未定".to_string())
}

/// 目的：共享档的现状——本平台实现链里性能最高的那一档在不在场、不在就回退到下一档。
/// 约束：**恒 met、不参与承载判定**——缺 virtiofsd 时组合自动回退 9p，虚拟机档照样可用；
///   所以"怎么补"写在 detail 里（met 恒真，how 按约定留空），不把性能档的缺失误报成档位不可用。
fn share_requirement(
    share_tool: Option<&str>,
    probe: &dyn HostProbe,
    effective_combo: &str,
) -> VmRequirement {
    let top = default_combos(std::env::consts::OS)
        .iter()
        .find(|c| c.engine == "qemu" && IMPLEMENTED_SHARES.contains(&c.share));
    let detail = match top {
        None => "本平台还没有已实现的共享档".to_string(),
        Some(c) => match share_tool_of(c.share) {
            None => format!("共享档 {}（不需要外部工具）", c.share),
            Some(name) => {
                if share_available(c.share, share_tool, probe) {
                    format!("共享档 {} 可用（找到 {}）", c.share, name)
                } else {
                    match share_tool.map(str::trim) {
                        Some(p)
                            if !p.is_empty()
                                && probe.is_file(Path::new(p))
                                && !probe.is_executable(Path::new(p)) =>
                        {
                            format!(
                                "共享档 {} 登记的那个不能执行（{}）：已按组合回退到 {}；给它执行位，或换成 PATH 上的 {} 即可启用",
                                c.share, p, effective_combo, name
                            )
                        }
                        _ => format!(
                            "共享档 {} 不在场（没找到 {}）：已按组合回退到 {}；把 {} 放进 PATH，或在设置里登记它的完整路径即可启用",
                            c.share, name, effective_combo, name
                        ),
                    }
                }
            }
        },
    };
    VmRequirement {
        id: "share",
        met: true,
        detail,
        how: String::new(),
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
            let root_kind = match &b.root {
                crate::capabilities::vm::domain::guest::GuestRoot::Dir(_) => "目录 rootfs",
                crate::capabilities::vm::domain::guest::GuestRoot::Disk(_) => "磁盘镜像 rootfs",
            };
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

/// 目的：派生执行计划：本机档不装载运行包（宿主自备解释器）。
/// 约束：虚拟机档**任何诊断都拒绝**——选型不成立（多版本未定版 / 定版不存在 / 系统路径冲突）与
///   **缺包**都算：没有工具用户完不成工作，与其拉一台跑不了工具的 guest，不如在选型这步如实拒绝。
/// 约束：guest 自带的运行能力（`guest_runtimes`）不算缺（镜像里已经有解释器）。
pub fn plan(
    spec: &ExecSpec,
    modules: &[Module],
    lib: &Library,
    guest_runtimes: &[String],
    missing: &[MissingEntry],
) -> Result<ExecPlan, Vec<Diagnosis>> {
    // 入口判据与档位无关：模块根下没有那个能直接跑的产物，两档都当场拒绝（一次把话说全）。
    let entries: Vec<Diagnosis> = missing
        .iter()
        .map(|m| Diagnosis::MissingEntry {
            module: m.module.clone(),
            tool: m.tool.clone(),
            command: m.command.clone(),
            buildable: m.buildable,
        })
        .collect();
    match spec.tier {
        Tier::Host => {
            if !entries.is_empty() {
                return Err(entries);
            }
            Ok(ExecPlan {
                tier: Tier::Host,
                net: spec.net,
                guest: None,
                packages: Vec::new(),
            })
        }
        Tier::Vm => {
            let mut diags = entries;
            diags.extend(vm_diagnoses(modules, lib, spec, guest_runtimes));
            if !diags.is_empty() {
                return Err(diags);
            }
            let mut chosen: Vec<&PackageManifest> = Vec::new();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut queue: Vec<String> = modules
                .iter()
                .flat_map(|m| m.manifest.runtimes.clone())
                .collect();
            while let Some(cap) = queue.pop() {
                if !seen.insert(cap.clone()) {
                    continue;
                }
                let versions = lib.versions_of(&cap);
                let picked = match spec.pins.get(&cap) {
                    Some(v) => versions.iter().copied().find(|p| &p.version == v),
                    None => versions.first().copied(),
                };
                if let Some(p) = picked {
                    for r in &p.requires {
                        queue.push(r.clone());
                    }
                    chosen.push(p);
                }
            }
            // 装载顺序：先独立前缀，后写进系统路径的包；各自内部按 (id, version) 稳定。
            chosen.sort_by(|a, b| {
                (rank(&a.kind), a.id.as_str(), a.version.as_str()).cmp(&(
                    rank(&b.kind),
                    b.id.as_str(),
                    b.version.as_str(),
                ))
            });
            let packages = chosen
                .iter()
                .map(|p| PlannedPackage {
                    id: p.id.clone(),
                    version: p.version.clone(),
                    kind: p.kind.clone(),
                    prefix: p.prefix.clone(),
                    source_dir: p.source_dir.clone(),
                    provides_paths: p.provides_paths.clone(),
                })
                .collect();
            Ok(ExecPlan {
                tier: Tier::Vm,
                net: spec.net,
                guest: spec.guest.clone(),
                packages,
            })
        }
    }
}

/// 目的：本档位下不能执行工具的模块（模块 id → 缺失的能力名）：本机档不装载运行包，一律可用（空表）。
/// 约束：虚拟机档：缺包 / 装配不成立（歧义 / 定版不存在 / 冲突）时，凡声明了运行能力的模块都不能执行工具。
/// 约束：guest 自带的运行能力不算缺（镜像里已经有解释器）——与 `plan` 同一把尺子。
pub fn unavailable(
    spec: &ExecSpec,
    modules: &[Module],
    lib: &Library,
    guest_runtimes: &[String],
) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if spec.tier != Tier::Vm {
        return out;
    }
    let diags = vm_diagnoses(modules, lib, spec, guest_runtimes);
    if diags.is_empty() {
        return out;
    }
    let mut assembly_broken = false;
    for d in &diags {
        match d {
            Diagnosis::Missing { module, capability } => {
                out.entry(module.clone())
                    .or_default()
                    .push(capability.clone());
            }
            _ => assembly_broken = true,
        }
    }
    if assembly_broken {
        for m in modules {
            if !m.manifest.runtimes.is_empty() {
                let mut caps = m.manifest.runtimes.clone();
                caps.sort();
                caps.dedup();
                out.insert(m.manifest.id.clone(), caps);
            }
        }
    }
    out
}

/// 目的：诊断的可读说法（面向界面/CLI 的 Err，与 conductor 其它 Err 同一做法）。
pub fn diagnose_text(diags: &[Diagnosis]) -> String {
    let lines: Vec<String> = diags
        .iter()
        .map(|d| match d {
            Diagnosis::Missing { module, capability } => {
                format!(
                    "模块 {} 需要运行包 {}：包库里没有、guest 镜像也没声明自带（把它放进依赖文件夹 runtimes/，或在 guest.yaml 的 runtimes 里声明）",
                    module, capability
                )
            }
            Diagnosis::Ambiguous {
                capability,
                versions,
            } => {
                format!(
                    "运行包 {} 有多个版本，需用户定版：{}",
                    capability,
                    versions.join("、")
                )
            }
            Diagnosis::UnknownPin {
                capability,
                version,
            } => {
                format!("运行包 {} 的定版 {} 不在包库里", capability, version)
            }
            Diagnosis::Conflict { path, a, b } => {
                format!("运行包 {} 与 {} 都要写进 {}，装配会互相覆盖", a, b, path)
            }
            Diagnosis::MissingEntry {
                module,
                tool,
                command,
                buildable,
            } => {
                let how = if *buildable {
                    format!("跑 solomni --build={} 让系统照模块声明的构建方法编一次", module)
                } else {
                    "模块要自带这个产物，或先按模块自己的 README 构建一次".to_string()
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

fn rank(kind: &str) -> u8 {
    if kind == KIND_SYSTEM {
        1
    } else {
        0
    }
}
