//! 目的：模块工具类动作：把 module.模块.工具 翻译成一次带围栏的工具调用。
//! 管：unavailable_modules、authorize_module、module_action、service_action、run_module_action。
//! 不管：工具怎么跑（tools 能力）、模块清单从哪来（modules 能力）。
//! 联动：execute_action 的 module 分支调它；常驻服务操作在 residents 里执行。

use super::*;

impl Handle {
    /// 目的：模块 id → 它在**当前设置档位**下缺的运行包能力（空表 = 都能跑）。
    /// 约束：与成员循环读同一把尺子（`packages::api::plan_host` 的 `unusable`，宿主目标）——本机档按宿主目标选包，
    ///   不因"没装包"拒绝本机解释器就能跑的工具；缺包 = 不执行，不静默降级。
    pub(super) fn unavailable_modules(&self) -> BTreeMap<String, Vec<String>> {
        self.call(|core| Ok(core.unavailable_modules_now()))
            .unwrap_or_default()
    }

    /// 目的：模块工具动作的授权——它是**人直接用**的入口，会话里的模块工具走成员循环。
    pub(super) fn authorize_module(&self, caller: &Caller) -> Result<(), String> {
        match caller {
            Caller::User => Ok(()),
            Caller::Role { .. } => Err(
                "模块工具动作由用户直接调用；会话里的模块工具走成员循环（那条路由角色面授权）"
                    .to_string(),
            ),
        }
    }

    /// 目的：把 `module.<模块id>.<工具名>` 解析成清单里的模块与工具（模块 id 可能含 `.`，按最长前缀匹配）。
    pub(super) fn module_action(
        &self,
        id: &str,
    ) -> Result<
        (
            crate::capabilities::catalog::modules::api::Module,
            String,
            crate::capabilities::catalog::modules::api::ToolDecl,
        ),
        String,
    > {
        let rest = id.strip_prefix("module.").unwrap_or("");
        let roster = self.call(|core| Ok(core.modules_handle().roster()))?;
        for m in &roster.modules {
            let prefix = format!("{}.", m.manifest.id);
            if let Some(tool) = rest.strip_prefix(&prefix)
                && let Some(decl) = m.manifest.tools.get(tool)
            {
                return Ok((m.clone(), tool.to_string(), decl.clone()));
            }
        }
        Err(format!(
            "清单里没有这个模块工具：{}（动作 id 形如 module.<模块id>.<工具名>）",
            id
        ))
    }

    /// 目的：把 `module.<模块id>.<服务名>.<操作名>` 解析成**已声明**服务的操作（模块 id / 服务名都可能含 `.`，按声明逐段匹配）。
    /// 返回：None = 不是常驻服务的操作（交给静态模块工具那条路）。
    pub(super) fn service_action(
        &self,
        id: &str,
    ) -> Result<Option<(String, String, String)>, String> {
        let rest = id.strip_prefix("module.").unwrap_or("");
        let roster = self.call(|core| Ok(core.modules_handle().roster()))?;
        for m in &roster.modules {
            let Some(after_m) = rest.strip_prefix(&format!("{}.", m.manifest.id)) else {
                continue;
            };
            for name in m.manifest.services.keys() {
                if let Some(op) = after_m.strip_prefix(&format!("{}.", name))
                    && !op.is_empty()
                {
                    return Ok(Some((m.manifest.id.clone(), name.clone(), op.to_string())));
                }
            }
        }
        Ok(None)
    }

    /// 目的：**人直接用模块工具**（无会话）：按 `module.yaml` 校验参数，给一份独立围栏，跑一次真命令。
    /// 约束：执行面与 agent 会话共用（`ToolExec::run_module`），只是围栏按模块目录 + 用户指定的工作目录派生。
    pub(super) fn run_module_action(
        &self,
        caller: &Caller,
        id: &str,
        args: &serde_json::Value,
    ) -> Result<Acted, String> {
        self.authorize_module(caller)?;
        let (module, tool, decl) = self.module_action(id)?;
        // 直接动作跑在宿主：按宿主目标取包计划（没有会话定版，多版本取稳定序第一个）。
        let module_for_plan = module.clone();
        let host_plan =
            self.call(move |core| Ok(core.host_plan_now(std::slice::from_ref(&module_for_plan))))?;
        // 与成员循环同一把尺子：该能力在本机档不可用 = 该模块的工具不执行，并如实说明缺哪个能力。
        if let Some(caps) = host_plan.unusable.get(&module.manifest.id) {
            return Err(format!(
                "模块 {} 的运行能力 {} 在本机档不可用，不能直接跑它的工具；把适用于本机目标的包放进依赖文件夹 runtimes/（契约见 RUNTIME_SPEC.md）",
                module.manifest.id,
                caps.join("、")
            ));
        }
        // `workspace` 是机制参数（围栏的落点），不是模块声明的参数：先摘出来再按声明校验其余。
        let mut module_args = args.clone();
        let work = module_args.as_object_mut().and_then(|o| {
            o.remove("workspace")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        });
        if let Some(schema) = decl.schema()
            && let Err(fault) = schema.check(&module_args)
        {
            let full = format!("module.{}.{}", module.manifest.id, tool);
            return Err(arg_fault_text(&full, &schema, &fault));
        }
        let work = work.map(std::path::PathBuf::from);
        let mut fence = crate::kernel::api::fence::FencePlan::standalone(
            &module.root,
            work.as_deref(),
            module.has_userdata,
        );
        fence.net = crate::kernel::api::fence::NetPolicy::from_flag(
            self.settings()?.sandbox.policy.network,
        );
        // 独立调用没有会话：给唯一 lease，跑完按它收尾（Windows 上才有独立的容器身份）。
        fence.lease = crate::kernel::api::fence::call_lease(&fence.agent);
        fence.extend_read_tree(host_plan.roots.iter().cloned());
        let args_json = serde_json::to_string(&module_args).unwrap_or_else(|_| "{}".to_string());
        // 人直接跑模块工具：这一趟**没有可回答的前端**（没有会话、没有裁决队），所以不给提问端口——
        // 围栏的必要落点授不上时按 fail-closed 拒绝；隐私字段只经环境变量注入（值不进命令行）。
        let env = self.secrets.resolve(&module.manifest.id)?;
        let outcome = self.tools.run_module(
            &module.manifest.id,
            &fence,
            &decl.command,
            &args_json,
            &env,
            host_plan.path.as_deref(),
            decl.parallel,
            None,
            crate::capabilities::tools::api::ResultBudget::full(),
        );
        // 按次收尾：撤 ACE、删按次 profile 与临时目录；失败如实记录（残留由 reconcile 按死归属回收）。
        if let Err(e) = self.tools.release_call(&fence) {
            self.log.info(
                "runtime::module_tool",
                &format!("围栏按次收尾未完成：{}", e),
            );
        }
        Ok(Acted::Done(serde_json::json!({
            "ok": outcome.ok,
            "severity": outcome.severity,
            "output": outcome.output,
            "notes": outcome.notes,
        })))
    }
}
