//! 目的：运行包的选型诊断与执行计划（纯逻辑）：缺包 / 多版本 / 定版 / 冲突，以及按能力选包。
//! 管：从包库事实派生 `PackagePlan` 与可读诊断；本机档与虚拟机档共用同一套目标选择。
//! 不管：目录遍历（detail/）、guest 挂载与叠写（vm 能力）、实际工具进程（tools）。
//! 联动：被 api.rs 重导出；由 conductor / diagnostics / residents 消费。

use crate::capabilities::modules::api::Module;
use crate::capabilities::packages::domain::manifest::{
    conflicts, Library, PackageManifest, KIND_SYSTEM,
};
use crate::kernel::api::ExecSpec;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// 目的：一条**包选择**诊断：结构化事实（界面文案由呈现层渲染）。
/// 约束：模块内产物不在场是另一类事实，归 `vm::api::EntryProblem`，不混进这里。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Problem {
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
    let unusable = unusable_map(modules, &problems);
    if !problems.is_empty() {
        return PackagePlan {
            packages: Vec::new(),
            problems,
            unusable,
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
fn unusable_map(modules: &[Module], problems: &[Problem]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if problems.is_empty() {
        return out;
    }
    let mut assembly_broken = false;
    for d in problems {
        match d {
            Problem::Missing { module, capability } => {
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
