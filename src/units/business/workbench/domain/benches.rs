//! 目的：一次工作内全部席位的工作面（发言席就是 agent，工作面与 agent 一一对应）。
//! 管：`Benches`（共享区根、会话树根、每席工作面、路径翻译器）与按 agent 取席。
//! 不管：这些值怎么组装（在 `workspace` 的席位组装处）；文件清单与用量（在 `workspace` 端口）。
//! 联动：协作 / 流程 / 工作单元持它；提示词与 @ 改写经它取根。

use crate::units::business::workbench::domain::bench::Bench;
use std::path::PathBuf;
use std::sync::Arc;

/// 目的：一次工作内的全部 agent 工作面。
#[derive(Debug, Clone, Default)]
pub struct Benches {
    /// 目的：本工作共享区的真实根（@ 改写要它；代拟还没名单时也有）。
    pub shared: PathBuf,
    /// 目的：本次工作的**会话树根**（`session/<顶层工作名>`）——各 agent 私沙与 `.work` 的父目录。
    /// 约束：布局归工作区单元；虚拟机档据此挂稳定根，不靠 `shared.parent()` 猜（外部绑定时后者不是会话树）。
    pub session_root: PathBuf,
    /// 目的：每席的工作面（顺序即名单顺序）。
    pub list: Vec<Bench>,
    /// 目的：这场会话的路径翻译器（虚拟机档 guest↔宿主；本机档 None = 恒等）。
    /// 约束：@ 改写发生在名单确认**之前**（协作需求），那时 `list` 还是空的，所以这里单独存一份。
    pub codec: Option<Arc<dyn crate::kernel::ports::PathCodec>>,
}

impl Benches {
    /// 目的：某 agent 实例的工作面；不存在 = 该 agent 没被分配工作区（调用方如实报错）。
    pub fn for_agent(&self, agent: &str) -> Option<&Bench> {
        self.list.iter().find(|b| b.agent == agent)
    }
}
