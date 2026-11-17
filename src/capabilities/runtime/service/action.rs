//! 目的：动作分发：一张动作表 → 参数校验 → 授权 → 执行 → 审计，全部只在这里发生一次。
//! 管：base64 解码、授权与审计、execute_action 的总分发，以及建会话用例的准备与提交。
//! 不管：登记处动作的字段怎么落（registry 能力）、模块工具怎么跑（tools）、建会话的用例本体（work）。
//! 联动：会话类动作转 session 子模块，模块工具类转 module 子模块，登记处类转 registry 子模块。

use super::*;
use crate::capabilities::catalog::registry::api::AgentMeta;

// 三个子模块各管一类动作：建会话 / 模块工具 / 登记处。
mod module;
mod registry;
mod session;

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

impl Handle {
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

    /// 目的：一次动作的执行体（授权与参数校验已在 `act` 里做完）。
    pub(super) fn execute_action(
        &self,
        id: &str,
        caller: &Caller,
        args: &serde_json::Value,
        out: Output,
    ) -> Result<Acted, String> {
        // 模块工具动作（动态，来自清单）先于静态表分派。
        if id.starts_with("module.") {
            // 静态模块工具优先（保持既有语义）；不是静态工具时，看它是不是已启动常驻服务的操作。
            if self.module_action(id).is_err()
                && let Some((module, service, op)) = self.service_action(id)?
            {
                self.authorize_module(caller)?;
                let receipt = self.residents.call(&module, &service, &op, args)?;
                return Ok(Acted::Done(
                    serde_json::json!({ "ok": receipt.ok, "output": receipt.output }),
                ));
            }
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
                // 整段输入恰好是 `/compact`：命令走动作，不把命令本身发给模型。
                // 服务端收口，Web 旧缓存 / CLI 直接输入也能触发。
                if s("text").trim() == "/compact" {
                    return TurnOps::compact(self, &s("session_id")).map(Acted::Advanced);
                }
                TurnOps::say(self, &s("session_id"), &s("text"), out).map(Acted::Advanced)
            }
            "set_module_enabled" => {
                let id = s("module");
                let on = args
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.modules.set_module_enabled(&id, on)?;
                // 模块下线 → 它的常驻服务全部停掉（服务不能再跑在一个已下线的模块上）。
                if !on {
                    let roster = self.modules.roster();
                    if let Some(m) = roster.modules.iter().find(|m| m.manifest.id == id) {
                        for name in m.manifest.services.keys() {
                            let _ = self.residents.stop(&id, name);
                        }
                    }
                }
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "control_resident" => {
                let module = s("module");
                let name = s("name");
                let lease = s("lease");
                match s("action").as_str() {
                    "start" => {
                        let list = self.residents.start(&module, &name, &lease)?;
                        let names: Vec<&str> = list.iter().map(|o| o.name.as_str()).collect();
                        Ok(Acted::Done(
                            serde_json::json!({ "ok": true, "operations": names }),
                        ))
                    }
                    "stop" => {
                        self.residents.stop(&module, &name)?;
                        Ok(Acted::Done(serde_json::json!({ "ok": true })))
                    }
                    "enable" => {
                        self.modules.set_service_enabled(&module, &name, true)?;
                        let list = self.residents.start(&module, &name, &lease)?;
                        let names: Vec<&str> = list.iter().map(|o| o.name.as_str()).collect();
                        Ok(Acted::Done(
                            serde_json::json!({ "ok": true, "operations": names }),
                        ))
                    }
                    "disable" => {
                        self.residents.stop(&module, &name)?;
                        self.modules.set_service_enabled(&module, &name, false)?;
                        Ok(Acted::Done(serde_json::json!({ "ok": true })))
                    }
                    other => Err(format!(
                        "control_resident 不支持的动作：{}（可用：start / stop / enable / disable）",
                        other
                    )),
                }
            }
            "activate_session" => {
                let view = RuntimeOps::activate(self, &s("session_id"))?;
                Ok(Acted::Done(serde_json::json!({
                    "ok": true,
                    "session": view,
                })))
            }
            // 只读诊断：准备（定位租约）在核心线程，采样（guest 里两轮 /proc）在调用方线程。
            "guest_diagnose" => {
                let sid = s("session_id");
                let job = self.call(move |core| core.prepare_guest_diagnose(&sid))?;
                let report = job.run()?;
                let value = serde_json::to_value(&report)
                    .map_err(|e| format!("诊断结果序列化失败：{}", e))?;
                Ok(Acted::Done(value))
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
                let sid = s("session_id");
                let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
                // 回档成功后：主/子会话各推一条 rewound（重置快照）并推进事件台水印。
                // 主视图仍随动作回包给调用方；推送让刷新与其它已打开标签页看到同一份重置。
                let outcome = self.rewind_with_reset(&sid, target, force)?;
                Ok(Acted::Replayed(outcome.main))
            }
            "update_task" => self
                .update_task(&s("session_id"), &s("text"))
                .map(Acted::Replayed),
            "compact" => TurnOps::compact(self, &s("session_id")).map(Acted::Advanced),
            "set_permission_profile" => {
                let sid = s("session_id");
                let profile =
                    crate::capabilities::permission::api::ProfileId::parse(&s("profile"))?;
                self.call(move |core| core.set_permission_profile(&sid, profile))?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "set_thinking" => {
                let sid = s("session_id");
                let level = s("level");
                self.call(move |core| core.set_thinking(&sid, &level))?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "set_context_freshness" => {
                let sid = s("session_id");
                let on = args.get("on").and_then(|v| v.as_bool()).unwrap_or(false);
                self.call(move |core| core.set_context_freshness(&sid, on))?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "edit_session" => {
                // 动作信封的 session_id 不属于编辑体：先摘掉再按严格 DTO 反序列化。
                let mut body = args.clone();
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("session_id");
                }
                let edit: SessionEdit =
                    serde_json::from_value(body).map_err(|e| format!("编辑内容非法：{}", e))?;
                self.edit(&s("session_id"), edit)?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "upload" => {
                let data = s("data_base64");
                let max_encoded = MAX_ASSET_BYTES.div_ceil(3) * 4;
                if data.len() as u64 > max_encoded {
                    return Err(format!(
                        "文件超过单文件上限（{} MiB）",
                        MAX_ASSET_BYTES / 1024 / 1024
                    ));
                }
                let bytes = base64_decode(&data)?;
                let overwrite = args
                    .get("overwrite")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let uploaded = self.upload(&s("session_id"), &s("name"), &bytes, overwrite)?;
                Ok(Acted::Done(
                    serde_json::json!({ "ok": true, "uploaded": uploaded }),
                ))
            }
            // 工作区回滚：只给 user；非破坏性，expect_head 防"看完详情后别人又提交了"。
            "restore_work_commit" => {
                let sid = s("session_id");
                let commit = args
                    .get("commit")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| "restore_work_commit 需要 commit（提交号）".to_string())?;
                let expect_head = args
                    .get("expect_head")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| {
                        "restore_work_commit 需要 expect_head（看到详情时的 head；空共享区传 0）"
                            .to_string()
                    })?;
                let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
                let target = sid.clone();
                let (after, events) = self.call(move |core| {
                    core.restore_work_commit(&target, commit, expect_head, force)
                })?;
                self.bus.push(&sid, &events);
                Ok(Acted::Done(
                    serde_json::json!({ "ok": true, "head": after }),
                ))
            }
            // 隐秘字段：值只进去、永不回显；不放模块工具面（agent 不需要写值）。
            "set_secret" => {
                self.secrets.set(&s("module"), &s("name"), &s("value"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "clear_secret" => {
                self.secrets.clear(&s("module"), &s("name"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            // 文件域 / 协作动词 / 核心操作这些动作由成员工具循环执行，不经分发器。
            // 登记处动作（供应商 / 密钥 / 模型 / agent / 设置）：产品级资源，只给人用。
            "upsert_provider" | "remove_provider" | "discover_models" | "upsert_model"
            | "remove_model" | "set_core_model" | "probe_model_tools" | "probe_replay_shape"
            | "upsert_agent" | "remove_agent" | "set_settings" => self.execute_registry(id, args),
            other => Err(format!("这个动作不由分发器执行：{}", other)),
        }
    }
}

impl ActionOps for Handle {
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
            && let Ok(roster) = self.call(|core| Ok(core.modules_handle().roster()))
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

/// 目的：一次建会话的准备结果（两段之间的交接物）。
pub(crate) struct PreparedSession {
    work: crate::capabilities::work::api::PreparedWork,
}

impl Core {
    /// 目的：规整一次建会话的**目标环境**（档位 / 虚拟根）：有父沿用父会话的执行段，顶层取载荷或设置默认。
    /// 参数：`spec` = 规整后的载荷。
    /// 返回：(档位, 虚拟根名)。
    /// 错误：档位名不合法。
    /// 约束：与准备段同一份判据（准备段也调它）——创建前的补编因此按同一个目标环境编。
    pub(crate) fn resolve_env(
        &self,
        spec: &d::NewSession,
    ) -> Result<(Tier, Option<String>), String> {
        let parent_exec = match &spec.parent {
            Some(p) => Some(self.history.meta(p)?.exec),
            None => None,
        };
        let tier = match &parent_exec {
            Some(exec) => exec.tier,
            None => match spec
                .tier
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                Some(t) => Tier::parse(t)?,
                None => self.registry.app_settings().tier,
            },
        };
        let guest = match &parent_exec {
            Some(exec) => exec.guest.clone(),
            None => spec
                .guest
                .as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .map(|b| b.to_string()),
        };
        Ok((tier, guest))
    }

    /// 目的：建会话的第一段（核心，毫秒级）——规整载荷、占名、备好工作区与计划。
    /// 返回：准备结果 + 可选的那段"要等 guest"的活（虚拟机档才有）。
    pub(crate) fn prepare_session(
        &mut self,
        spec: &d::NewSession,
    ) -> Result<(PreparedSession, Option<GuestBoot>), String> {
        let mode = match spec.mode {
            d::SessionMode::Single => WorkMode::Single,
            d::SessionMode::Collab => WorkMode::Collab,
            d::SessionMode::Proxy => WorkMode::Proxy,
        };
        let parent = spec.parent.clone();
        if mode == WorkMode::Proxy && parent.is_some() {
            return Err("代理不能往里套代理".to_string());
        }
        let agents: Vec<AgentMeta> = spec
            .agents
            .iter()
            .map(|a| AgentMeta {
                name: a.name.clone(),
                transient: a.transient,
                modules: a.modules.clone(),
                model: a.model.clone(),
                chara: a.chara.clone(),
            })
            .collect();
        // 名字：点名就用它（撞名加尾号），否则派生（子会话按父 + agent 名，顶层按 agent 名）。
        let base = spec
            .agents
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| format!("w-{}", spec.request_id));
        let derived = match &parent {
            Some(p) => format!("{}--{}", p, base),
            None => base,
        };
        let name = match spec
            .name
            .as_ref()
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
        {
            Some(n) => self.unique_work_name(n, &derived),
            None => self.unique_work_name(&derived, "work"),
        };
        // 环境属于主会话：有父就整段沿用父会话的 ExecSpec（档位 / 虚拟根 / 网络 / 定版），
        // 不给子会话留任何单独覆盖的入口。顶层仍由人经呈现层给档位与虚拟根。
        let (tier, vm_guest) = self.resolve_env(spec)?;
        // 本次需求：协作必须有；单 agent 的"开头"由调用方在开工那一刻给（见 create_session_call）。
        let task = if mode == WorkMode::Collab {
            let t = spec.task.trim();
            if t.is_empty() {
                return Err("协作模式必须填写本次需求（task）".to_string());
            }
            Some(t.to_string())
        } else {
            None
        };
        // 委托代拟由形态派生：协作、又没给名单 = 交给核心按需求拟名单（不再是一个参数）。
        let delegate = mode == WorkMode::Collab && agents.is_empty();
        let work = WorkSpec {
            name,
            mode,
            agents,
            task,
            delegate,
            tier,
            permission: spec.permission,
            guest: vm_guest,
            workspace: spec
                .workspace
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from),
            workspace_grant: spec
                .workspace_grant
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(crate::capabilities::conversation::session::api::WorkspaceGrant::parse)
                .transpose()?,
        };
        // 有父 = 子工作（`create_work_inner` 直接给 parent）；没有父 = 顶层会话，走同一个入口包装。
        let prepared = match parent.as_deref() {
            Some(p) => self.prepare_work(work, Some(p))?,
            None => self.prepare_work(work, None)?,
        };
        Ok((PreparedSession { work: prepared.0 }, prepared.1))
    }

    /// 目的：建会话的第二段（核心）——装配会话对象、落盘身份、开场事实。
    pub(crate) fn commit_session(
        &mut self,
        prepared: PreparedSession,
    ) -> Result<(d::Created, Vec<SessionEvent>), String> {
        let opened = self.commit_work(prepared.work)?;
        Ok((
            d::Created {
                session: opened.sid,
                agents: opened.agents,
            },
            opened.facts,
        ))
    }

    /// 目的：建会话失败时撤销准备段（占名与空工作区目录）。
    pub(crate) fn abort_session(&mut self, prepared: PreparedSession) {
        self.abort_work(prepared.work);
    }
}

#[cfg(test)]
impl Core {
    /// 目的：**建会话的唯一实现**——父会话由 `spec.parent` 给出（None = 顶层会话，人经呈现层建的）。
    /// 参数：`spec` 是规整后的载荷（名字 / 档位 / 名单 / 本次需求）。
    /// 返回：建出的会话与名单 + 开场事实（事实由 api 层发布到事件台）。
    /// 错误：形态、名字、档位或名单不成立时如实拒绝，不留半成品。
    #[cfg(test)]
    pub(crate) fn create_session(
        &mut self,
        spec: &d::NewSession,
    ) -> Result<(d::Created, Vec<SessionEvent>), String> {
        let (prepared, boot) = self.prepare_session(spec)?;
        if let Some(boot) = boot
            && let Err(e) = boot.run()
        {
            self.abort_session(prepared);
            return Err(e);
        }
        self.commit_session(prepared)
    }
}
