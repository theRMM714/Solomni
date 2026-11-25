//! 目的：系统工具与布局保留名的装配输入——`systools/tools.yaml`（工具是什么）+ `systools/names.yaml`（布局保留名）。
//! 管：从目录读两份 yaml 并反序列化成 `ToolBook` / 保留名；缺目录 / 缺文件如实报错，不静默造默认。
//! 不管：表本体与校验规则（在 `domain`）；装配（在入口层组合根）。
//! 联动：实现 `ports::SystoolsSource`；它属于工具单元（放提示词单元会让 `prompt → tools` 成环，见 docs/architecture/layers.md）。

use crate::units::tools::api::{SystemTools, ToolBook};
use crate::units::tools::ports::SystoolsSource;
use serde::Deserialize;
use std::path::PathBuf;

/// 工具总表的文件形状。
#[derive(Deserialize)]
struct ToolFile {
    tools: ToolBook,
}

/// 名字表的文件形状：两块保留名——agent 实例名（布局目录）与模块 id（动作 id 前缀）。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamesFile {
    #[serde(default)]
    agents: Vec<String>,
    #[serde(default)]
    modules: Vec<String>,
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
    /// 系统工具与保留名：读 `tools.yaml`（工具是什么）与 `names.yaml`（布局保留名）。
    fn load(&self) -> Result<SystemTools, String> {
        let path = self.dir.join("tools.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("工具总表读不了（systools/tools.yaml）：{}", e))?;
        let tools: ToolFile = yaml_serde::from_str(&text)
            .map_err(|e| format!("工具总表非法（systools/tools.yaml）：{}", e))?;
        if tools.tools.is_empty() {
            return Err("工具总表里一个工具都没有（systools/tools.yaml）".to_string());
        }
        let path = self.dir.join("names.yaml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("名字表读不了（systools/names.yaml）：{}", e))?;
        let names: NamesFile = yaml_serde::from_str(&text)
            .map_err(|e| format!("名字表非法（systools/names.yaml）：{}", e))?;
        if names.agents.is_empty() {
            return Err(
                "名字表里 agent 保留名一块是空的（systools/names.yaml 的 agents）".to_string(),
            );
        }
        if names.modules.is_empty() {
            return Err(
                "名字表里模块保留名一块是空的（systools/names.yaml 的 modules）".to_string(),
            );
        }
        Ok(SystemTools {
            tools: tools.tools,
            reserved_agents: names.agents,
            reserved_modules: names.modules,
        })
    }
}
