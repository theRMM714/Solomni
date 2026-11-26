//! 目的：地基库的对外面——按「对外概念」分文件，一个文件一个面；单元只经这里取形状与机制。
//! 管：二十个面的模块声明（`fence` / `note` / `run` / `types` / `clock` / `fs_meta` / `host_probe` / `i18n` / `locus` / `process` / `procs` / `lock` / `view_drive` / `actor` / `jobs` / `layout` / `name` / `net` / `path` / `platform`）。
//! 不管：任何条目的实现（在 `contracts/`、`domain/`、`detail/`）；也不给内核内部用——内核内部直接取所属机制，绕道本面由 `structure/kernel-mechanisms` 拦住。
//! 联动：消费方是核心（`src/units/runtime/api/`）与各单元；面的取舍见 `docs/kernel/README.md`「对外面」。

pub mod actor;
pub mod clock;
pub mod fence;
pub mod fs_meta;
pub mod host_probe;
pub mod i18n;
pub mod jobs;
pub mod layout;
pub mod lock;
pub mod locus;
pub mod name;
pub mod net;
pub mod note;
pub mod path;
pub mod platform;
pub mod process;
pub mod procs;
pub mod run;
pub mod types;
pub mod view_drive;
