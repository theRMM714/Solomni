//! 目的：conductor 自己的会话 / 工作动作——建会话、发消息、裁决、回档、改配置、上传与模块启停。
//! 管：`create_session_call` / `control_session_call` 两条唯一实现、`build_new_session` 参数规整，以及会话 / 工作动词的执行体 `execute_session_action`。
//! 不管：模块工具与常驻服务的执行面（在 `module.rs`）；登记处动词（在 `registry.rs`）；授权与审计（在 `action.rs`）。
//! 联动：动作表声明与 callers 在 `systools/tools.yaml`；被调能力面是 `TurnOps` / `LifecycleOps` / `WorkOps`。

use super::*;
use crate::capabilities::proxy::api as d;
use crate::capabilities::workspace::api::{MAX_ASSET_BYTES, base64_decode};

impl ConductorHandle {
    /// 目的：建会话的**唯一实现**——人经呈现层（没有父）与核心代理（有父）都走这条。
    /// 参数：`caller` 是调用者身份；`spec` 是规整后的载荷（父会话由机制填）。
    /// 返回：建出的会话与名单；开场事实已推上事件台。
    /// 错误：名字 / 档位 / 名单不成立时如实拒绝，不留半成品。
    pub(crate) fn create_session_call(
        &self,
        caller: &Caller,
        spec: &d::NewSession,
    ) -> Result<d::Created, String> {
        // 先补编：名单里声明了 `build:` 的模块若在目标环境还没有入口，就在这一段（调用方线程）编好——
        // 核心线程不陪着等编译器；编不成如实拒绝，不进入准备段（什么都不留下）。
        let (tier, guest) = self.call({
            let spec = spec.clone();
            move |core| core.work.resolve_env(&*core.registry, &spec)
        })?;
        let mut ids: Vec<String> = Vec::new();
        for a in &spec.agents {
            for m in &a.modules {
                if !ids.contains(m) {
                    ids.push(m.clone());
                }
            }
        }
        self.ensure_built(&ids, tier, guest.as_deref())?;
        // **两段**：核心只做校验 / 占名 / 出计划（毫秒级）；起 guest 要等几十秒到几分钟，
        // 在这一段（调用方线程）上跑——核心线程因此不被占住，别的请求照常响应。
        let (prepared, boot) = self.call({
            let spec = spec.clone();
            move |core| {
                core.work
                    .prepare_session(&mut core.turn, &*core.registry, &spec)
            }
        })?;
        if let Some(boot) = boot
            && let Err(e) = boot.run()
        {
            let _ = self.call(move |core| {
                core.work.abort_session(&mut core.turn, prepared);
                Ok(())
            });
            return Err(e);
        }
        let (created, facts) = self.call(move |core| {
            core.work
                .commit_session(&mut core.turn, &*core.registry, prepared)
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
                move |core| core.turn.dispatch_target(&c)
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
        let note = d::control_note(action, why);
        // 「继续」在真正跑模型之前先把动作记下来：否则整轮生成结束前转录里没有任何痕迹，
        // 用户看到的只是"点了一下、什么都没发生"。停止/关闭仍按实际操作结果记。
        if action == d::ControlAction::Continue {
            self.record_notice(sid, SessionEvent::Notice(note.clone()));
        }
        let state = match action {
            d::ControlAction::Stop => {
                // 走会话能力的停止：冻结整棵子树 + 级联中断在跑的生成（回执给实际停下的会话数）。
                let stopped = TurnOps::stop(self, sid);
                format!("stopped:{}", stopped.len())
            }
            d::ControlAction::Continue => match caller {
                // 人经呈现层：接着走的那一轮按常规生成跑完（与历史行为一致）。
                Caller::User => {
                    let _ = TurnOps::continue_flow(self, sid, Output::Final)?;
                    "active".to_string()
                }
                // 核心代理：解冻后**脱离调用方点火**——它不等子会话跑完，靠 observe / messages 回头看。
                Caller::Role { .. } => {
                    let resumed = self.call({
                        let s = sid.to_string();
                        move |core| core.work.resume_subtree(&core.turn, &s)
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
        if action != d::ControlAction::Continue {
            self.record_notice(sid, SessionEvent::Notice(note));
        }
        Ok(state)
    }

    /// 目的：把呈现层 / 模型给的参数规整成建会话载荷（ref 形式在这里查登记处）。
    pub(super) fn build_new_session(
        &self,
        args: &serde_json::Value,
    ) -> Result<d::NewSession, String> {
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
                        chara: v.chara,
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
                let chara = obj
                    .get("chara")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                agents.push(d::NewAgent {
                    name,
                    transient,
                    modules,
                    model,
                    chara,
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
            guest: args
                .get("guest")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            parent: None,
            permission: args
                .get("permission")
                .and_then(|v| v.as_str())
                .map(crate::capabilities::permission::api::ProfileId::parse)
                .transpose()?,
            workspace: args
                .get("workspace")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
            workspace_grant: args
                .get("workspace_grant")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
        })
    }

    /// 目的：会话 / 工作动作的执行体（分派、授权与参数校验已在 `action.rs` 里做完）。
    pub(super) fn execute_session_action(
        &self,
        id: &str,
        caller: &Caller,
        args: &serde_json::Value,
        out: Output,
    ) -> Result<Acted, String> {
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
                let view = LifecycleOps::activate(self, &s("session_id"))?;
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
                self.call(move |core| {
                    core.work
                        .set_permission_profile(&mut core.turn, &*core.registry, &sid, profile)
                })?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "set_thinking" => {
                let sid = s("session_id");
                let level = s("level");
                self.call(move |core| {
                    core.work
                        .set_thinking(&mut core.turn, &*core.registry, &sid, &level)
                })?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "set_context_freshness" => {
                let sid = s("session_id");
                let on = args.get("on").and_then(|v| v.as_bool()).unwrap_or(false);
                self.call(move |core| {
                    core.work
                        .set_context_freshness(&mut core.turn, &*core.registry, &sid, on)
                })?;
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
                    core.work.restore_work_commit(
                        &mut core.turn,
                        &target,
                        commit,
                        expect_head,
                        force,
                    )
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
            other => Err(format!("这个动作不由分发器执行：{}", other)),
        }
    }
}
