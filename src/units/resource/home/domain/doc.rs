//! 目的：登记处文档的清单——`data/.home` 下允许出现的六份文件。
//! 管：文档名的枚举、文件名、私密标记。
//! 不管：内容形态与字段（在各业务单元自己的 domain）。
//! 联动：`api::Home` 的入参；文件名只在这里写一次。

/// 目的：登记处的一份文档；新增要落在 `data/.home` 的文件必须先在这里加一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doc {
    /// 目的：供应商与通道密钥。
    Providers,
    /// 目的：模型目录与核心默认模型。
    Models,
    /// 目的：基本设置（含沙箱策略）。
    Settings,
    /// 目的：agent 登记。
    Agents,
    /// 目的：模块声明的隐秘字段值。
    Secrets,
    /// 目的：模块启停覆盖。
    Modules,
}

impl Doc {
    /// 目的：这份文档在登记处目录下的文件名。
    pub fn file_name(self) -> &'static str {
        match self {
            Doc::Providers => "providers.yaml",
            Doc::Models => "models.yaml",
            Doc::Settings => "settings.yaml",
            Doc::Agents => "agents.yaml",
            Doc::Secrets => "secrets.yaml",
            Doc::Modules => "modules.yaml",
        }
    }

    /// 目的：是否当私密文件对待（unix 下收紧到 0600）；其余平台由机制层空操作。
    /// 约束：含密钥的两份必须为真；其余沿用历史形态（模块启停覆盖原本就是普通权限）。
    pub fn is_private(self) -> bool {
        !matches!(self, Doc::Modules)
    }
}
