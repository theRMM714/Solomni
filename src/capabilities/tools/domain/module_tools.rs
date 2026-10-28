//! 目的：模块清单 → 工具面——把 `module.yaml` 声明的工具与参数契约转成工具能力认识的形态。
//! 管：参数契约的声明形态（`ToolDecl::schema`）、模型侧的工具参数段与工具清单段。
//! 不管：`module.yaml` 的读取与参数**声明形态**（`Param` / `ParamType` 归 `workspace`）——这里只消费，免得 `workspace → tools` 成环。
//! 联动：由本能力的 `service/` 在装配工具面时调用；声明形态来自 `workspace::api`。

use crate::capabilities::modules::api::{Module, ToolDecl};
use crate::capabilities::prompt::api::{Prompt, Segment};
use crate::capabilities::tools::domain::schema::ToolSchema;

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
                effect: None,
                reach: None,
                decision: None,
                scope: false,
                // 模块工具的调用者由**成员归属**决定（装了它的角色 + 用户直跑），不在这一层声明。
                callers: Vec::new(),
            })
    }
}

/// 目的：渲染参数契约段（模块工具 + 已就绪服务操作合并成**一段**，标题只出现一次）。
/// 参数：`prompt` 提供段标题与空态，`modules` 是这一席的模块清单，
///   `extra_sections` 是装配方按**已就绪服务操作**渲染好的行（不带标题；由本函数并进同一段）。
/// 返回：给模型看的参数段；两组都没声明参数时用册子里「没有」的说法。
pub fn module_tool_params(
    prompt: &dyn Prompt,
    modules: &[Module],
    extra_sections: &[String],
) -> String {
    let mut sections: Vec<String> = Vec::new();
    for m in modules {
        for (name, decl) in &m.manifest.tools {
            if let Some(schema) = decl.schema() {
                sections.push(format!(
                    "- {}.{}\n{}",
                    m.manifest.id,
                    name,
                    schema.render_for_prompt()
                ));
            }
        }
    }
    sections.extend(extra_sections.iter().cloned());
    if sections.is_empty() {
        return "（本 agent 的模块工具都没声明参数，args 由工具自己解释）".to_string();
    }
    format!(
        "{}\n{}",
        prompt.text(Segment::ModuleToolParamsHeader),
        sections.join("\n")
    )
}

/// 目的：渲染该 agent 的外部工具清单（**按模块分组，每行一个模块**）。
/// 参数：`modules` 是这一席的模块清单。
/// 返回：模型据此在信封里写 module；都没有声明工具时如实说明。
pub fn module_tools(modules: &[Module]) -> String {
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
                .join("、");
            format!("- {}：{}", m.manifest.id, names)
        })
        .collect();
    if lines.is_empty() {
        "（本 agent 的模块都未声明外部工具）".to_string()
    } else {
        lines.join("\n")
    }
}
