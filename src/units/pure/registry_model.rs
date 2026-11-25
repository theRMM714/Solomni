//! 目的：登记处的读模型——agent、供应商、模型与设置项的值形态，脱离 IO 与用例。
//! 管：`api` 对外形状与校验，`domain` 的实现。
//! 不管：写 yaml 与密钥（在 catalog 的 detail/）；名单落地与模型清单（在 catalog 的 domain/picks.rs）；登记处用例（在 catalog 的 service.rs）。
//! 联动：catalog（环 2）、turn/runtime（环 3）与呈现层都经 `registry_model::api` 取 agent 与模型形状。

pub mod api;
pub mod domain;
