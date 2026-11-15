//! 目的：动作分发——参数校验 → 按 callers 授权 → 执行 → 审计，各只有一处。
//! 管：把 `systools/tools.yaml` 的动作声明落到具体用例；人经呈现层与模型经工具调用共用这一份。
//! 不管：生成怎么跑（在单 / 协作生成里）；工具进程怎么起（在 tools 能力里）；只读视图不进动作表。
//! 联动：声明与 callers 在 `systools/tools.yaml`；角色面在 `systools/roles.yaml`；呈现层只造 `ActionCall`（见 docs/tools/tools-and-roles.md）。

use super::*;
use crate::capabilities::tools::api::{ToolSchema, arg_fault_text};

mod module;
mod registry;
mod session;

impl ConductorHandle {
    /// 目的：授权——动作表 `callers` 里有这个调用者身份才放行（授权只此一处，判据在 permission 能力）。
    fn authorize_action(
        &self,
        id: &str,
        caller: &Caller,
        schema: &ToolSchema,
    ) -> Result<(), String> {
        crate::capabilities::permission::api::authorize_action(&schema.callers, caller.token(), id)
    }

    /// 目的：一次动作的审计记录（同一处产出）：谁、调了什么、成没成。模型侧另有工具行。
    fn audit_action(&self, caller: &Caller, id: &str, ok: bool, detail: &str) {
        let who = match caller {
            Caller::User => "user".to_string(),
            Caller::Role { role, work, agent } => format!("{}（{}/{}）", role, work, agent),
        };
        self.log.info(
            "action",
            &crate::capabilities::tools::api::action_audit(&who, id, ok, detail),
        );
    }

    /// 目的：一次动作的分派——按动作 id 的归属交给三处执行面，自己不做业务。
    /// 约束：模块工具（`module.` 前缀）先于静态表；登记处动词是产品级资源，只给人用；其余是会话 / 工作动词。
    pub(super) fn execute_action(
        &self,
        id: &str,
        caller: &Caller,
        args: &serde_json::Value,
        out: Output,
    ) -> Result<Acted, String> {
        if id.starts_with("module.") {
            return self.execute_module_action(caller, id, args);
        }
        match id {
            "upsert_provider" | "remove_provider" | "discover_models" | "upsert_model"
            | "remove_model" | "set_core_model" | "probe_model_tools" | "probe_replay_shape"
            | "upsert_agent" | "remove_agent" | "set_settings" => self.execute_registry(id, args),
            other => self.execute_session_action(other, caller, args, out),
        }
    }
}

impl ActionOps for ConductorHandle {
    /// 目录：只列这个调用者能调的动作，并给出**此刻可不可用**（人经呈现层按它渲染，不写第二份清单）。
    fn catalog(&self, caller: &Caller, _sid: Option<&str>) -> Result<Vec<ActionView>, String> {
        let token = caller.token();
        let mut out = Vec::new();
        for (id, schema) in &self.book {
            if !schema.callers.iter().any(|c| c == token) {
                continue;
            }
            out.push(ActionView {
                id: id.clone(),
                desc: schema.desc.clone(),
                params: schema
                    .params
                    .iter()
                    .flatten()
                    .map(|(name, p)| ActionParamView {
                        name: name.clone(),
                        ty: p.ty.name().to_string(),
                        required: p.required,
                        desc: p.desc.clone(),
                    })
                    .collect(),
                available: true,
                reason: String::new(),
            });
        }
        // 模块工具动作（动态，来自清单）：模块声明的每个工具都是一条动作，人可直接跑。
        // 会话里的模块工具由成员循环执行——那是同一个声明、同一个执行面的另一个适配器。
        if matches!(caller, Caller::User)
            && let Ok(roster) = self.call(|core| Ok(core.scan()))
        {
            let missing = self.unavailable_modules();
            for m in &roster.modules {
                for (tool, decl) in &m.manifest.tools {
                    let mut params: Vec<ActionParamView> = match decl.schema() {
                        Some(schema) => schema
                            .params
                            .iter()
                            .flatten()
                            .map(|(name, p)| ActionParamView {
                                name: name.clone(),
                                ty: p.ty.name().to_string(),
                                required: p.required,
                                desc: p.desc.clone(),
                            })
                            .collect(),
                        None => Vec::new(),
                    };
                    params.push(ActionParamView {
                        name: "workspace".to_string(),
                        ty: "string".to_string(),
                        required: false,
                        desc: "工作目录（绝对路径）；省略 = 模块自己的 userdata/".to_string(),
                    });
                    let (available, reason) = match missing.get(&m.manifest.id) {
                        Some(caps) => (false, format!("缺运行能力：{}", caps.join("、"))),
                        None => (true, String::new()),
                    };
                    out.push(ActionView {
                        id: format!("module.{}.{}", m.manifest.id, tool),
                        desc: decl.desc.clone(),
                        params,
                        available,
                        reason,
                    });
                }
            }
            // 已就绪常驻服务的操作：也是一条动态模块动作（id = module.<模块>.<服务>.<操作>）。
            // 未启动的服务不出现（操作是跑起来才发现的）；呈现层因此不必另建一份清单。
            for view in self.residents.services().unwrap_or_default() {
                if view.state != crate::capabilities::residents::api::ServiceState::Ready {
                    continue;
                }
                for op in &view.operations {
                    out.push(ActionView {
                        id: format!("module.{}.{}.{}", view.module, view.name, op.name),
                        desc: op.description.clone(),
                        params: Vec::new(),
                        available: true,
                        reason: String::new(),
                    });
                }
            }
        }
        Ok(out)
    }

    /// 一次动作：参数按声明校验 → 按 `callers` 授权 → 执行 → 审计。CLI 与 Web 共用这一份。
    fn act(&self, call: ActionCall) -> Result<Acted, String> {
        let id = call.id.clone();
        let caller = call.caller.clone();
        // 模块工具动作是**动态动作**（来自清单，不在静态表里）：授权与参数校验都在它自己的实现里。
        let r = if id.starts_with("module.") {
            self.execute_action(&id, &caller, &call.args, call.out)
        } else {
            match self.book.get(&id) {
                None => Err(format!("未知动作：{}", id)),
                Some(schema) => {
                    let mut args = call.args;
                    // `compact` 在册子里是**压缩回合的模型工具**（必填 summary）；用户动作只带信封里的
                    // session_id，不能拿工具参数来校验动作。动作的 callers 仍按同一张表授权。
                    let checked = if id == "compact" {
                        Ok(())
                    } else {
                        schema
                            .check(&args)
                            .map_err(|fault| arg_fault_text(&id, schema, &fault))
                    };
                    match checked {
                        Err(e) => Err(e),
                        Ok(()) => {
                            schema.apply_defaults(&mut args);
                            self.authorize_action(&id, &caller, schema)
                                .and_then(|_| self.execute_action(&id, &caller, &args, call.out))
                        }
                    }
                }
            }
        };
        match &r {
            Ok(_) => self.audit_action(&caller, &id, true, ""),
            Err(e) => self.audit_action(&caller, &id, false, e),
        }
        r
    }
}
