//! 一次工具执行的围栏（纯数据）：由该 agent 的沙箱与执行档位派生，机制在 adapters（confine）。
//! 权限方位类以 agent 为界：可读可写的只有本次工作的共享区与该 agent 的私有沙箱，
//! 成员模块目录随它（读写，回执里如实提示）；其余一律不可达——这是**策略**，装在哪个平台用什么机制由适配层定。
//! `ro` 是**用户显式授权**的只读根（`.home/settings.yaml` 的 `fence_read`）：只读、不继承写，
//! 默认空 = 一个都不放行（与 `fence_write` 同一套哲学：没经用户同意就不动本机任何权限项）。
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
    /// 只读的根：**用户显式授权**的额外可达范围（默认空）。
    /// 只读位由各平台机制落实（Landlock 只读位 / seatbelt `file-read*` / Windows `RIGHTS_RO`），
    /// 且**必须授给该 agent 自己的容器身份**，不能像解释器基线那样授给共享组（那等于把用户数据开放给机器上任意容器）。
    #[serde(default)]
    pub ro: Vec<PathBuf>,
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
        FenceSpec {
            agent: sb.agent.clone(),
            rw,
            ro: Vec::new(),
            cwd: PathBuf::new(),
            net,
        }
    }

    /// 挂上用户显式授权的只读根（策略层只带事实；只读位怎么落由适配层定）。
    pub fn with_read_only(mut self, ro: Vec<PathBuf>) -> FenceSpec {
        self.ro = ro;
        self
    }

    /// 把这个围栏的工作目录设成某个模块的根（该模块的工具就在这里跑）。
    pub fn at(&self, module_root: &std::path::Path) -> FenceSpec {
        let mut out = self.clone();
        out.cwd = module_root.to_path_buf();
        out
    }
}
