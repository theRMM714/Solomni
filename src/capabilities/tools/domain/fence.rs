//! 目的：从权限 reach 派生围栏描述符（策略在这里，机制与类型在 kernel）。
//! 管：FenceSpec::for_call / for_session / standalone——把生效 reach 与模块事实变成可达范围。
//! 不管：描述符自身的形状与平台机制（在 kernel::domain::fence 与 kernel::detail::confine）。
//! 联动：经 tools::api 给成员循环与协调业务用；kernel 不反向依赖本模块。

use crate::capabilities::permission::api::{Reach, ReachSet};
use crate::capabilities::workspace::api::Sandbox;
use crate::kernel::api::FenceSpec;
use std::path::{Path, PathBuf};

impl FenceSpec {
    /// 目的：为**一次调用**派生围栏；base 是同一会话的 for_session 结果（提供主机只读根与全局断网事实）。
    /// 约束：shared-write 永不进 rw；module-write 只给指定模块自己的 userdata；网络再与全局上限求交。
    pub fn for_call(
        sb: &Sandbox,
        reach: &ReachSet,
        module: Option<&str>,
        base: &FenceSpec,
    ) -> FenceSpec {
        let mut rw: Vec<PathBuf> = Vec::new();
        let mut ro_tree: Vec<PathBuf> = Vec::new();
        let mut ro: Vec<PathBuf> = Vec::new();
        if reach.contains(Reach::Private) {
            rw.push(sb.private.clone());
        }
        if reach.contains(Reach::ModuleWrite) {
            if let Some(id) = module {
                if sb.modules_with_userdata.contains(id) {
                    if let Some(root) = sb.modules.get(id) {
                        rw.push(root.join("userdata"));
                    }
                }
            }
        }
        if reach.contains(Reach::SharedRead) {
            ro_tree.push(sb.shared.clone());
        }
        if reach.contains(Reach::ModuleRead) {
            ro_tree.extend(sb.modules.values().cloned());
        }
        if reach.contains(Reach::HostRead) {
            ro.extend(base.ro.iter().cloned());
        }
        rw.sort();
        rw.dedup();
        ro.sort();
        ro.dedup();
        ro_tree.sort();
        ro_tree.dedup();
        FenceSpec {
            agent: sb.agent.clone(),
            lease: sb.session.clone(),
            rw,
            ro: { ro },
            ro_tree,
            private: sb.private.clone(),
            cwd: PathBuf::new(),
            net: reach.contains(Reach::Network) && base.net,
        }
    }

    /// 目的：为一个会话派生**并集围栏**——覆盖它可能签发过的全部 reach，用于撤销持久授权与生命周期。
    pub fn for_session(sb: &Sandbox, host_roots: &[PathBuf], global_net: bool) -> FenceSpec {
        let reach = &sb.permissions.reach;
        let mut rw: Vec<PathBuf> = Vec::new();
        let mut ro_tree: Vec<PathBuf> = Vec::new();
        let mut ro: Vec<PathBuf> = Vec::new();
        if reach.contains(Reach::Private) {
            rw.push(sb.private.clone());
        }
        if reach.contains(Reach::ModuleWrite) {
            for id in &sb.modules_with_userdata {
                if let Some(root) = sb.modules.get(id) {
                    rw.push(root.join("userdata"));
                }
            }
        }
        if reach.contains(Reach::SharedRead) {
            ro_tree.push(sb.shared.clone());
        }
        if reach.contains(Reach::ModuleRead) {
            ro_tree.extend(sb.modules.values().cloned());
        }
        if reach.contains(Reach::HostRead) {
            ro.extend(host_roots.iter().cloned());
        }
        rw.sort();
        rw.dedup();
        ro.sort();
        ro.dedup();
        ro_tree.sort();
        ro_tree.dedup();
        FenceSpec {
            agent: sb.agent.clone(),
            lease: sb.session.clone(),
            rw,
            ro: { ro },
            ro_tree,
            private: sb.private.clone(),
            cwd: PathBuf::new(),
            net: global_net && reach.contains(Reach::Network),
        }
    }

    /// 目的：**无会话**（人直接跑一个模块工具 / 常驻服务）的围栏——模块目录递归只读 + 它自己的 userdata 可写。
    /// 约束：这是模块自身进程的口径；agent 的内置文件工具对模块目录（含 userdata）只读。
    pub fn standalone(
        module_root: &Path,
        work_root: Option<&Path>,
        has_userdata: bool,
    ) -> FenceSpec {
        let userdata = module_root.join("userdata");
        let private = match work_root {
            Some(p) => p.to_path_buf(),
            None if has_userdata => userdata.clone(),
            None => PathBuf::new(),
        };
        let mut rw: Vec<PathBuf> = Vec::new();
        if !private.as_os_str().is_empty() {
            rw.push(private.clone());
        }
        if has_userdata && private != userdata {
            rw.push(userdata);
        }
        rw.sort();
        rw.dedup();
        FenceSpec {
            agent: "user".to_string(),
            lease: String::new(),
            rw,
            ro: Vec::new(),
            ro_tree: vec![module_root.to_path_buf()],
            private,
            cwd: module_root.to_path_buf(),
            net: false,
        }
    }
}
