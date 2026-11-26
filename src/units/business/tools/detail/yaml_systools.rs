//! 目的：系统工具与布局保留名的装配输入——`systools/tools.yaml`（工具是什么）+ `systools/names.yaml`（布局保留名）。
//! 管：经 `Assets` 读两份文档并反序列化成 `ToolBook` / 保留名；缺文件 / 非法如实报错，不静默造默认。
//! 不管：表本体与校验规则（在 `domain`）；目录与读盘（在 `assets`）；装配（在入口层组合根）。
//! 联动：实现 `ports::SystoolsSource`；它属于工具单元（放提示词单元会让 `prompt → tools` 成环，见 docs/architecture/layers.md）。

use crate::units::business::tools::api::{SystemTools, ToolBook};
use crate::units::business::tools::ports::SystoolsSource;
use crate::units::resource::assets::api::{Assets, Doc};
use serde::Deserialize;
use std::sync::Arc;

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
    assets: Arc<dyn Assets>,
}

impl YamlSystools {
    /// 目的：`assets` = 随包只读内容的入站面（`systools/` 的落点与读法归它）。
    pub fn new(assets: Arc<dyn Assets>) -> YamlSystools {
        YamlSystools { assets }
    }
}

impl SystoolsSource for YamlSystools {
    /// 系统工具与保留名：读 `tools.yaml`（工具是什么）与 `names.yaml`（布局保留名）。
    fn load(&self) -> Result<SystemTools, String> {
        let text = self
            .assets
            .read(Doc::Tools)
            .map_err(|e| format!("工具总表读不了（{}）：{}", Doc::Tools.label(), e))?;
        let tools: ToolFile = yaml_serde::from_str(&text)
            .map_err(|e| format!("工具总表非法（{}）：{}", Doc::Tools.label(), e))?;
        if tools.tools.is_empty() {
            return Err(format!(
                "工具总表里一个工具都没有（{}）",
                Doc::Tools.label()
            ));
        }
        let text = self
            .assets
            .read(Doc::Names)
            .map_err(|e| format!("名字表读不了（{}）：{}", Doc::Names.label(), e))?;
        let names: NamesFile = yaml_serde::from_str(&text)
            .map_err(|e| format!("名字表非法（{}）：{}", Doc::Names.label(), e))?;
        if names.agents.is_empty() {
            return Err(format!(
                "名字表里 agent 保留名一块是空的（{} 的 agents）",
                Doc::Names.label()
            ));
        }
        if names.modules.is_empty() {
            return Err(format!(
                "名字表里模块保留名一块是空的（{} 的 modules）",
                Doc::Names.label()
            ));
        }
        Ok(SystemTools {
            tools: tools.tools,
            reserved_agents: names.agents,
            reserved_modules: names.modules,
        })
    }
}
