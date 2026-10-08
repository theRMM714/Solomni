//! 目的：模块清单 → 工具面——把 `module.yaml` 声明的工具与参数契约转成工具能力认识的形态。
//! 管：参数契约的声明形态（`ToolDecl::schema`）、模型侧的工具参数段与工具清单段。
//! 不管：`module.yaml` 的读取与参数**声明形态**（`Param` / `ParamType` 归 `workspace`）——这里只消费，免得 `workspace → tools` 成环。
//! 联动：由本能力的 `service/` 在装配工具面时调用；声明形态来自 `workspace::api`。

use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::tools::domain::schema::ToolSchema;
use crate::capabilities::workspace::api::{Module, ToolDecl};

impl ToolDecl {
    /// 目的：把这个工具声明的参数契约转成工具能力认识的形态（校验与渲染共用）。
    /// 返回：声明了参数时给出 schema；没声明 = `None` = 不校验。
    pub fn schema(&self) -> Option<ToolSchema> {
        self.params
            .as_ref()
            .map(|p| crate::capabilities::tools::api::ToolSchema {
                desc: self.desc.clone(),
                params: Some(p.clone()),
                parallel: self.parallel,
                // 模块工具的能力由它的运行方式决定（外部命令），不在这一层声明。
                capability: String::new(),
                // 模块工具的调用者由**成员归属**决定（装了它的角色 + 用户直跑），不在这一层声明。
                callers: Vec::new(),
            })
    }
}

/// 目的：渲染模块工具的参数契约段（只列**声明了**参数的）。
/// 参数：`prompt` 提供文案，`modules` 是这一席的模块清单。
/// 返回：给模型看的参数段；一个都没声明时用册子里「没有」的说法。
pub fn module_tool_params(prompt: &dyn Prompt, modules: &[Module]) -> String {
    let texts = prompt.tools();
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
        return prompt.text(Segment::NoModuleToolParams).to_string();
    }
    format!(
        "{}\n{}",
        prompt.text(Segment::ModuleToolParamsHeader),
        sections.join("\n")
    )
}

/// 目的：渲染该 agent 的外部工具清单（**按模块分组，每行一个模块**）。
/// 参数：`prompt` 提供文案，`modules` 是这一席的模块清单。
/// 返回：模型据此在信封里写 module；都没有声明工具时用册子里的说法。
pub fn module_tools(prompt: &dyn Prompt, modules: &[Module]) -> String {
    let texts = prompt.tools();
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
        prompt.text(Segment::NoModuleTools).to_string()
    } else {
        lines.join("\n")
    }
}
