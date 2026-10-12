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
}

/// 目的：执行计划：运行时派生，从不落盘（装配阶段按 (id, version) 去包里定位内容）。
#[derive(Debug, Clone)]
pub struct ExecPlan {
    pub tier: Tier,
    pub net: bool,
    pub base: Option<String>,
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
/// 约束：传递依赖也在这里走完：包 requires 的能力缺失时，归到触发出它的模块上。
pub fn vm_diagnoses(modules: &[Module], lib: &Library, spec: &ExecSpec) -> Vec<Diagnosis> {
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
    pub base: Option<&'a str>,
    pub qemu: Option<&'a str>,
    /// 目的：本机事实探测（读环境、查路径存在性都在它后面，见 ports::HostProbe）。
    pub probe: &'a dyn HostProbe,
}

/// QEMU 可执行文件名（各平台同名；平台扩展名由适配层的探测实现处理）。
const QEMU_BIN: &str = "qemu-system-x86_64";

/// 目的：虚拟机档的全部前置要求（**只读事实，不起任何虚拟机**）。
/// 约束：为什么做成清单：用户看到的必须是"缺哪几项、每项怎么补"，而不是一句笼统的"前置条件不具备"。
/// 约束：base 与会话选型有关，qemu 登记在设置里（见 providers::AppSettings）。
pub fn vm_requirements(vm: &VmInputs<'_>) -> Vec<VmRequirement> {
    let hyper = vm.probe.hypervisor_available();
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
        // guest 后端与执行分派都已就位，但**尚未在真机验证**：验证通过前就绪项判否，虚拟机档不放开。
        // 有意收口：没验过就不让用户选，避免造出"隔离没多、能力反少"的会话。
        VmRequirement {
            id: "guest",
            met: false,
            detail: "guest 后端与执行分派已就位，但尚未在真机验证（就绪项暂判否）".to_string(),
            how: "真机上跑通一次 vm 档工具后再放开（现状与验收见 tests/gaps.yaml）".to_string(),
        },
        // QEMU：用户自备。没登记就看 PATH——检测的是"起得来 guest 的那件东西在不在"。
        qemu_requirement(vm.qemu, vm.probe),
        // 基础根：用户自备（发行版基底 + 内核所在目录）。本机档为空 = 不适用。
        base_requirement(vm.base, vm.probe),
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

/// QEMU 检测：登记了就用登记的路径，没登记就看 PATH（产品不自带 QEMU，也不下载）。
fn qemu_requirement(registered: Option<&str>, probe: &dyn HostProbe) -> VmRequirement {
    let (found, where_from) = match registered.map(str::trim) {
        Some(p) if !p.is_empty() => (probe.is_file(Path::new(p)), "设置里登记的路径"),
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
            "自行安装 QEMU 并放进 PATH，或在设置里登记它的完整路径（本产品不自带、不下载 QEMU）"
                .to_string()
        },
    }
}

/// 目的：基础根检测——用户自备的 guest 基础（rootfs 目录 + vmlinuz/bzImage）在不在、布局对不对。
fn base_requirement(base: Option<&str>, probe: &dyn HostProbe) -> VmRequirement {
    let checked = base.map(str::trim).filter(|p| !p.is_empty()).map(|p| {
        (
            p,
            crate::capabilities::vm::domain::guest::classify_base(Path::new(p), probe),
        )
    });
    let (met, detail) = match checked {
        None => (false, "没有指定基础根".to_string()),
        Some((p, Ok(b))) => (
            true,
            format!(
                "基础根在场：{}（rootfs{}，内核 {}）",
                p,
                if b.initrd.is_some() { " + initrd" } else { "" },
                b.kernel.file_name().unwrap_or_default().to_string_lossy()
            ),
        ),
        Some((p, Err(why))) => (false, format!("基础根不可用：{}（{}）", p, why)),
    };
    VmRequirement {
        id: "base",
        met,
        detail,
        how: if met {
            String::new()
        } else {
            "自备一份 guest 基础根：一个目录，里面是发行版 rootfs 与 vmlinuz（可选 initrd.img），把目录填进会话的 base（本产品不预置、不下载镜像）"
                .to_string()
        },
    }
}

/// 目的：本机能不能承载这个档位（**只读事实，不碰任何东西**）。
/// 约束：虚拟机档的前置条件不具备时，虚拟机档**不允许创建或改入**——这是用户环境问题，不是选型问题；
/// 约束：界面的"能不能点"与「开始」的校验走同一个函数，两处不会各说各话。
/// 约束：判据**只有一份**：就是 vm_requirements 那份清单，逐项都满足才算承载得了。
pub fn tier_readiness(spec: &ExecSpec, qemu: Option<&str>, probe: &dyn HostProbe) -> TierReadiness {
    if spec.tier != Tier::Vm {
        return TierReadiness {
            requirements: Vec::new(),
        };
    }
    TierReadiness {
        requirements: vm_requirements(&VmInputs {
            base: spec.base.as_deref(),
            qemu,
            probe,
        }),
    }
}

/// 目的：虚拟机档的可读拒绝理由（创建与编辑共用同一把尺子）。
pub fn tier_refusal(spec: &ExecSpec, qemu: Option<&str>, probe: &dyn HostProbe) -> Option<String> {
    let readiness = tier_readiness(spec, qemu, probe);
    if readiness.ready() {
        return None;
    }
    Some(format!(
        "虚拟机档现在不可用（{}）：请换本机档。各项前置与怎么补见配置界面的「虚拟机档前置」与 --doctor",
        readiness.missing().join("；")
    ))
}

/// 目的：派生执行计划：本机档不装载运行包（宿主自备解释器）。
/// 约束：虚拟机档只有**选型不成立**才拒绝（多版本未定版 / 定版不存在 / 系统路径冲突——用户要解决的选型问题）；
/// 约束：**缺包不拦会话**：那只是该模块的工具不可用（由 unavailable 收口，降级而非崩溃）。
pub fn plan(
    spec: &ExecSpec,
    modules: &[Module],
    lib: &Library,
) -> Result<ExecPlan, Vec<Diagnosis>> {
    match spec.tier {
        Tier::Host => Ok(ExecPlan {
            tier: Tier::Host,
            net: spec.net,
            base: None,
            packages: Vec::new(),
        }),
        Tier::Vm => {
            let hard: Vec<Diagnosis> = vm_diagnoses(modules, lib, spec)
                .into_iter()
                .filter(|d| !matches!(d, Diagnosis::Missing { .. }))
                .collect();
            if !hard.is_empty() {
                return Err(hard);
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
                })
                .collect();
            Ok(ExecPlan {
                tier: Tier::Vm,
                net: spec.net,
                base: spec.base.clone(),
                packages,
            })
        }
    }
}

/// 目的：本档位下不能执行工具的模块（模块 id → 缺失的能力名）：本机档不装载运行包，一律可用（空表）。
/// 约束：虚拟机档：装配不成立（歧义 / 定版不存在 / 冲突）时，凡声明了运行能力的模块都不能执行工具。
pub fn unavailable(
    spec: &ExecSpec,
    modules: &[Module],
    lib: &Library,
) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if spec.tier != Tier::Vm {
        return out;
    }
    let diags = vm_diagnoses(modules, lib, spec);
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
                    "模块 {} 需要运行包 {}，包库里没有（把它放进依赖文件夹 runtimes/）",
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
        "执行档位 虚拟机；基础根 {}；装载 {} 个包：{}；网络 {}",
        plan.base.as_deref().unwrap_or("（未指定）"),
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
