//! 目的：虚拟机盘单元——`data/.vm` 的布局与命名所有权（构建缓存、下载缓存、可写层、构建状态、虚拟根、便携引擎、台账）。
//! 管：`.vm` 下每个子目录与文件名的拼法（含路径段的安全化）。
//! 不管：怎么用这些文件（构建在 `builds`、补齐在 `provision`、盘点在 `storage`、引擎在 `vm`）；介质根名（在 `kernel` 的 `layout`）。
//! 联动：各单元的 detail 适配器从自己的产品根经 `api::paths` 取落点；台账 `data/state` 同层规则见 docs/architecture/state.md。

pub mod api;
pub mod domain;
