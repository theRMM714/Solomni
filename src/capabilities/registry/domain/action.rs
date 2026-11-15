//! 目的：登记处动作的纯解析——设置的局部合并、模型草稿的缺省继承、探测结论的回包。
//! 管：`apply_settings`（未提交字段保留现值）、`model_draft`（省略的思考强度与工具形态从原模型继承）、`probe_view`（探测结论 → 响应 JSON）。
//! 不管：登记处的落盘与队列（在 `service.rs` / `api.rs`）；设置与模型的校验规则（在 `providers.rs`）。
//! 联动：由 `registry/api.rs` 重导出；conductor 的登记处动作只做参数搬运与入队。

use super::providers::{AppSettings, ModelDraft, ModelView, StorageSettings, ThinkingSpec};
use crate::capabilities::llm::api::{ProbeOutcome, ToolMode};

/// 目的：设置的部分更新——未提交的字段保留现值；`sandbox` / `thinking` / `storage` 整段替换。
/// 参数：`current` = 登记处此刻的设置；`args` = 动作参数（只认提交了的键）。
/// 返回：合并后的设置；三段之一解析失败时按段给错误文案，不改 `current`。
pub fn apply_settings(
    current: &AppSettings,
    args: &serde_json::Value,
) -> Result<AppSettings, String> {
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
    Ok(AppSettings {
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
    })
}

/// 目的：动作参数 → 模型草稿；省略的思考强度与工具形态从原模型（同 id）继承。
/// 参数：`args` = 动作参数；`prev` = 登记处里同 id 的现有模型（新建时 None）。
/// 返回：可直接入队的草稿；思考强度或工具形态非法时给固定文案。
/// 约束：`tools` 只认字符串；`thinking` 整段替换，不逐档合并。
pub fn model_draft(
    args: &serde_json::Value,
    prev: Option<&ModelView>,
) -> Result<ModelDraft, String> {
    let text = |k: &str| {
        args.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let context = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0);
    let thinking = match args.get("thinking") {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| format!("思考强度非法：{}", e))?,
        None => prev
            .map(|m| ThinkingSpec {
                default: m.thinking.default.clone(),
                levels: m.thinking.levels.clone(),
            })
            .unwrap_or_default(),
    };
    // 表单带 tools 就用它；没带 = 编辑保留原值，新建缺省（native）。
    let tools = match args.get("tools").and_then(|v| v.as_str()) {
        Some(raw) => serde_json::from_value::<ToolMode>(serde_json::json!(raw))
            .map_err(|_| format!("工具调用方式只认 native / envelope：{}", raw))?,
        None => prev.map(|m| m.tools).unwrap_or_default(),
    };
    Ok(ModelDraft {
        id: text("id"),
        name: text("name"),
        api_model: text("api_model"),
        provider: text("provider"),
        note: text("note"),
        tools,
        context,
        thinking,
    })
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
