//! 目的：根（启动、组合根、探针与守门进程）共用的机制——只给程序入口用。
//! 管：产品根规范化（`root`）、启动期基础工具链 PATH（`toolchains`）、组合根共用装配（`assembly`）与机器探针（`probes`）。
//! 不管：任何业务语义；启动流程本身（在 `src/main.rs`）。
//! 联动：消费者是 `src/main.rs`、`src/guard/` 与测试。

pub mod assembly;
pub mod probes;
pub mod root;
pub mod toolchains;
