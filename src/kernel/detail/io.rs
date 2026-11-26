//! 目的：IO 面——直接碰 OS、网络、进程与时钟的唯一一层。
//! 管：声明 IO 子模块（`env`：进程环境；`net`：出站网络与入站回环）；子模块只放原语，不带业务语义。
//! 不管：谁在什么场景调用（业务单元）、请求与响应的语义、重试与错误归类。
//! 联动：对外面在 `src/kernel/api/`；机制分层见 `docs/kernel/mechanisms.yaml`；单元地图见 `docs/kernel/unit-map.md`。

pub mod env;
pub mod net;
