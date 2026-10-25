//! 目的：运行包的选型诊断与执行计划（纯逻辑）：缺包 / 多版本 / 定版 / 冲突，以及按能力选包。
//! 管：从包库事实派生 `PackagePlan` 与可读诊断；本机档与虚拟机档共用同一套目标选择。
//! 不管：目录遍历（detail/）、guest 挂载与叠写（vm 能力）、实际工具进程（tools）。
//! 联动：被 api.rs 重导出；由 conductor / diagnostics / residents 消费。

use crate::capabilities::modules::api::Module;
use crate::capabilities::packages::domain::manifest::{
    conflicts, Library, PackageManifest, Target, KIND_SYSTEM,
};
use crate::kernel::api::ExecSpec;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// 目的：一条**包选择**诊断：结构化事实（界面文案由呈现层渲染）。
/// 约束：模块内产物不在场是另一类事实，归 `vm::api::EntryProblem`，不混进这里。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Problem {
    /// 模块（含它的包的传递依赖）需要的能力，包库里没有任何包提供。
    Missing { module: String, capability: String },
    /// 模块需要的是基础能力，但项目内基础 profile 没装，宿主也没有回退。
    MissingBase { module: String, capability: String },
    /// 同一能力有多个版本，需用户定版。
    Ambiguous {
        capability: String,
        versions: Vec<String>,
    },
    /// exec 段定的版本在库里不存在。
    UnknownPin { capability: String, version: String },
    /// 两个包会写进同一处路径（只发生在虚拟机档）。
    Conflict { path: String, a: String, b: String },
    /// 包库里只有**别的目标**能用的包（system 类、只有 guest 载荷、平台不匹配等）。
    NotForTarget {
        module: String,
        capability: String,
        target: String,
        packages: Vec<String>,
    },
    /// 选中的包没有可执行的 `bin/`（本机档无法把命令放进 PATH）。
    NoBin {
        module: String,
        capability: String,
        package: String,
    },
    /// PATH 合成失败（路径里有无法放进 PATH 的字符）。
    Path { why: String },
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

/// 目的：一次选型的结果：选中的包、结构化问题、按模块汇总的不可用能力。
/// 约束：`unusable` 与不可执行判据同源——工具循环与直接动作都读它，不另算一遍。
#[derive(Debug, Clone, Default)]
pub struct PackagePlan {
    /// 目的：装载顺序：先独立前缀，后写进系统路径的包；各自内部按 (id, version) 稳定。
    pub packages: Vec<PlannedPackage>,
    pub problems: Vec<Problem>,
    /// 目的：模块 id → 本档位下用不了的能力名（空 = 都能用）。
    pub unusable: BTreeMap<String, Vec<String>>,
    /// 目的：本机档的生效 PATH（包 `bin/` 在前，当前 PATH 在后）；虚拟机档为空。
    pub path: Option<OsString>,
    /// 目的：本机档要放行给围栏的包内容根（只读）；虚拟机档为空。
    pub roots: Vec<PathBuf>,
    /// 目的：能力 → 实际提供者（包或宿主 PATH），供构建指纹与排障。
    pub providers: BTreeMap<String, Provider>,
}

/// 目的：一个能力最终由谁提供。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Provider {
    /// 项目内基础 profile（python / node / cc）。
    Base {
        id: String,
        version: String,
    },
    /// 宿主 PATH（额外能力的回退；基础能力只有在围栏可授权时才走这里）。
    Host,
    Package {
        id: String,
        version: String,
    },
}

/// 目的：宿主回退的一个解释器：路径 + 平台围栏能不能放行。
/// 约束：fenced=false 不等于不可执行——工具调用仍可经统一裁决通道选择"本次无围栏跑"；
///   启动报告必须把这一条如实说清（服务没有可回答前端时 fail-closed）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFallback {
    pub path: std::path::PathBuf,
    pub fenced: bool,
    pub why: String,
}

/// 目的：项目内基础 profile 的事实（组合根从 active.json / HostProbe / 围栏探针算好）。
#[derive(Debug, Clone, Default)]
pub struct BaseFacts {
    pub active: Vec<(String, String)>,
    pub fallback: std::collections::BTreeMap<String, HostFallback>,
}

/// 目的：能力名 → 基础 profile 的工具 id；不是基础能力就返回 None。
pub fn base_id_for(cap: &str) -> Option<&'static str> {
    match cap {
        "python" | "python3" => Some("python"),
        "node" => Some("node"),
        "cc" | "gcc" | "g++" | "c++" | "clang" | "clang++" => Some("cc"),
        _ => None,
    }
}

/// 目的：多版本且没定版时的策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinPolicy {
    /// 会话：必须由用户定版，核心不替用户挑。
    Require,
    /// 一次性构建 / 直接动作 / 常驻服务：没有定版入口，按包库稳定序取第一个。
    First,
}

/// 目的：一次选型要问宿主的事实（由调用方用 HostProbe / std::env 提供，domain 不读盘）。
pub struct Facts<'a> {
    /// 目的：宿主生效 PATH 上有没有这个可执行名（基础工具链或宿主自装）。
    pub has_exe: &'a dyn Fn(&str) -> bool,
    /// 目的：这个 `bin` 目录可用吗（本机档要求目录在场；虚拟机档不用）。
    pub bin_ok: &'a dyn Fn(&Path) -> bool,
    /// 目的：当前进程 PATH（合成生效 PATH 的尾部）。
    pub current_path: &'a OsStr,
    /// 目的：项目内基础 profile 已装上的工具（id, version）。
    pub base: &'a [(String, String)],
    /// 目的：基础能力的宿主回退事实（工具 id → 解释器路径 + 围栏可用性）。
    pub host_fallback: &'a BTreeMap<String, HostFallback>,
}

/// 目的：一条选型请求：谁（模块 / build）需要哪个能力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub module: String,
    pub capability: String,
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

/// 目的：从模块声明派生选型请求。
pub fn requests_from(modules: &[Module]) -> Vec<Request> {
    let mut out = Vec::new();
    for m in modules {
        for cap in &m.manifest.runtimes {
            out.push(Request {
                module: m.manifest.id.clone(),
                capability: cap.clone(),
            });
        }
    }
    out
}

/// 目的：从一段能力名清单派生请求（构建期 `needs` 用；module 是给诊断看的归属名）。
pub fn requests_for(caps: &[String], module: &str) -> Vec<Request> {
    caps.iter()
        .map(|cap| Request {
            module: module.to_string(),
            capability: cap.clone(),
        })
        .collect()
}

/// 目的：本机档的包计划：按宿主目标选 prefix、产出 PATH/只读根；包优先于基础工具链/宿主 PATH。
pub fn plan_host(
    spec: &ExecSpec,
    modules: &[Module],
    lib: &Library,
    policy: PinPolicy,
    facts: &Facts,
) -> PackagePlan {
    plan_host_requests(&requests_from(modules), &spec.pins, lib, policy, facts)
}

/// 目的：本机档的包计划（请求显式给；构建期 `needs` 用它）。
pub fn plan_host_requests(
    requests: &[Request],
    pins: &BTreeMap<String, String>,
    lib: &Library,
    policy: PinPolicy,
    facts: &Facts,
) -> PackagePlan {
    let target = Target::host();
    let eligible = lib.for_target(&target);
    let mut problems: Vec<Problem> = Vec::new();
    let mut chosen: Vec<&PackageManifest> = Vec::new();
    let mut providers: BTreeMap<String, Provider> = BTreeMap::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut queue: Vec<Request> = requests.to_vec();
    while let Some(req) = queue.pop() {
        if !seen.insert((req.capability.clone(), req.module.clone())) {
            continue;
        }
        let cap = &req.capability;
        let versions = eligible.versions_of(cap);
        if versions.is_empty() {
            // 基础能力：项目内 profile 优先，其次宿主回退（只有围栏能授权才算可用）。
            // 额外能力：仍按包库 → 宿主 PATH 回退。
            if lib.versions_of(cap).is_empty() {
                if let Some(id) = base_id_for(cap) {
                    if let Some((_, version)) = facts.base.iter().find(|(i, _)| i == id) {
                        providers.insert(
                            cap.clone(),
                            Provider::Base {
                                id: id.to_string(),
                                version: version.clone(),
                            },
                        );
                        continue;
                    }
                    match facts.host_fallback.get(id) {
                        Some(_) => {
                            providers.insert(cap.clone(), Provider::Host);
                        }
                        None => {
                            problems.push(Problem::MissingBase {
                                module: req.module.clone(),
                                capability: cap.clone(),
                            });
                        }
                    }
                    continue;
                }
            }
            if has_host_executable(facts.has_exe, cap) {
                providers.insert(cap.clone(), Provider::Host);
                continue;
            }
            let all: Vec<String> = lib
                .versions_of(cap)
                .iter()
                .map(|p| format!("{}@{}", p.id, p.version))
                .collect();
            if all.is_empty() {
                problems.push(Problem::Missing {
                    module: req.module.clone(),
                    capability: cap.clone(),
                });
            } else {
                problems.push(Problem::NotForTarget {
                    module: req.module.clone(),
                    capability: cap.clone(),
                    target: target.label(),
                    packages: all,
                });
            }
            continue;
        }
        let picked: Option<&PackageManifest> = match pins.get(cap) {
            Some(v) => match eligible.pick(cap, v) {
                Some(p) => Some(p),
                None => {
                    let all: Vec<String> = lib
                        .versions_of(cap)
                        .iter()
                        .map(|p| format!("{}@{}", p.id, p.version))
                        .collect();
                    if all.iter().any(|p| p.ends_with(&format!("@{}", v))) {
                        problems.push(Problem::NotForTarget {
                            module: req.module.clone(),
                            capability: cap.clone(),
                            target: target.label(),
                            packages: all,
                        });
                    } else {
                        problems.push(Problem::UnknownPin {
                            capability: cap.clone(),
                            version: v.clone(),
                        });
                    }
                    None
                }
            },
            None if versions.len() == 1 => Some(versions[0]),
            None if policy == PinPolicy::First => Some(versions[0]),
            None => {
                problems.push(Problem::Ambiguous {
                    capability: cap.clone(),
                    versions: versions.iter().map(|p| p.version.clone()).collect(),
                });
                None
            }
        };
        let Some(p) = picked else { continue };
        if !(facts.bin_ok)(&p.source_dir.join("bin")) {
            problems.push(Problem::NoBin {
                module: req.module.clone(),
                capability: cap.clone(),
                package: format!("{}@{}", p.id, p.version),
            });
            continue;
        }
        for r in &p.requires {
            queue.push(Request {
                module: req.module.clone(),
                capability: r.clone(),
            });
        }
        providers.insert(
            cap.clone(),
            Provider::Package {
                id: p.id.clone(),
                version: p.version.clone(),
            },
        );
        chosen.push(p);
    }

    chosen.sort_by(|a, b| {
        (a.id.as_str(), a.version.as_str()).cmp(&(b.id.as_str(), b.version.as_str()))
    });
    let packages: Vec<PlannedPackage> = chosen.iter().map(|p| planned(p)).collect();
    let (path, roots) = host_env(&packages, facts, &mut problems);
    problems.sort();
    problems.dedup();
    PackagePlan {
        unusable: unusable_map(requests, &problems),
        packages,
        problems,
        path,
        roots,
        providers,
    }
}

/// 目的：本机档的生效 PATH 与只读根：选中包的 `bin/` 前置，包内容根放行。
fn host_env(
    packages: &[PlannedPackage],
    facts: &Facts,
    problems: &mut Vec<Problem>,
) -> (Option<OsString>, Vec<PathBuf>) {
    let mut bins: Vec<PathBuf> = Vec::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for p in packages {
        let bin = p.source_dir.join("bin");
        if (facts.bin_ok)(&bin) {
            bins.push(bin);
        }
        roots.push(p.source_dir.clone());
    }
    roots.sort();
    roots.dedup();
    match compose_path(&bins, facts.current_path) {
        Ok(path) => (path, roots),
        Err(why) => {
            problems.push(Problem::Path { why });
            (None, roots)
        }
    }
}

/// 目的：把包的 `bin/` 前置到当前 PATH；不重复、顺序稳定。
fn compose_path(bins: &[PathBuf], current: &OsStr) -> Result<Option<OsString>, String> {
    if bins.is_empty() {
        return Ok(None);
    }
    let mut seen: BTreeSet<OsString> = BTreeSet::new();
    let mut parts: Vec<OsString> = Vec::new();
    for b in bins {
        let s = b.clone().into_os_string();
        if seen.insert(s.clone()) {
            parts.push(s);
        }
    }
    for p in std::env::split_paths(current) {
        let s = p.into_os_string();
        if seen.insert(s.clone()) {
            parts.push(s);
        }
    }
    std::env::join_paths(parts)
        .map(Some)
        .map_err(|e| format!("PATH 合成失败：{}", e))
}

/// 目的：能力名 → 宿主上可能对应的可执行名；未知能力按同名查。
pub fn has_host_executable(has: &dyn Fn(&str) -> bool, cap: &str) -> bool {
    let names: &[&str] = match cap {
        "python" => &["python", "python3"],
        "node" => &["node"],
        "cc" | "c-c++" => &["cc", "gcc", "g++", "clang", "clang++"],
        "bash" => &["bash"],
        _ => return has(cap),
    };
    names.iter().any(|n| has(n))
}

/// 目的：包选择诊断（空 = 可以成立）：缺包 / 多版本歧义 / 定版不存在 / 系统路径冲突。
/// 参数：guest_runtimes = guest 镜像**自带**的运行能力名——镜像里已经有的不必再放 `runtimes/` 包，不算缺。
/// 约束：传递依赖也在这里走完：包 requires 的能力缺失时，归到触发出它的模块上。
pub fn diagnoses(
    modules: &[Module],
    lib: &Library,
    spec: &ExecSpec,
    guest_runtimes: &[String],
) -> Vec<Problem> {
    let mut out: Vec<Problem> = Vec::new();
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
            out.push(Problem::Missing {
                module,
                capability: cap,
            });
            continue;
        }
        let picked: Option<&PackageManifest> = match spec.pins.get(&cap) {
            Some(v) => match lib.pick(&cap, v) {
                Some(p) => Some(p),
                None => {
                    out.push(Problem::UnknownPin {
                        capability: cap.clone(),
                        version: v.clone(),
                    });
                    None
                }
            },
            None if versions.len() == 1 => Some(versions[0]),
            None => {
                out.push(Problem::Ambiguous {
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
    for (path, a, b) in conflicts(&refs) {
        out.push(Problem::Conflict { path, a, b });
    }
    out.sort();
    out.dedup();
    out
}

/// 目的：虚拟机档的包计划：诊断不成立时 packages 为空；问题与不可用能力都如实带回。
/// 约束：选包顺序与 `diagnoses` 同一份事实；guest 自带的运行能力不算缺（镜像里已经有解释器）。
pub fn plan_guest(
    spec: &ExecSpec,
    modules: &[Module],
    lib: &Library,
    guest_runtimes: &[String],
) -> PackagePlan {
    let problems = diagnoses(modules, lib, spec, guest_runtimes);
    let requests = requests_from(modules);
    let unusable = unusable_map(&requests, &problems);
    if !problems.is_empty() {
        return PackagePlan {
            packages: Vec::new(),
            problems,
            unusable,
            ..Default::default()
        };
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
    chosen.sort_by(|a, b| {
        (rank(&a.kind), a.id.as_str(), a.version.as_str()).cmp(&(
            rank(&b.kind),
            b.id.as_str(),
            b.version.as_str(),
        ))
    });
    PackagePlan {
        packages: chosen.iter().map(|p| planned(p)).collect(),
        problems,
        unusable,
        ..Default::default()
    }
}

/// 目的：构建期要装载的包——`needs` 里 guest 自带的跳过，其余按包库挑（连同传递 `requires`）。
/// 返回：（包计划，缺的能力名）。
/// 约束：同一能力多版本时取包库的默认序（会话的定版不适用于一次性构建）；缺的能力如实交给调用方报错。
pub fn build_packages(
    needs: &[String],
    lib: &Library,
    guest_runtimes: &[String],
) -> (Vec<PlannedPackage>, Vec<String>) {
    let mut out = Vec::new();
    let mut missing = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = needs.to_vec();
    while let Some(cap) = queue.pop() {
        if !seen.insert(cap.clone()) {
            continue;
        }
        if guest_runtimes.iter().any(|g| g == &cap) {
            continue;
        }
        let Some(p) = lib.versions_of(&cap).first().copied() else {
            missing.push(cap);
            continue;
        };
        for r in &p.requires {
            queue.push(r.clone());
        }
        out.push(planned(p));
    }
    missing.sort();
    (out, missing)
}

/// 目的：把选中的清单变成计划项。
fn planned(p: &PackageManifest) -> PlannedPackage {
    PlannedPackage {
        id: p.id.clone(),
        version: p.version.clone(),
        kind: p.kind.clone(),
        prefix: p.prefix.clone(),
        source_dir: p.source_dir.clone(),
        provides_paths: p.provides_paths.clone(),
    }
}

/// 目的：模块 id → 本档位下用不了的能力名：缺包只点名那个能力；装配不成立（歧义 / 定版 / 冲突）则
/// 凡声明了运行能力的模块都不可用——与工具循环的"缺包 = 不执行"同一把尺子。
fn unusable_map(requests: &[Request], problems: &[Problem]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if problems.is_empty() {
        return out;
    }
    let mut assembly_broken = false;
    for d in problems {
        match d {
            Problem::Missing { module, capability }
            | Problem::MissingBase { module, capability } => {
                out.entry(module.clone())
                    .or_default()
                    .push(capability.clone());
            }
            _ => assembly_broken = true,
        }
    }
    if assembly_broken {
        for req in requests {
            out.entry(req.module.clone())
                .or_default()
                .push(req.capability.clone());
        }
    }
    for caps in out.values_mut() {
        caps.sort();
        caps.dedup();
    }
    out
}

/// 目的：诊断的可读说法（面向界面/CLI 的 Err）。
pub fn diagnose_text(diags: &[Problem]) -> String {
    let lines: Vec<String> = diags
        .iter()
        .map(|d| match d {
            Problem::Missing {
                module,
                capability,
            } => {
                format!(
                    "模块 {} 需要运行包 {}：包库里没有、guest 镜像也没声明自带（把它放进依赖文件夹 runtimes/，或在 guest.yaml 的 runtimes 里声明）",
                    module, capability
                )
            }
            Problem::MissingBase {
                module,
                capability,
            } => {
                format!(
                    "模块 {} 的基础运行能力 {} 未安装：先运行 node setup-runtime.js --yes 把项目内基础工具链装好",
                    module, capability
                )
            }
            Problem::Ambiguous {
                capability,
                versions,
            } => {
                format!(
                    "运行包 {} 有多个版本，需用户定版：{}",
                    capability,
                    versions.join("、")
                )
            }
            Problem::UnknownPin {
                capability,
                version,
            } => {
                format!("运行包 {} 的定版 {} 不在包库里", capability, version)
            }
            Problem::Conflict { path, a, b } => {
                format!("运行包 {} 与 {} 都要写进 {}，装配会互相覆盖", a, b, path)
            }
            Problem::NotForTarget {
                module,
                capability,
                target,
                packages,
            } => {
                format!(
                    "模块 {} 需要运行包 {}：包库里只有不服务 {} 的包（{}）",
                    module,
                    capability,
                    target,
                    packages.join("、")
                )
            }
            Problem::NoBin {
                module,
                capability,
                package,
            } => {
                format!(
                    "模块 {} 的运行包 {}（{}）没有可执行的 bin/，本机档放不进 PATH",
                    module, capability, package
                )
            }
            Problem::Path { why } => format!("运行包 PATH 合成失败：{}", why),
        })
        .collect();
    lines.join("；")
}

/// 目的：装载顺序：先独立前缀（0），后写进系统路径的包（1）。
fn rank(kind: &str) -> u8 {
    if kind == KIND_SYSTEM {
        1
    } else {
        0
    }
}
