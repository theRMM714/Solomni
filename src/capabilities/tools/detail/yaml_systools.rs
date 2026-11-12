//! 目的：系统工具、角色与布局保留名的装配输入——`systools/tools.yaml`（工具是什么）+ `systools/roles.yaml`（身份有什么）+ `systools/names.yaml`（布局保留名）。
//! 管：从目录读三份 yaml 并反序列化成 `ToolBook` / `RoleTable` / 保留名；缺目录 / 缺文件如实报错，不静默造默认。
//! 不管：表本体与校验规则（在 `domain`）；装配（在入口层组合根）。
//! 联动：实现 `ports::SystoolsSource`；它属于工具能力（放提示词能力会让 `prompt → tools` 成环，见 ARCHITECTURE.md §一）。

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

/// 名字表的文件形状：目录保留名（agent 实例名不得占用）。
#[derive(Deserialize)]
struct NamesFile {
    #[serde(default)]
    reserved: Vec<String>,
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
    /// 系统工具、角色与保留名：读 `tools.yaml`（工具是什么）、`roles.yaml`（身份有什么）与 `names.yaml`（布局保留名）。
    fn load(&self) -> Result<SystemTools, String> {
        let path = self.dir.join("tools.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("工具总表读不了（systools/tools.yaml）：{}", e))?;
        let tools: ToolFile = yaml_serde::from_str(&text)
            .map_err(|e| format!("工具总表非法（systools/tools.yaml）：{}", e))?;
        if tools.tools.is_empty() {
            return Err("工具总表里一个工具都没有（systools/tools.yaml）".to_string());
        }
        let path = self.dir.join("roles.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("角色表读不了（systools/roles.yaml）：{}", e))?;
        let roles: RoleFile = yaml_serde::from_str(&text)
            .map_err(|e| format!("角色表非法（systools/roles.yaml）：{}", e))?;
        let path = self.dir.join("names.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("名字表读不了（systools/names.yaml）：{}", e))?;
        let names: NamesFile = yaml_serde::from_str(&text)
            .map_err(|e| format!("名字表非法（systools/names.yaml）：{}", e))?;
        if names.reserved.is_empty() {
            return Err("名字表里一个保留名都没有（systools/names.yaml）".to_string());
        }
        Ok(SystemTools {
            tools: tools.tools,
            roles: roles.roles,
            reserved_names: names.reserved,
        })
    }
}
