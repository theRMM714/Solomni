//! 目的：工作区单元测试：沙箱寻址与共享区版本化。
//! 管：本单元的不变式——寻址、外部绑定、版本化工作流。
//! 不管：虚拟机装配（`src/tests/vm`）、runtime 编排。
//! 联动：`docs/storage/README.md` 与 `ARCHITECTURE.md`。

mod archive;
mod config;
mod external;
mod filter;
mod gate;
mod ignore;
mod resolve;
mod support;
mod work;
