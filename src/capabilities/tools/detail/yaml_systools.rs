//! 系统工具与角色的装配输入：`systools/tools.yaml`（工具是什么）+ `systools/roles.yaml`（身份有什么）。
//! **它属于工具能力**（工具总表与角色表是工具侧的事实，不是提示词）：
//! 放在提示词能力里会让 `prompt → tools` 成环（见 ARCHITECTURE.md §一）。
//! 缺目录/缺文件 = 装配错误（如实报错，不静默造默认）。

use crate::capabilities::tools::api::{RoleTable, SystemTools, ToolBook};
use crate::capabilities::tools::ports::SystoolsSource;
use serde::Deserialize;
use std::path::PathBuf;

/// 工具总表的文件形状。
#[derive(Deserialize)]
struct ToolFile {
    tools: ToolBook,
}

/// 角色表的文件形状。
#[derive(Deserialize)]
struct RoleFile {
    roles: RoleTable,
}

pub struct YamlSystools {
    dir: PathBuf,
}

impl YamlSystools {
    pub fn new(dir: PathBuf) -> YamlSystools {
        YamlSystools { dir }
    }
}

impl SystoolsSource for YamlSystools {
    /// 系统工具与角色：读 `tools.yaml`（工具是什么）与 `roles.yaml`（身份有什么）。
    fn load(&self) -> Result<SystemTools, String> {
        let path = self.dir.join("tools.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("工具总表读不了（systools/tools.yaml）：{}", e))?;
        let tools: ToolFile = serde_yaml::from_str(&text)
            .map_err(|e| format!("工具总表非法（systools/tools.yaml）：{}", e))?;
        if tools.tools.is_empty() {
            return Err("工具总表里一个工具都没有（systools/tools.yaml）".to_string());
        }
        let path = self.dir.join("roles.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("角色表读不了（systools/roles.yaml）：{}", e))?;
        let roles: RoleFile = serde_yaml::from_str(&text)
            .map_err(|e| format!("角色表非法（systools/roles.yaml）：{}", e))?;
        Ok(SystemTools {
            tools: tools.tools,
            roles: roles.roles,
        })
    }
}
