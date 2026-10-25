//! 目的：构建能力的机制细节——源指纹与构建状态落盘。
//! 管：`fingerprint`（源指纹、`.vm/build-state/<模块>.json` 的读与写）。
//! 不管：什么时候该重编（在 `service.rs`，按入口在场与目标比对）；选包与围栏（在 `service.rs`）。
//! 联动：落点与字段见 MODULE_SPEC 的「构建·缓存」。

pub(crate) mod fingerprint;
