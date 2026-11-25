//! 目的：装配一席要用的能力面参数对象——组合根注入的同一批端口，按字段交给本单元的装配机。
//! 管：`SessionPorts`（llm / 日志 / 提示词册 / 工具总表 / 工具执行 / 虚拟机 / 常驻服务 / 工作区 / 压缩通道）。
//! 不管：装配逻辑本身（在 `domain::assembly`）、端口的实现（各单元自己）。
//! 联动：`Turn::session_ports` 按自己持有的那份克隆出来；装配与重建的每个入口都收它——它与装配机同单元，否则环 2 内成环。

use std::sync::Arc;

use crate::kernel::ports::Log;
use crate::units::agentloop::api::CompactChatFactory;
use crate::units::llm::api::Llm;
use crate::units::prompt::api::Prompt;
use crate::units::residents::api::ResidentOps;
use crate::units::tools::api::{ToolExec, Tools};
use crate::units::vm::api::Vm;
use crate::units::workspace::api::Workspace;

/// 目的：装配一场会话要用的能力面（组合根注入的同一批端口）。
pub struct SessionPorts {
    /// 目的：llm 用例面：一席的收发句柄按它造。
    pub llm: Arc<dyn Llm + Send + Sync>,
    /// 目的：日志面：装配时记一条「单 agent 工作」。
    pub log: Arc<dyn Log + Send + Sync>,
    /// 目的：提示词册：工具说明与压缩提示从它取段。
    pub prompt: Arc<dyn Prompt>,
    /// 目的：工具总表：席位面合成与内置工具表都从它读。
    pub systools: Arc<dyn Tools>,
    /// 目的：工具执行面：一席的工具环境要它跑外部 / 内置工具。
    pub tools: Arc<dyn ToolExec + Send + Sync>,
    /// 目的：虚拟机用例面：一席的 guest 事实随工具面交给机制层。
    pub vm: Arc<dyn Vm + Send + Sync>,
    /// 目的：常驻服务管理面：已就绪服务的操作并入一席的工具面。
    pub residents: Arc<dyn ResidentOps + Send + Sync>,
    /// 目的：工作区用例面：一席的 `work_*` 工具执行者按它造。
    pub workspace: Arc<dyn Workspace + Send + Sync>,
    /// 目的：压缩通道工厂：新会话与重建会话共用同一份。
    pub compact: Arc<dyn CompactChatFactory>,
}
