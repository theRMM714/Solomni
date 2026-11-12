//! 目的：工作与会话配置的方法族入口——形状（`PreparedWork` / `PendingWork` / `PreparedEdit`）与跨子模块共用的判据。
//! 管：建工作的两段结果、改配的准备结果、模块声明的能力表、运行包重建判据。
//! 不管：报告与档位（`report.rs`）、配置读写（`config.rs`）、调用参数（`model.rs`）、宿主工具路径（`host.rs`）、视图（`views.rs`）、建工作与 guest 启动（`create.rs`）。
//! 联动：api 层与兄弟方法族经这里的方法名调用；各子模块 impl 同一个 `Conductor`。
//!   本文件不碰 `Conductor` 的字段：只放形状与跨子模块共用的判据。

mod config;
mod create;
mod host;
mod model;
mod report;
mod views;

use super::guest_job::{EditGuest, GuestBoot};
use super::*;
use crate::capabilities::packages::api as pkgs;

/// 目的：一次创建的准备结果——"已经建完"或"待提交"（后者等调用方线程把 guest 起起来）。
pub(crate) enum PreparedWork {
    /// 已经建完（代理形态：没有 guest 可等）。
    Ready(WorkOpened),
    /// 目的：待提交——核心已占名并备好工作区与计划，慢的那一段由调用方跑；装箱避免枚举被最大变体撑大。
    Pending(Box<PendingWork>),
}

/// 目的：一次创建的"待提交"准备结果。
pub(crate) struct PendingWork {
    meta: Box<SessionMeta>,
    mode: WorkMode,
    metas: Vec<AgentMeta>,
    task: Option<String>,
    delegate: bool,
    name: String,
    agent_names: Vec<String>,
    sandboxes: crate::capabilities::workspace::api::Sandboxes,
    /// 每个 agent 已解析好的通道（提交段直接装配，不再可能因"没模型"失败）。
    channels: BTreeMap<String, Channel>,
    /// 协作会话核心自己的通道（协作必须有；单 agent 未用核心默认时为空）。
    core_channel: Option<Channel>,
    /// 失败时要撤掉的工作区空目录（只对顶层会话给：子会话与父共用目录）。
    discard: Option<(String, Vec<String>)>,
}

/// 目的：一次改配的准备结果——新的 meta 与要同步的子树（落盘用）。
/// 约束：围栏授权是按次的（调用结束即撤），改配不再按旧配置撤权，所以这里不留旧值。
pub(crate) struct PreparedEdit {
    pub(crate) sid: String,
    pub(crate) new_meta: SessionMeta,
    pub(crate) new_children: Vec<SessionMeta>,
}
/// 目的：模块声明的能力（模块 id → 能力名，升序；去重）——报告 metadata。
fn declared(modules: &[Module]) -> BTreeMap<String, Vec<String>> {
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

/// 目的：新子会话要不要为运行包重建共享 guest——只有活 guest 且它缺其中某个包时才要。
pub(crate) fn needs_guest_packages_rebuild(
    active: bool,
    want: &[String],
    have: Option<&[String]>,
) -> bool {
    if !active {
        return false;
    }
    match have {
        Some(have) => want.iter().any(|p| !have.contains(p)),
        None => !want.is_empty(),
    }
}

/// 目的：guest 装配只认"agent → 模块集合"与环境；模型 / 权限变化不该触发重建。
fn agent_modules(meta: &SessionMeta) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for a in &meta.agents {
        let mut modules = a.modules.clone();
        modules.sort();
        modules.dedup();
        out.insert(a.name.clone(), modules);
    }
    out
}

/// 目的：这次改配要不要重建共享 guest——模块集合或 ExecSpec（档位 / 虚拟根 / 定版）变了才要。
/// 网络是全局策略；全局变更走整体重启，不在这里触发按会话重建。
fn guest_assembly_changed(old: &SessionMeta, new: &SessionMeta) -> bool {
    old.exec.tier != new.exec.tier
        || old.exec.guest != new.exec.guest
        || old.exec.pins != new.exec.pins
        || agent_modules(old) != agent_modules(new)
}
