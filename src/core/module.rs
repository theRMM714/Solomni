//! 模块打包契约（module.yaml）与扫描结果。
//! 目录遍历机制在 adapters（ModuleSource 端口）；「清单即事实」的重扫策略由 core 执行。
//! 模型选择是会话级决定（记录在会话里），模块清单不再承载模型/供应商偏好。

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// module.yaml —— 打包契约。模块对世界的全部自我介绍。
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleManifest {
    pub id: String,
    pub brief: String,
    pub system: String,
    /// 运行能力声明：本模块的工具需要哪些运行包能力（如 python / node / bash / c-c++）。
    /// 只声明能力名，不写版本——版本由用户在会话的执行档位里定（见 core/exec.rs 与 RUNTIME_SPEC.md）。
    #[serde(default)]
    pub runtimes: Vec<String>,
    /// 外部工具表：工具名 → 该工具的声明（启动命令 + 可选的参数契约）。
    /// 内置工具名（read / write / search）为保留名，模块不得占用（见 check_tools）。
    #[serde(default)]
    pub tools: BTreeMap<String, ToolDecl>,
}

/// 一个模块工具：模块作者声明「怎么启动它」以及「它吃什么参数」。
/// 参数契约**可选**：不写就照旧不校验、也不写进提示词；写了就由核心按它校验，并把说明写进系统提示。
#[derive(Debug, Clone, Deserialize)]
pub struct ToolDecl {
    /// 启动命令（相对模块根写，例如 python tools/x.py）；工作目录 = 该模块的根目录。
    pub command: String,
    /// 给模型看的一句话说明（可选）。
    #[serde(default)]
    pub desc: String,
    /// 参数契约（可选）：参数名 → 声明。
    #[serde(default)]
    pub params: Option<BTreeMap<String, crate::core::schema::Param>>,
    /// 这个工具**可并发执行**（缺省 false = 独占串行）：只读、无副作用的工具才该声明 true，
    /// 同一回复里的多个可并发调用会真的并发跑（结果仍按调用顺序回填）。
    #[serde(default)]
    pub parallel: bool,
}

impl ToolDecl {
    /// 参数契约的声明形态（校验与渲染共用）；没声明参数 = None = 不校验。
    pub fn schema(&self) -> Option<crate::core::schema::ToolSchema> {
        self.params
            .as_ref()
            .map(|p| crate::core::schema::ToolSchema {
                desc: self.desc.clone(),
                params: Some(p.clone()),
                parallel: self.parallel,
                // 模块工具的能力由它的运行方式决定（外部命令），不在这一层声明。
                capability: String::new(),
            })
    }
}

/// 运行能力声明的校验（纯逻辑；扫描模块时由适配层调用）：非法或重复 = 拒收并说明原因，不纠正。
pub fn check_runtimes(m: &ModuleManifest) -> Result<(), String> {
    let mut seen: Vec<&String> = Vec::new();
    for c in &m.runtimes {
        if !crate::core::packages::valid_capability(c) {
            return Err(format!(
                "runtimes 里的能力名不合法：{}（只允许小写字母、数字、- _ .，且以字母或数字开头）",
                c
            ));
        }
        if seen.contains(&c) {
            return Err(format!("runtimes 里重复声明了：{}", c));
        }
        seen.push(c);
    }
    Ok(())
}

/// 外部工具表的校验（纯逻辑；扫描模块时由适配层调用）：内置工具名是保留名，占用 = 拒收并说明原因。
pub fn check_tools(m: &ModuleManifest) -> Result<(), String> {
    for (name, decl) in &m.tools {
        if crate::core::systool::is_builtin(name) {
            return Err(format!(
                "tools 里的 {} 是核心内置工具名（保留名），模块不得占用",
                name
            ));
        }
        if decl.command.trim().is_empty() {
            return Err(format!("tools 里的 {} 没写 command（启动命令）", name));
        }
    }
    Ok(())
}

/// 一个已发现的模块 = 文件夹 + 清单。
#[derive(Debug, Clone)]
pub struct Module {
    pub manifest: ModuleManifest,
    pub root: PathBuf,
}

/// 一个 agent 的职责提示词：把它的模块 system 合成一份能力包，再挂内置工具说明与外部工具清单。
/// 模块只是能力包（没有"发言"这回事）；发言席是 agent，所以这份 system 按 agent 成文。
/// sys_tools 由 core::systool 按该 agent 的沙箱渲染后传入。
pub fn agent_system(
    prompts: &crate::core::prompt::Prompts,
    agent: &str,
    modules: &[Module],
    sys_tools: &str,
    mode: crate::core::providers::ToolMode,
) -> String {
    let parts = modules
        .iter()
        .map(|m| format!("\n== {} ==\n{}", m.manifest.id, m.manifest.system.trim()))
        .collect::<Vec<_>>()
        .join("");
    prompts.render(
        &prompts.core.agent.system,
        &[
            ("agent", agent.to_string()),
            ("modules", parts),
            ("sys_tools", sys_tools.to_string()),
            ("module_tools", module_tools(prompts, modules)),
            ("module_tool_params", module_tool_params(prompts, modules)),
            // 两套调用约定**互斥**：一个通道只用一套（同时教会让模型在正文里讲解参数而被误判成调用）
            (
                "tool_calling",
                match mode {
                    crate::core::providers::ToolMode::Native => {
                        prompts.core.tool_calling_native.clone()
                    }
                    crate::core::providers::ToolMode::Envelope => {
                        prompts.core.tool_calling_envelope.clone()
                    }
                },
            ),
        ],
    )
}

/// 模块工具的参数契约（只列**声明了**参数的）：模型据此写信封里的 args；没声明的照旧不校验。
pub fn module_tool_params(prompts: &crate::core::prompt::Prompts, modules: &[Module]) -> String {
    let texts = &prompts.core.tool_texts;
    let mut sections: Vec<String> = Vec::new();
    for m in modules {
        for (name, decl) in &m.manifest.tools {
            if let Some(schema) = decl.schema() {
                sections.push(texts.render(
                    &texts.module_tool_params_line,
                    &[
                        ("module", m.manifest.id.clone()),
                        ("tool", name.clone()),
                        ("signature", schema.render_for_prompt()),
                    ],
                ));
            }
        }
    }
    if sections.is_empty() {
        return prompts.core.no_module_tool_params.clone();
    }
    format!(
        "{}\n{}",
        prompts.core.module_tool_params_header,
        sections.join("\n")
    )
}

/// 该 agent 的外部工具清单：**按模块分组，每行一个模块**（模块 id：工具名、…）。
/// 模型据此在信封里写 module；都没有声明工具时用册子里的说法（用法不变）。
pub fn module_tools(prompts: &crate::core::prompt::Prompts, modules: &[Module]) -> String {
    let texts = &prompts.core.tool_texts;
    let lines: Vec<String> = modules
        .iter()
        .filter(|m| !m.manifest.tools.is_empty())
        .map(|m| {
            let names = m
                .manifest
                .tools
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(&texts.tool_list_separator);
            texts.render(
                &texts.module_tools_line,
                &[("id", m.manifest.id.clone()), ("tools", names)],
            )
        })
        .collect();
    if lines.is_empty() {
        prompts.core.no_module_tools.clone()
    } else {
        lines.join("\n")
    }
}

/// 扫描结果：合法模块 + 拒收原因（校验，不是挑选——如实呈现）。
pub struct Roster {
    pub modules: Vec<Module>,
    pub rejected: Vec<String>,
}

/// 模块公地清单（拟名单时给模型看）：id / 简述。
pub fn listing(roster: &Roster, texts: &crate::core::prompt::ToolTexts) -> String {
    roster
        .modules
        .iter()
        .map(|m| {
            texts.render(
                &texts.module_listing_line,
                &[
                    ("id", m.manifest.id.clone()),
                    ("brief", m.manifest.brief.clone()),
                ],
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
