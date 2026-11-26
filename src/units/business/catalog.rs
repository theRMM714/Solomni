//! 目的：catalog 单元——登记、打包与密钥这些静态事实放在一处，供上层按需取用。
//! 管：`modules` 的清单契约与启停、`packages` 的运行包库、`secrets` 的隐秘字段、`registry` 的 providers / models / agents / settings 登记与 agent 身份（`AgentMeta`）。
//! 不管：会话与协作（`conversation`）、一轮对话的编排（`turn`）、构建产物与存储回收（`maintenance`）、工具执行与虚拟机（`tools` / `vm`）。
//! 联动：单元内子业务各自保留四分区；跨单元引用经 `catalog::<子业务>::api`（见 docs/modules/README.md、docs/registry/README.md、docs/packages/README.md、docs/secrets/README.md）。
pub mod modules;
pub mod packages;
pub mod registry;
pub mod secrets;
