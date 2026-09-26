//! **模块清单 → 工具面**：把 `module.yaml` 声明的工具与参数契约转成工具能力认识的形态。
//!
//! 归属：这些都是**工具侧**的知识（保留名、参数契约、模型侧说明），
//! 留在 `workspace` 会让 `workspace → tools` 成环（见 docs/architecture/refactor-plan.md §3.8）。
//! 参数**声明形态**（`Param` / `ParamType`）仍归 `workspace`（它是 `module.yaml` 的字段）。

use crate::capabilities::tools::domain::schema::ToolSchema;
use crate::capabilities::workspace::api::{Module, ModuleManifest, ToolDecl};

impl ToolDecl {
    /// 参数契约的声明形态（校验与渲染共用）；没声明参数 = None = 不校验。
    pub fn schema(&self) -> Option<ToolSchema> {
        self.params
            .as_ref()
            .map(|p| crate::capabilities::tools::api::ToolSchema {
                desc: self.desc.clone(),
                params: Some(p.clone()),
                parallel: self.parallel,
                // 模块工具的能力由它的运行方式决定（外部命令），不在这一层声明。
                capability: String::new(),
            })
    }
}

/// 外部工具表的校验（纯逻辑；扫描模块时由适配层调用）：内置工具名是保留名，占用 = 拒收并说明原因。
pub fn check_tools(m: &ModuleManifest) -> Result<(), String> {
    for (name, decl) in &m.tools {
        if crate::capabilities::tools::api::is_builtin(name) {
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

/// 模块工具的参数契约（只列**声明了**参数的）：模型据此写信封里的 args；没声明的照旧不校验。
pub fn module_tool_params(
    prompts: &crate::capabilities::prompt::api::Prompts,
    modules: &[Module],
) -> String {
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
pub fn module_tools(
    prompts: &crate::capabilities::prompt::api::Prompts,
    modules: &[Module],
) -> String {
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
