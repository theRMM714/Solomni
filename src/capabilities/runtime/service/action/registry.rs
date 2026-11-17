//! 目的：登记处类动作与探测回包：供应商 / 模型 / agent / 全局设置的读写都转登记处的用例。
//! 管：execute_registry、apply_settings、probe_json，以及探测结论到 JSON 的纯逻辑 probe_view。
//! 不管：登记处内部怎么校验（registry 能力）；探测怎么做（llm）。
//! 联动：execute_action 的登记处分支调它；probe_json 的结果原样回给前端。

use super::*;

impl Handle {
    /// 目的：登记处动作的执行体——供应商 / 密钥 / 模型 / agent / 设置，**只给人用**（`callers: [user]`）。
    /// 约束：`api_key` 只进登记处，不进动作目录、不进审计；`set_settings` 是部分更新（未提交字段保留现值）。
    pub(super) fn execute_registry(
        &self,
        id: &str,
        args: &serde_json::Value,
    ) -> Result<Acted, String> {
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
                // 思考强度：省略 = 保留原值（与 tools 同一条规则）；给了就整段替换。
                let thinking = match args.get("thinking") {
                    Some(v) => serde_json::from_value(v.clone())
                        .map_err(|e| format!("思考强度非法：{}", e))?,
                    None => RegistryOps::models(self)?
                        .into_iter()
                        .find(|m| m.id == s("id"))
                        .map(
                            |m| crate::capabilities::catalog::registry::api::ThinkingSpec {
                                default: m.thinking.default,
                                levels: m.thinking.levels,
                            },
                        )
                        .unwrap_or_default(),
                };
                // 工具调用形态：表单带 tools 就用它；没带 = 编辑保留原值，新建缺省（native）。
                let tools = match args.get("tools").and_then(|v| v.as_str()) {
                    Some(raw) => serde_json::from_value::<crate::capabilities::llm::api::ToolMode>(
                        serde_json::json!(raw),
                    )
                    .map_err(|_| format!("工具调用方式只认 native / envelope：{}", raw))?,
                    None => self
                        .models()?
                        .into_iter()
                        .find(|m| m.id == s("id"))
                        .map(|m| m.tools)
                        .unwrap_or_default(),
                };
                let draft = crate::capabilities::catalog::registry::api::ModelDraft {
                    id: s("id"),
                    name: s("name"),
                    api_model: s("api_model"),
                    provider: s("provider"),
                    note: s("note"),
                    tools,
                    context,
                    thinking,
                };
                self.upsert_model(draft)?;
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
                self.upsert_agent(&s("name"), &modules, &s("model"), &s("note"), &s("chara"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": true })))
            }
            "remove_agent" => {
                let ok = self.remove_agent(&s("name"))?;
                Ok(Acted::Done(serde_json::json!({ "ok": ok })))
            }
            "set_settings" => {
                let before = self.settings()?.sandbox;
                self.apply_settings(args)?;
                let after = self.settings()?.sandbox;
                // 沙箱策略是进程级启动事实：变了就要求整体重启，避免旧 guest / 旧快照半生效。
                Ok(Acted::Done(
                    serde_json::json!({ "ok": true, "restart": before != after }),
                ))
            }
            other => Err(format!("未知的登记处动作：{}", other)),
        }
    }

    /// 目的：设置的部分更新——未提交的字段保留现值；`sandbox` 整段替换（全局硬约束与会话默认值）。
    /// 约束：沙箱段由登记处校验；本层不解释策略。
    pub(super) fn apply_settings(&self, args: &serde_json::Value) -> Result<(), String> {
        let current = self.settings()?;
        let num = |k: &str| args.get(k).and_then(|v| v.as_u64());
        let sandbox = match args.get("sandbox") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| format!("settings.sandbox 配置非法：{}", e))?,
            None => current.sandbox.clone(),
        };
        let thinking = match args.get("thinking") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| format!("settings.thinking 配置非法：{}", e))?,
            None => current.thinking.clone(),
        };
        let storage: StorageSettings = match args.get("storage") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| format!("settings.storage 配置非法：{}", e))?,
            None => current.storage.clone(),
        };
        let settings = AppSettings {
            streaming: args
                .get("streaming")
                .and_then(|v| v.as_bool())
                .unwrap_or(current.streaming),
            show_reasoning: args
                .get("show_reasoning")
                .and_then(|v| v.as_bool())
                .unwrap_or(current.show_reasoning),
            thinking,
            tier: current.tier,
            sandbox,
            qemu_path: current.qemu_path.clone(),
            virtiofsd_path: current.virtiofsd_path.clone(),
            smbd_path: current.smbd_path.clone(),
            max_guests: num("max_guests")
                .map(|v| v as u32)
                .unwrap_or(current.max_guests),
            storage,
            llm_timeout_secs: num("llm_timeout_secs").unwrap_or(current.llm_timeout_secs),
            compact_at_percent: num("compact_at_percent")
                .map(|v| v as u8)
                .unwrap_or(current.compact_at_percent),
            compact_keep_percent: num("compact_keep_percent")
                .map(|v| v as u8)
                .unwrap_or(current.compact_keep_percent),
            compact_model: args
                .get("compact_model")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| current.compact_model.clone()),
            discuss_remind_cap: num("discuss_remind_cap")
                .map(|v| v as u32)
                .unwrap_or(current.discuss_remind_cap),
        };
        self.set_settings(settings)
    }

    /// 目的：探测结论 → 响应 JSON；`mode` 取探测后登记处里的实际形态（结论只翻译、不解释）。
    pub(super) fn probe_json(
        &self,
        id: &str,
        outcome: &crate::capabilities::llm::api::ProbeOutcome,
    ) -> serde_json::Value {
        let mode = RegistryOps::models(self)
            .ok()
            .and_then(|ms| ms.into_iter().find(|m| m.id == id))
            .map(|m| m.tools);
        probe_view(outcome, mode)
    }
}

/// 目的：探测结论 → 响应 JSON；`mode` 是探测后登记处里的**实际**形态（None = 取不到）。
/// 约束：结论只翻译、不解释——三种取值原样穿过，呈现层与前端不各自改写。
pub fn probe_view(outcome: &ProbeOutcome, mode: Option<ToolMode>) -> serde_json::Value {
    let (kind, detail) = match outcome {
        ProbeOutcome::Supported { detail } => ("supported", detail.clone()),
        ProbeOutcome::Unsupported { detail } => ("unsupported", detail.clone()),
        ProbeOutcome::Unknown { detail } => ("unknown", detail.clone()),
    };
    serde_json::json!({ "ok": true, "outcome": kind, "detail": detail, "mode": mode })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三种结论**原样**穿过（不改写、不降级）；`mode` 取登记处的实际值，不由结论反推。
    #[test]
    fn probe_view_reports_every_verdict_verbatim() {
        let cases = [
            (
                ProbeOutcome::Supported {
                    detail: "真的调了".to_string(),
                },
                "supported",
                "真的调了",
            ),
            (
                ProbeOutcome::Unsupported {
                    detail: "供应商说 tools 不认识".to_string(),
                },
                "unsupported",
                "供应商说 tools 不认识",
            ),
            (
                ProbeOutcome::Unknown {
                    detail: "没发起调用".to_string(),
                },
                "unknown",
                "没发起调用",
            ),
        ];
        for (outcome, want, detail) in cases {
            let v = probe_view(&outcome, Some(ToolMode::Envelope));
            assert_eq!(v["outcome"], want);
            assert_eq!(v["detail"], detail);
            assert_eq!(v["mode"], "envelope", "形态取登记处现有值: {}", v);
        }
        let no_mode = probe_view(
            &ProbeOutcome::Unknown {
                detail: "x".to_string(),
            },
            None,
        );
        assert!(no_mode["mode"].is_null());
    }
}
