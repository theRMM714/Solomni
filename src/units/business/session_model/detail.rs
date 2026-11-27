//! 目的：本单元自己的适配器——把两份声明 yaml 读成装好的声明。
//! 管：`yaml_decls`（流程段表与会话类型表的读取、解析与装配期校验）。
//! 不管：领域形状与规则（在 `domain/`）；读盘（`resource::assets` 的端口）。
//! 联动：只有入口层的组合根构造它；别的单元看到的是 `SessionDecls`。

pub mod yaml_decls;
