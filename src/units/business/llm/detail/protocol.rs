//! 目的：供应商协议的实现模块根——wire 形状只在这里。
//! 管：声明各协议实现（现在只有 openai）与「按通道选协议」的唯一选择点。
//! 不管：出站机制（http_chat / http_stream）；协议端口的定义（在 ports）。
//! 联动：http_chat / http_stream / http_probe 经 ports::Protocol 调它；新增协议在这里加一支选择。

pub mod openai;

use crate::units::business::llm::ports::Protocol;
use std::sync::Arc;

/// 目的：**唯一的协议选择点**：给一条通道挑一种 wire 形状。
///    现在登记处还没有协议字段（只有一种实现），所以恒定返回 OpenAI 兼容实现。
///    新增协议时：在 registry 声明 + 在这里按通道选 + 加一个实现文件，调用方与流机制都不用动。
pub(crate) fn for_channel(
    _channel: &crate::units::business::llm::api::Channel,
) -> Arc<dyn Protocol> {
    Arc::new(openai::new())
}

/// 目的：不依赖通道的默认实现（探针在拿到通道之前也要用同一份形状）。
pub(crate) fn default_protocol() -> Arc<dyn Protocol> {
    Arc::new(openai::new())
}
