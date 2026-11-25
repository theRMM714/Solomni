//! 目的：`systools/` 下的固定文档清单——一份文档一个变体，文件名只在这里出现。
//! 管：文档名与文件名的一一对应。
//! 不管：文档形态（各消费方的 `detail` 里）；读盘（`service` 经端口）。
//! 联动：`tools` 读 `SystoolsTools` / `SystoolsNames`，`conversation` 读 `SystoolsProfiles`；契约见 docs/assets/README.md。

/// 目的：`systools/` 下可读的固定文档。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doc {
    /// 工具总表（`tools.yaml`）：工具是什么、收哪些参数。
    SystoolsTools,
    /// 名字表（`names.yaml`）：agent 实例名与模块 id 的保留名。
    SystoolsNames,
    /// 会话类型表（`profiles.yaml`）：一个会话类型就是一串流程与它的静止机制。
    SystoolsProfiles,
}

impl Doc {
    /// 目的：文档在 `systools/` 下的文件名。
    /// 返回：静态文件名，如 `tools.yaml`。
    pub fn file_name(self) -> &'static str {
        match self {
            Doc::SystoolsTools => "tools.yaml",
            Doc::SystoolsNames => "names.yaml",
            Doc::SystoolsProfiles => "profiles.yaml",
        }
    }

    /// 目的：消费者写错误文案时用的相对路径（`systools/<文件名>`）。
    /// 返回：静态相对路径，不含宿主目录。
    pub fn label(self) -> &'static str {
        match self {
            Doc::SystoolsTools => "systools/tools.yaml",
            Doc::SystoolsNames => "systools/names.yaml",
            Doc::SystoolsProfiles => "systools/profiles.yaml",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_document_has_a_name_and_a_label() {
        for doc in [Doc::SystoolsTools, Doc::SystoolsNames, Doc::SystoolsProfiles] {
            assert!(doc.file_name().ends_with(".yaml"));
            assert!(doc.label().starts_with("systools/"));
        }
    }
}
