//! 目的：登记处的纯定义——文档名与它在这块盘上的形态。
//! 管：`Doc` 的取值、文件名、是否私密（unix 0600）。
//! 不管：文件机制（在 detail）；yaml 解码（在 catalog）。
//! 联动：api 重导出 `Doc`；service 用它把 DTO 映射成文件名。

pub mod doc;
