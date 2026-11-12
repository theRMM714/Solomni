//! 目的：会话能力测试的入口：按不变式分文件（提示词/建立/回档/回合/编辑/虚拟机档/树与配置）。
//! 管：子模块声明与公共导入。
//! 不管：各族用例细节；替身与装配见 `tests::doubles`、`tests::builders`。
//! 联动：契约见 docs/session/session-model.md。

mod edit;
mod lifecycle;
mod prompt;
mod rewind;
mod tree;
mod turn;
mod vm;
