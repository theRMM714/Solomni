//! 目的：建会话与控会话的动作：解析参数、补编模块、跑两段式、派发第一轮。
//! 管：create_session_call（建会话 + 派发）、control_session_call（停 / 继续 / 关闭）、build_new_session（参数解析）。
//! 不管：会话怎么建（work）、guest 怎么拉（activation）、怎么派发（turn）。
//! 联动：execute_action 的建会话与控会话分支调它；调用者身份由动作表给出。

use super::*;
use crate::capabilities::catalog::registry::api::AgentMeta;

impl Handle {
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
            move |core| core.resolve_env(&spec)
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
            move |core| core.prepare_session(&spec)
        })?;
        if let Some(boot) = boot
            && let Err(e) = boot.run()
        {
            let _ = self.call(move |core| {
                core.abort_session(prepared);
                Ok(())
            });
            return Err(e);
        }
        let (created, facts) = self.call(move |core| core.commit_session(prepared))?;
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
            if mode == crate::capabilities::conversation::session::api::Mode::Collab {
                crate::capabilities::turn::turn::api::start_child_collab(self, &created.session);
            } else {
                crate::capabilities::turn::turn::api::spawn_detached_node(
                    self,
                    &created.session,
                    &spec.task,
                );
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
                let stopped = RuntimeOps::stop(self, sid);
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
                        move |core| core.turn.resume_subtree(&s)
                    })?;
                    if resumed {
                        let mode = self.call({
                            let s = sid.to_string();
                            move |core| Ok(core.session_mode(&s))
                        })?;
                        match mode {
                            Some(crate::capabilities::conversation::session::api::Mode::Collab) => {
                                crate::capabilities::turn::turn::api::spawn_detached_collab(
                                    self, sid,
                                )
                            }
                            Some(crate::capabilities::conversation::session::api::Mode::Proxy) => {
                                crate::capabilities::turn::turn::api::spawn_detached_proxy(
                                    self, sid,
                                )
                            }
                            _ => crate::capabilities::turn::turn::api::spawn_detached_continue(
                                self, sid,
                            ),
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
        let mut agents: Vec<crate::capabilities::catalog::registry::api::AgentMeta> = Vec::new();
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
                    agents.push(AgentMeta::normalized(
                        v.name, v.modules, v.model, v.chara, false,
                    ));
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
                agents.push(AgentMeta::normalized(
                    name, modules, model, chara, transient,
                ));
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
}
