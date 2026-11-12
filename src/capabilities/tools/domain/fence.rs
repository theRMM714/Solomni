//! 目的：从权限 reach 派生围栏描述符（策略在这里，机制与类型在 kernel）。
//! 管：FenceSpec::for_call / for_session / standalone——把生效 reach 与模块事实变成可达范围；裁决卡材料（fence_ask 与选项 id）。
//! 不管：描述符自身的形状（在 kernel::contracts::fence）、裁决与回执文案（在 service/fence.rs）、平台机制（在 kernel::detail::confine）。
//! 联动：经 tools::api 给成员循环与协调业务用；kernel 不反向依赖本模块。

use crate::capabilities::permission::api::{Reach, ReachSet};
use crate::capabilities::workspace::api::Sandbox;
use crate::kernel::api::{Ask, FenceBlocked, FenceSpec};
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
        if reach.contains(Reach::ModuleWrite)
            && let Some(id) = module
            && sb.modules_with_userdata.contains(id)
            && let Some(root) = sb.modules.get(id)
        {
            rw.push(root.join("userdata"));
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

/// 目的：必要落点授不上时给用户的选项 id——**本轮无围栏跑一次**（这一次调用按无围栏执行）。
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub const OPT_FENCE_UNFENCED: &str = "fence_unfenced_once";
/// 目的：必要落点授不上时给用户的选项 id——**放弃这次调用**（不执行；没有回答也是它）。
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub const OPT_FENCE_ABORT: &str = "fence_abort";

/// 目的：把"必要落点授不上"变成一条裁决（消息三段 + 选项集）——问什么、几个选项由工具层自己定。
/// 参数：`name` = 谁撞上的（agent 实例名，写进信封）；`unfenced_possible` = 无围栏跑这一次
///   **真能不能跑起来**（工作目录在不在这类事实，由调用方读盘后喂进来——domain 不读盘）。
/// 返回：`None` = **构不出可用选项**（除"放弃"外没有一条真能执行的）：调用方**不发起裁决**，
///   改为停掉这个会话 + 落一条警告（契约禁止置灰）。
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub fn fence_ask(block: &FenceBlocked, name: &str, unfenced_possible: bool) -> Option<Ask> {
    if !unfenced_possible {
        return None;
    }
    Some(Ask {
        role: "tools".to_string(),
        name: name.to_string(),
        title: format!("围栏的{}装不上，这次调用怎么跑？", block.part.label()),
        body: "按规则不许悄悄按无围栏执行。要么这一轮破例跑一次（这次就没有容器那层强制），要么放弃这次调用。"
            .to_string(),
        detail: block.line(),
        options: vec![
            (
                OPT_FENCE_UNFENCED.to_string(),
                "本轮无围栏跑一次".to_string(),
            ),
            (OPT_FENCE_ABORT.to_string(), "放弃这次调用".to_string()),
        ],
        // 没人答时按发起方自己的选项收场：当我选了"放弃这次调用"（= 不执行，fail-closed）。
        on_unanswered: Some(OPT_FENCE_ABORT.to_string()),
    })
}
