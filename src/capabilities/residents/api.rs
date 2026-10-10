//! 目的：常驻服务的入站能力面——统一管理 API（协议中立：核心不认识 MCP / ACP）。
//! 管：ResidentOps 与它的 DTO（服务视图、操作、状态、回执）；未注入时的空实现 NoResidents。
//! 不管：适配器端口（在 `ports.rs`，不进 api）；具体协议；模块声明的解析（在 workspace）。
//! 联动：实现见 `service.rs`；由 `main.rs` 装配进 `Ops`，呈现层只经 `ResidentOps` 调用。
#![allow(dead_code)] // 生产接线（动作面 / 租约回收 / 真实适配器）随 Phase 1 剩余项接入；在此之前只被契约测试驱动。

use crate::capabilities::workspace::api::Param;
use std::collections::BTreeMap;

/// 目的：一次服务操作的事实回执（ok = 成功；output 由适配器给出）。
#[derive(Debug)]
pub struct Receipt {
    pub ok: bool,
    pub output: String,
}

/// 目的：一个由常驻服务提供的操作（名字 + 说明 + 可选参数契约）。
#[derive(Debug, Clone)]
pub struct Operation {
    pub name: String,
    pub description: String,
    pub params: Option<BTreeMap<String, Param>>,
}

/// 目的：一个常驻服务此刻的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    /// 用户关掉了：不启动。
    Disabled,
    /// 已启用但未启动。
    Stopped,
    /// 已拉起、可调用。
    Ready,
    /// 启动 / 握手失败的如实原因。
    Failed(String),
}

impl ServiceState {
    /// 目的：给人看的状态名（失败带原因）。
    pub fn label(&self) -> String {
        match self {
            ServiceState::Disabled => "已禁用".to_string(),
            ServiceState::Stopped => "未启动".to_string(),
            ServiceState::Ready => "运行中".to_string(),
            ServiceState::Failed(why) => format!("失败：{}", why),
        }
    }
}

/// 目的：给呈现层与工具层看的服务视图（不携带任何协议细节）。
#[derive(Debug, Clone)]
pub struct ServiceView {
    pub module: String,
    pub name: String,
    pub adapter: String,
    pub state: ServiceState,
    pub operations: Vec<Operation>,
    /// 目的：不可用时的如实原因（未知适配器 / 缺声明等）；可用时为空串。
    pub reason: String,
}

/// 目的：常驻服务的**唯一管理 API**——整个项目只从这里调用。
/// 参数：module + name 定位声明；start 的 lease 是本次实例的会话租约（空 = 无会话，进程级）。
/// 返回：start 给出发现到的操作；call 给出事实回执；其余给出成没成。
/// 约束：协议语义全在适配器；本 API 只表达「服务、操作、状态、租约」，不出现任何协议词。
pub trait ResidentOps: Send + Sync {
    fn services(&self) -> Result<Vec<ServiceView>, String>;
    fn start(&self, module: &str, name: &str, lease: &str) -> Result<Vec<Operation>, String>;
    fn stop(&self, module: &str, name: &str) -> Result<(), String>;
    fn set_enabled(&self, module: &str, name: &str, enabled: bool) -> Result<(), String>;
    fn call(
        &self,
        module: &str,
        name: &str,
        op: &str,
        args: &serde_json::Value,
    ) -> Result<Receipt, String>;
    fn reap(&self, lease: &str) -> Result<(), String>;
}

/// 目的：没有配置任何常驻服务时的空实现（组合根未注入时的默认；服务目录本就是空的，不是静默兜底）。
pub struct NoResidents;

impl ResidentOps for NoResidents {
    fn services(&self) -> Result<Vec<ServiceView>, String> {
        Ok(Vec::new())
    }
    fn start(&self, module: &str, name: &str, _lease: &str) -> Result<Vec<Operation>, String> {
        Err(format!("没有配置任何常驻服务：{}/{}", module, name))
    }
    fn stop(&self, _module: &str, _name: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_enabled(&self, _module: &str, _name: &str, _enabled: bool) -> Result<(), String> {
        Ok(())
    }
    fn call(
        &self,
        module: &str,
        name: &str,
        _op: &str,
        _args: &serde_json::Value,
    ) -> Result<Receipt, String> {
        Err(format!("没有配置任何常驻服务：{}/{}", module, name))
    }
    fn reap(&self, _lease: &str) -> Result<(), String> {
        Ok(())
    }
}
