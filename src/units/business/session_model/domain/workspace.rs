//! 目的：会话侧的**工作台声明**——会话只说「要一个什么样的工作台」，不说它怎么实现。
//! 管：`WorkspaceDecl`（策略 id + 外部根 + 不透明参数）与 `LAYOUT_POLICY`（今天的唯一布局策略 id）。
//! 不管：布局怎么铺、共享区归属怎么算、版本库怎么提交（都在 `workspace` 单元）；权限档位与网络（在 `permission`）。
//! 联动：`SessionMeta.workspace` 落盘它；`workspace` 单元按 `policy` 选实现、按 `params` 解释自己的参数。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 目的：今天的唯一布局策略 id：产品内的工作目录 + 会话目录 + 共享区。
pub const LAYOUT_POLICY: &str = "layout";

/// 目的：会话要的工作台：用哪套策略、主副本落在哪、策略自己的参数是什么。
/// 约束：`params` 是**不透明**的 JSON——会话不认里面的字段，写进去什么由策略自己解释。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceDecl {
    /// 目的：布局策略 id（策略表在 `workspace` 单元）。
    #[serde(default = "default_policy")]
    pub policy: String,
    /// 目的：共享区主副本绑定的**外部**根；`None` = 产品内的 `session/<名>/work`。
    /// 约束：只写在工作根上，子会话不写；创建后不可改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    /// 目的：策略自己的参数（今天装共享区归属的会话覆盖）。
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

fn default_policy() -> String {
    LAYOUT_POLICY.to_string()
}

impl Default for WorkspaceDecl {
    /// 目的：默认就是产品内布局策略（空串不是合法策略 id）。
    fn default() -> Self {
        WorkspaceDecl {
            policy: default_policy(),
            root: None,
            params: serde_json::Value::Null,
        }
    }
}

impl WorkspaceDecl {
    /// 目的：一套只按产品内布局的工作台声明（没有外部根、没有参数）。
    pub fn layout() -> WorkspaceDecl {
        WorkspaceDecl::default()
    }

    /// 目的：带外部工作区根的声明。
    /// 参数：`root` = 已解析、已校验的绝对路径。
    pub fn at(root: PathBuf) -> WorkspaceDecl {
        WorkspaceDecl {
            root: Some(root),
            ..WorkspaceDecl::default()
        }
    }
}
