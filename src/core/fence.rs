//! 一次工具执行的围栏（纯数据）：由该 agent 的沙箱与执行档位派生，机制在 adapters（confine）。
//! 权限方位类以 agent 为界：可读可写的只有本次工作的共享区与该 agent 的私有沙箱，
//! 成员模块目录随它（读写，回执里如实提示）；其余一律不可达——这是**策略**，装在哪个平台用什么机制由适配层定。
//! 本机档与虚拟机档共用这份围栏：虚拟机档的 guest 内视图由装配阶段按同一批根组装。

use crate::core::workspace::Sandbox;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 守门进程要执行的命令与其环境上下文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FenceSpec {
    /// 该 agent 的实例名（日志与审计用）。
    pub agent: String,
    /// 可读可写的根：本次工作共享区 + 该 agent 私有沙箱 + 它自己的模块目录。
    pub rw: Vec<PathBuf>,
    /// 工具进程的工作目录（它所属模块的根目录）。
    pub cwd: PathBuf,
    /// 是否放行出站网络（默认否）。
    pub net: bool,
}

impl FenceSpec {
    /// 从该 agent 的沙箱派生（模块目录按模块 id 升序，顺序稳定；同一模块不会同属两个 agent）。
    pub fn from_sandbox(sb: &Sandbox, net: bool) -> FenceSpec {
        let mut rw = vec![sb.shared.clone(), sb.private.clone()];
        rw.extend(sb.modules.values().cloned());
        FenceSpec { agent: sb.agent.clone(), rw, cwd: PathBuf::new(), net }
    }

    /// 把这个围栏的工作目录设成某个模块的根（该模块的工具就在这里跑）。
    pub fn at(&self, module_root: &std::path::Path) -> FenceSpec {
        let mut out = self.clone();
        out.cwd = module_root.to_path_buf();
        out
    }

}
