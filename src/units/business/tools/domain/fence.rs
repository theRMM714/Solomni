//! 目的：从固定授予派生围栏描述符（策略在这里，机制与类型在 kernel）。
//! 管：FencePlan::for_call / for_session / standalone——把沙箱与模块事实变成可达范围；裁决卡材料（fence_ask 与选项 id）。
//! 不管：描述符自身的形状（在 kernel::contracts::fence）、裁决与回执文案（在 service/fence.rs）、平台机制（在 kernel::detail::confine）。
//! 联动：经 tools::api 给成员循环与协调业务用；kernel 不反向依赖本模块。

use crate::kernel::api::fence::{Access, FenceBlocked, FencePlan, Grant, NetPolicy, Reach};
use crate::kernel::api::types::Ask;
use crate::units::business::workspace::api::Sandbox;
use std::path::{Path, PathBuf};

impl FencePlan {
    /// 目的：为**一次调用**派生围栏；base 是同一会话的 for_session 结果（提供主机只读根与全局断网事实）。
    /// 约束：授予是固定的——私有沙箱恒可写（`home` 隐含）、模块目录恒递归只读、主机只读根恒单层只读、
    ///   共享区永不进授予（提交走 work_commit）；唯一随调用变的是"这次调的是哪个模块"
    ///   （该模块自己的 userdata 加进可写）与全局网络开关。
    pub fn for_call(sb: &Sandbox, module: Option<&str>, base: &FencePlan) -> FencePlan {
        let mut plan = FencePlan {
            agent: sb.agent.clone(),
            lease: sb.session.clone(),
            home: sb.private.clone(),
            cwd: PathBuf::new(),
            net: base.net.clone(),
            grants: base
                .paths(Access::Read, Reach::Dir)
                .into_iter()
                .map(Grant::read)
                .collect(),
        };
        plan.grants
            .extend(sb.modules.values().map(Grant::read_tree));
        if let Some(id) = module
            && let Some(private) = sb.module_data.get(id)
        {
            plan.grants.push(Grant::write_tree(private));
        }
        plan.normalize();
        plan
    }

    /// 目的：为一个会话派生**并集围栏**——覆盖它可能签发过的全部落点，用于撤销持久授权与生命周期。
    pub fn for_session(sb: &Sandbox, host_roots: &[PathBuf], global_net: bool) -> FencePlan {
        let mut plan = FencePlan {
            agent: sb.agent.clone(),
            lease: sb.session.clone(),
            home: sb.private.clone(),
            cwd: PathBuf::new(),
            net: NetPolicy::from_flag(global_net && sb.permissions.allows_network()),
            grants: host_roots.iter().cloned().map(Grant::read).collect(),
        };
        plan.grants
            .extend(sb.modules.values().map(Grant::read_tree));
        plan.grants
            .extend(sb.module_data.values().map(Grant::write_tree));
        plan.normalize();
        plan
    }

    /// 目的：**无会话**（人直接跑一个模块工具 / 常驻服务）的围栏——模块源码递归只读 + 它的私有区可写。
    /// 参数：`module_root` = 模块源码根（`modules/<id>`）；`private` = 私有区根
    ///   （`data/modules/<id>/userdata`，由 `content` 单元给）；`work_root` = 替代 HOME 的工作根。
    /// 约束：这是模块自身进程的口径；agent 的内置文件工具对模块目录（含私有区）只读。
    pub fn standalone(
        module_root: &Path,
        private: Option<&Path>,
        work_root: Option<&Path>,
    ) -> FencePlan {
        let home = match work_root {
            Some(p) => p.to_path_buf(),
            None => private.map(|p| p.to_path_buf()).unwrap_or_default(),
        };
        let mut plan = FencePlan {
            agent: "user".to_string(),
            lease: String::new(),
            home,
            cwd: module_root.to_path_buf(),
            net: NetPolicy::Off,
            grants: vec![Grant::read_tree(module_root)],
        };
        // 有私有区且它没被 `home` 覆盖时，单独授一份写（模块自己的数据目录）。
        if let Some(private) = private
            && plan.home != private
        {
            plan.grants.push(Grant::write_tree(private));
        }
        plan.normalize();
        plan
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
        title: format!("围栏的{}装不上，这次调用怎么跑？", block.what),
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
        // 没人答就是没放行：发起方按 fail-closed 收场（不执行这次调用）。
        kind: "fence".to_string(),
        payload: serde_json::json!({ "fence": block.what, "line": block.line() }),
        notes: Vec::new(),
    })
}
