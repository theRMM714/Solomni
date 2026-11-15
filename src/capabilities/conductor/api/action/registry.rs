//! 目的：登记处动作的适配——只做参数搬运与入队，解析规则在 registry 的 domain。
//! 管：`execute_registry`（供应商 / 模型 / agent / 设置的 11 个分支）与 `probe_json`（探测结论 → 响应）。
//! 不管：设置合并与模型草稿的纯解析（在 registry 能力的 domain）；队列与落盘（在 registry 的 service）。
//! 联动：动作表把登记处动词分派到这里；登记处能力面是 `RegistryOps`。

use super::*;
use crate::capabilities::registry::api::{RegistryOps, apply_settings, model_draft, probe_view};

impl ConductorHandle {
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
                // 思考强度与工具形态省略时要从原模型继承：只有真省略了才去读一次登记处。
                let prev = match (
                    args.get("thinking"),
                    args.get("tools").and_then(|v| v.as_str()),
                ) {
                    (Some(_), Some(_)) => None,
                    _ => RegistryOps::models(self)?
                        .into_iter()
                        .find(|m| m.id == s("id")),
                };
                let draft = model_draft(args, prev.as_ref())?;
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
                let current = self.settings()?;
                self.set_settings(apply_settings(&current, args)?)?;
                let after = self.settings()?.sandbox;
                // 沙箱策略是进程级启动事实：变了就要求整体重启，避免旧 guest / 旧快照半生效。
                Ok(Acted::Done(
                    serde_json::json!({ "ok": true, "restart": before != after }),
                ))
            }
            other => Err(format!("未知的登记处动作：{}", other)),
        }
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
