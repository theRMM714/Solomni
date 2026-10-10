//! 目的：常驻服务的出站端口（只有 service.rs 持有，R12）：协议适配器。
//! 管：ServiceAdapter 与它拉起的 ServiceInstance 的形状；LaunchSpec 是策略侧给的事实。
//! 不管：生命周期、租约、开关（在 manager/service）；具体协议实现（在 detail 适配器，只由组合根构造）。
//! 联动：端口形状见本文件；由 service.rs 消费。
#![allow(dead_code)] // 真实适配器随 MCP/ACP 接入；在此之前端口只被契约测试的假适配器实现。

use crate::capabilities::residents::api::Operation;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 目的：拉起一个服务实例所需的事实（策略在 manager：哪个模块的哪个服务、命令、工作目录、私有配置）。
pub struct LaunchSpec {
    pub module: String,
    pub name: String,
    pub command: String,
    pub cwd: PathBuf,
    pub options: BTreeMap<String, String>,
    /// 目的：该模块已配置的隐秘字段注入项（env 名 → 值）；只走进程环境，不进命令行。
    pub env: Vec<(String, String)>,
}

/// 目的：一个已拉起的服务实例——适配器负责协议，manager 负责生命周期与串行调用。
pub trait ServiceInstance: Send {
    /// 目的：一次操作调用；适配器把结果转成文本。
    fn call(&mut self, op: &str, args: &serde_json::Value) -> Result<String, String>;
    /// 目的：关闭这个实例（连接与进程由适配器收尾）。
    fn stop(&mut self);
}

/// 目的：协议适配器——一个适配器认领一种协议；它只由组合根构造并注入。
pub trait ServiceAdapter: Send + Sync {
    /// 这个适配器的 id（与 module.yaml 的 services 声明的 adapter 匹配）。
    fn id(&self) -> &str;
    /// 目的：按声明拉起一个实例并握手，返回它提供的操作。
    fn start(
        &self,
        spec: &LaunchSpec,
    ) -> Result<(Box<dyn ServiceInstance>, Vec<Operation>), String>;
}
