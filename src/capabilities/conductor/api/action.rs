//! 目的：动作分发——参数校验 → 按 callers 授权 → 执行 → 审计，各只有一处。
//! 管：把 `systools/tools.yaml` 的动作声明落到具体用例；人经呈现层与模型经工具调用共用这一份。
//! 不管：生成怎么跑（在单 / 协作生成里）；工具进程怎么起（在 tools 能力里）；只读视图不进动作表。
//! 联动：声明与 callers 在 `systools/tools.yaml`；角色面在 `systools/roles.yaml`；呈现层只造 `ActionCall`（见 docs/tools/tools-and-roles.md）。

use super::*;
use crate::capabilities::conductor::domain::proxy as d;
use crate::capabilities::registry::api::{AppSettings, RegistryOps};
use crate::capabilities::tools::api::{arg_fault_text, ToolSchema};

/// 极简 base64 解码（上传动作的参数是 base64；标准字母表，容忍换行与缺失填充）。
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return Err("base64 含非法字符".to_string()),
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

impl ConductorHandle {
    /// 目的：授权——动作表 `callers` 里有这个调用者身份才放行（授权只此一处）。
    fn authorize_action(
        &self,
        id: &str,
        caller: &Caller,
        schema: &ToolSchema,
    ) -> Result<(), String> {
        let token = caller.token();
        if schema.callers.iter().any(|c| c == token) {
            Ok(())
        } else {
            Err(format!(
                "动作 {} 不接受这个调用者：{}（越出授权范围）",
                id, token
            ))
        }
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

    /// 目的：建会话的**唯一实现**——人经呈现层（没有父）与核心代理（有父）都走这条。
    /// 参数：`caller` 是调用者身份；`spec` 是规整后的载荷（父会话由机制填）。
    /// 返回：建出的会话与名单；开场事实已推上事件台。
    /// 错误：名字 / 档位 / 名单不成立时如实拒绝，不留半成品。
    pub(crate) fn create_session_call(
        &self,
        caller: &Caller,
        spec: &d::NewSession,
    ) -> Result<d::Created, String> {
        let (created, facts) = self.call({
            let spec = spec.clone();
            move |core| core.create_session(&spec)
        })?;
        self.publish(&created.session, facts);
        // 有父 = 核心代理建的子工作：**建好就开工**（single 以 task 为第一句；collab 开始讨论）。
        if spec.parent.is_some() {
            self.record_notice(
                &created.session,
                SessionEvent::Notice(d::relay_note(d::MessageKind::Task)),
            );
            let mode = self.call({
                let c = created.session.clone();
                move |core| core.dispatch_target(&c)
            })?;
            if mode == "collab" {
                self.start_child_collab(&created.session);
            } else {
                self.spawn_detached_node(&created.session, &spec.task);
            }
        }
        let _ = caller;
        Ok(created)
    }

    /// 目的：控制会话的**唯一实现**——stop / continue / close，人经呈现层与核心代理都走这条。
    /// 返回：这一下的实际状态（如 `stopped:2` / `active` / `closed`），不假装"停止了一个没在跑的会话"。
    pub(crate) fn control_session_call(
        &self,
        caller: &Caller,
        sid: &str,
        action: d::ControlAction,
        reason: &str,
    ) -> Result<String, String> {
        let why = if reason.trim().is_empty() {
            "（未填理由）"
        } else {
            reason
        };
        let state = match action {
            d::ControlAction::Stop => {
                // 走会话能力的停止：冻结整棵子树 + 级联中断在跑的生成（回执给实际停下的会话数）。
                let stopped = SessionOps::stop(self, sid);
                format!("stopped:{}", stopped.len())
            }
            d::ControlAction::Continue => match caller {
                // 人经呈现层：接着走的那一轮按常规生成跑完（与历史行为一致）。
                Caller::User => {
                    let _ = SessionOps::continue_flow(self, sid, Output::Final)?;
                    "active".to_string()
                }
                // 核心代理：解冻后**脱离调用方点火**——它不等子会话跑完，靠 observe / messages 回头看。
                Caller::Role { .. } => {
                    let resumed = self.call({
                        let s = sid.to_string();
                        move |core| core.resume_subtree(&s)
                    })?;
                    if resumed {
                        let mode = self.call({
                            let s = sid.to_string();
                            move |core| Ok(core.session_mode_str(&s))
                        })?;
                        match mode.as_str() {
                            "collab" => self.spawn_detached_collab(sid),
                            "proxy" => self.spawn_detached_proxy(sid),
                            _ => self.spawn_detached_continue(sid),
                        }
                    }
                    "active".to_string()
                }
            },
            d::ControlAction::Close => {
                let st = self.call({
                    let s = sid.to_string();
                    move |core| core.proxy_control(&s, action)
                })?;
                st.state
            }
        };
        self.record_notice(sid, SessionEvent::Notice(d::control_note(action, why)));
        Ok(state)
    }

    /// 目的：把呈现层 / 模型给的参数规整成建会话载荷（ref 形式在这里查登记处）。
    fn build_new_session(&self, args: &serde_json::Value) -> Result<d::NewSession, String> {
        let mode = d::SessionMode::parse(args.get("mode").and_then(|v| v.as_str()).unwrap_or(""))?;
        let mut agents: Vec<d::NewAgent> = Vec::new();
        if let Some(arr) = args.get("agents").and_then(|v| v.as_array()) {
            for (i, a) in arr.iter().enumerate() {
                let obj = a
                    .as_object()
                    .ok_or_else(|| format!("agents[{}] 必须是对象", i))?;
                if let Some(r) = obj.get("ref").and_then(|v| v.as_str()) {
                    let want = r.to_string();
                    let lookup = want.clone();
                    let views = self.call(move |core| core.registry().pick_agents(&[lookup]))?;
                    let v = views
                        .into_iter()
                        .next()
                        .ok_or_else(|| format!("agents[{}]：agent {} 不在登记处", i, want))?;
                    agents.push(d::NewAgent {
                        name: v.name,
                        transient: false,
                        modules: v.modules,
                        model: v.model,
                    });
                    continue;
                }
                let name = obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() {
                    return Err(format!("agents[{}] 缺少 name", i));
                }
                let modules = obj
                    .get("modules")
                    .and_then(|v| v.as_array())
                    .map(|m| {
                        m.iter()
                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let model = obj
                    .get("model")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_string());
                let transient = obj
                    .get("transient")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                agents.push(d::NewAgent {
                    name,
                    transient,
                    modules,
                    model,
                });
            }
        }
        Ok(d::NewSession {
            mode,
            agents,
            task: args
                .get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            request_id: args
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            name: args
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            tier: args
                .get("tier")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            parent: None,
        })
    }

    /// 目的：模块 id → 它在当前档位下缺的运行包能力（空表 = 都能跑）。
    /// 约束：与成员循环读同一把尺子（`runtime_report.missing`）——缺包 = 不执行，不静默降级。
    fn missing_runtimes(&self) -> BTreeMap<String, Vec<String>> {
        let tier = self
            .call(|core| Ok(core.registry().app_settings().tier))
            .unwrap_or_default();
        ConductorOps::runtime_report(self, tier)
            .map(|r| r.missing)
            .unwrap_or_default()
    }

    /// 目的：模块工具动作的授权——它是**人直接用**的入口，会话里的模块工具走成员循环。
    fn authorize_module(&self, caller: &Caller) -> Result<(), String> {
        match caller {
            Caller::User => Ok(()),
            Caller::Role { .. } => Err(
                "模块工具动作由用户直接调用；会话里的模块工具走成员循环（那条路由角色面授权）"
                    .to_string(),
            ),
        }
    }

    /// 目的：把 `module.<模块id>.<工具名>` 解析成清单里的模块与工具（模块 id 可能含 `.`，按最长前缀匹配）。
    fn module_action(
        &self,
        id: &str,
    ) -> Result<
        (
            crate::capabilities::workspace::api::Module,
            String,
            crate::capabilities::workspace::api::ToolDecl,
        ),
        String,
    > {
        let rest = id.strip_prefix("module.").unwrap_or("");
        let roster = self.call(|core| Ok(core.scan()))?;
        for m in &roster.modules {
            let prefix = format!("{}.", m.manifest.id);
            if let Some(tool) = rest.strip_prefix(&prefix) {
                if let Some(decl) = m.manifest.tools.get(tool) {
                    return Ok((m.clone(), tool.to_string(), decl.clone()));
                }
            }
        }
        Err(format!(
            "清单里没有这个模块工具：{}（动作 id 形如 module.<模块id>.<工具名>）",
            id
        ))
    }

    /// 目的：**人直接用模块工具**（无会话）：按 `module.yaml` 校验参数，给一份独立围栏，跑一次真命令。
    /// 约束：执行面与 agent 会话共用（`ToolExec::run_module`），只是围栏按模块目录 + 用户指定的工作目录派生。
    fn run_module_action(
        &self,
        caller: &Caller,
        id: &str,
        args: &serde_json::Value,
    ) -> Result<Acted, String> {
        self.authorize_module(caller)?;
        let (module, tool, decl) = self.module_action(id)?;
        // 与成员循环同一把尺子：该能力不在包库里 = 该模块的工具不执行，并如实说明缺哪个能力。
        if let Some(caps) = self.missing_runtimes().get(&module.manifest.id) {
            return Err(format!(
                "模块 {} 的运行包 {} 未装载，不能直接跑它的工具；把包放进依赖文件夹 runtimes/（契约见 RUNTIME_SPEC.md）",
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
        if let Some(schema) = decl.schema() {
            if let Err(fault) = schema.check(&module_args) {
                let full = format!("module.{}.{}", module.manifest.id, tool);
                return Err(arg_fault_text(&self.texts, &full, &schema, &fault));
            }
        }
        let work = work.map(std::path::PathBuf::from);
        let fence = crate::kernel::api::FenceSpec::standalone(
            &module.root,
            work.as_deref(),
            module.has_userdata,
        );
        let args_json = serde_json::to_string(&module_args).unwrap_or_else(|_| "{}".to_string());
        // 人直接跑模块工具：这一趟**没有可回答的前端**（没有会话、没有裁决队），所以不给提问端口——
        // 围栏的必要落点授不上时按 fail-closed 拒绝这次调用（回执写清哪一环、怎么补）。
        let outcome = self
            .tools
            .run_module(&fence, &decl.command, &args_json, None);
        Ok(Acted::Done(
            serde_json::json!({ "ok": outcome.ok, "output": outcome.output }),
        ))
    }

    /// 目的：一次动作的执行体（授权与参数校验已在 `act` 里做完）。
    fn execute_action(
        &self,
        id: &str,
        caller: &Caller,
        args: &serde_json::Value,
        out: Output,
    ) -> Result<Acted, String> {
        // 模块工具动作（动态，来自清单）先于静态表分派。
        if id.starts_with("module.") {
            return self.run_module_action(caller, id, args);
        }
        let s = |k: &str| {
            args.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        match id {
            "create_session" => {
                let spec = self.build_new_session(args)?;
                let created = self.create_session_call(caller, &spec)?;
                Ok(Acted::Done(
                    serde_json::json!({ "sid": created.session, "agents": created.agents }),
                ))
            }
            "send_message" => {
                SessionOps::say(self, &s("session_id"), &s("text"), out).map(Acted::Advanced)
            }
            "control_session" => {
                let action = d::ControlAction::parse(&s("action"))?;
                let reason = s("reason");
                let state = self.control_session_call(caller, &s("session_id"), action, &reason)?;
                Ok(Acted::Done(
                    serde_json::json!({ "ok": true, "state": state }),
                ))
            }
            "set_task" => self
                .set_task(&s("session_id"), &s("text"))
                .map(Acted::Advanced),
            // **唯一的回答口**：带卡片 id + 选项 id（+ 附言），校验属于当时那张卡的选项集。
            "answer_card" => self
                .answer_card(&s("session_id"), &s("card"), &s("option"), &s("note"))
                .map(Acted::Advanced),
            "withdraw" => self
                .withdraw_agree(&s("session_id"), &s("agent"))
                .map(Acted::Advanced),
            "rewind" => {
                let n = args.get("id").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
                let target = match s("mode").as_str() {
                    "delete" => RewindTarget::Delete(n),
                    "restore" => RewindTarget::Restore(n),
                    _ => RewindTarget::Archive(n),
                };
                self.rewind(&s("session_id"), target).map(Acted::Replayed)
            }
            "update_task" => self
                .update_task(&s("session_id"), &s("text"))
                .map(Acted::Replayed),
            "compact" => SessionOps::compact(self, &s("session_id")).map(Acted::Advanced),
            "edit_session" => {
                let edit: SessionEdit = serde_json::from_value(args.clone())
                    .map_err(|e| format!("编辑内容非法：{}", e))?;
                self.edit(&s("session_id"), edit)?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "upload" => {
                let bytes = base64_decode(&s("data_base64"))?;
                let overwrite = args
                    .get("overwrite")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let uploaded = self.upload(&s("session_id"), &s("name"), &bytes, overwrite)?;
                Ok(Acted::Done(
                    serde_json::json!({ "ok": true, "uploaded": uploaded }),
                ))
            }
            // 文件域 / 协作动词 / 核心操作这些动作由成员工具循环执行，不经分发器。
            // 登记处动作（供应商 / 密钥 / 模型 / agent / 设置）：产品级资源，只给人用。
            "upsert_provider" | "remove_provider" | "discover_models" | "upsert_model"
            | "remove_model" | "set_core_model" | "probe_model_tools" | "probe_replay_shape"
            | "upsert_agent" | "remove_agent" | "set_settings" => self.execute_registry(id, args),
            other => Err(format!("这个动作不由分发器执行：{}", other)),
        }
    }

    /// 目的：登记处动作的执行体——供应商 / 密钥 / 模型 / agent / 设置，**只给人用**（`callers: [user]`）。
    /// 约束：`api_key` 只进登记处，不进动作目录、不进审计；`set_settings` 是部分更新（未提交字段保留现值）。
    fn execute_registry(&self, id: &str, args: &serde_json::Value) -> Result<Acted, String> {
        let s = |k: &str| {
            args.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        match id {
            "upsert_provider" => {
                self.upsert_provider(&s("id"), &s("base_url"), &s("api_key"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "remove_provider" => {
                let ok = self.remove_provider(&s("id"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": ok })))
            }
            "discover_models" => {
                let models = self.discover_models(&s("id"))?;
                Ok(Acted::Done(
                    serde_json::json!({ "ok": true, "models": models }),
                ))
            }
            "upsert_model" => {
                let context = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0);
                self.upsert_model(
                    &s("id"),
                    &s("name"),
                    &s("api_model"),
                    &s("provider"),
                    &s("note"),
                    context,
                )?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "remove_model" => {
                let ok = self.remove_model(&s("id"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": ok })))
            }
            "set_core_model" => {
                let ok = self.set_core_model(&s("id"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": ok })))
            }
            "probe_model_tools" => {
                let outcome = self.probe_model_tools(&s("id"))?;
                Ok(Acted::Done(self.probe_json(&s("id"), &outcome)))
            }
            "probe_replay_shape" => {
                let report = self.probe_replay_shape(&s("id"))?;
                Ok(Acted::Done(
                    serde_json::json!({ "ok": true, "shapes": report.shapes }),
                ))
            }
            "upsert_agent" => {
                let modules: Vec<String> = args
                    .get("modules")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                self.upsert_agent(&s("name"), &modules, &s("model"), &s("note"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "remove_agent" => {
                let ok = self.remove_agent(&s("name"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": ok })))
            }
            "set_settings" => {
                self.apply_settings(args)?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            other => Err(format!("未知的登记处动作：{}", other)),
        }
    }

    /// 目的：设置的部分更新——未提交的字段保留现值（执行档位 / 围栏写权限 / 会话权限默认值不在这一层暴露）。
    fn apply_settings(&self, args: &serde_json::Value) -> Result<(), String> {
        let current = self.settings()?;
        let num = |k: &str| args.get(k).and_then(|v| v.as_u64());
        let settings = AppSettings {
            streaming: args
                .get("streaming")
                .and_then(|v| v.as_bool())
                .unwrap_or(current.streaming),
            show_reasoning: args
                .get("show_reasoning")
                .and_then(|v| v.as_bool())
                .unwrap_or(current.show_reasoning),
            tier: current.tier,
            fence_write: current.fence_write,
            fence_read: current.fence_read.clone(),
            permissions: current.permissions.clone(),
            qemu_path: current.qemu_path.clone(),
            llm_timeout_secs: num("llm_timeout_secs").unwrap_or(current.llm_timeout_secs),
            compact_at_percent: num("compact_at_percent")
                .map(|v| v as u8)
                .unwrap_or(current.compact_at_percent),
            discuss_remind_cap: num("discuss_remind_cap")
                .map(|v| v as u32)
                .unwrap_or(current.discuss_remind_cap),
        };
        self.set_settings(settings)
    }

    /// 目的：探测结论 → 响应 JSON；`mode` 取探测后登记处里的实际形态（结论只翻译、不解释）。
    fn probe_json(
        &self,
        id: &str,
        outcome: &crate::capabilities::llm::api::ProbeOutcome,
    ) -> serde_json::Value {
        let mode = RegistryOps::models(self)
            .ok()
            .and_then(|ms| ms.into_iter().find(|m| m.id == id))
            .map(|m| m.tools);
        crate::capabilities::conductor::domain::action::probe_view(outcome, mode)
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
        if matches!(caller, Caller::User) {
            if let Ok(roster) = self.call(|core| Ok(core.scan())) {
                let missing = self.missing_runtimes();
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
                    if let Err(fault) = schema.check(&args) {
                        Err(arg_fault_text(&self.texts, &id, schema, &fault))
                    } else {
                        schema.apply_defaults(&mut args);
                        self.authorize_action(&id, &caller, schema)
                            .and_then(|_| self.execute_action(&id, &caller, &args, call.out))
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
