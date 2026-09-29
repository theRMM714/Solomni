//! **队列代理**：把各能力的入站契约挂在核心手柄上——呈现层因此只持 `Ops`，拿不到 `Conductor`。
//!
//! 每个方法都是「克隆要用的东西 → 进队列执行 → 取回结果」，不做业务判断（判断在 `service/`）。

use super::*;
use crate::capabilities::registry::api::RegistryOps;
use crate::capabilities::registry::api::{AgentView, AppSettings, ModelView, ProviderView};
use crate::capabilities::session::api::AgentMeta;
pub use crate::capabilities::session::api::Pending;
use crate::capabilities::session::api::{HistoryView, SessionMeta};
use crate::capabilities::workspace::api::Roster;
pub use crate::kernel::api::Tier;
use std::sync::Arc;

impl SessionOps for ConductorHandle {
    fn create_work(&self, spec: WorkSpec) -> Result<(WorkOpened, u64), String> {
        let bus = Arc::clone(&self.bus);
        self.call(move |core| {
            let opened = core.create_work(spec)?;
            let head = bus.push(&opened.sid, &opened.facts);
            Ok((opened, head))
        })
    }

    fn say(&self, sid: &str, text: &str, out: Output) -> Result<Advance, String> {
        self.single_generation(sid, Some(text.to_string()), out)
    }

    fn continue_flow(&self, sid: &str, out: Output) -> Result<Advance, String> {
        self.single_generation(sid, None, out)
    }

    fn collab_step(&self, sid: &str, step: CollabStep, text: &str) -> Result<Advance, String> {
        match step {
            // 短步骤（写需求 / 定名单）不调模型，而且"定名单"还有落盘与建沙箱的后续——留在核心线程上。
            CollabStep::SetTask | CollabStep::ConfirmSlate => {
                let sid = sid.to_string();
                let text = text.to_string();
                let bus = Arc::clone(&self.bus);
                self.call(move |core| {
                    let events = core.collab_continue(&sid, step, &text)?;
                    let head = bus.push(&sid, &events);
                    Ok(Advance { head })
                })
            }
            // 长步骤（开始讨论 / 回答）：队列只占"取/交"两步，泵在工作线程上跑。
            // 「同意方案」也要跑泵（过关后接着推进），所以和长步骤走同一条路。
            CollabStep::Begin | CollabStep::Decide => {
                self.collab_generation(sid, CollabWork::Step(step), text)
            }
        }
    }

    fn withdraw_agree(&self, sid: &str, agent: &str) -> Result<Advance, String> {
        let sid = sid.to_string();
        let agent = agent.to_string();
        let bus = Arc::clone(&self.bus);
        self.call(move |core| {
            let events = core.withdraw_agree(&sid, &agent)?;
            let head = bus.push(&sid, &events);
            Ok(Advance { head })
        })
    }

    fn slate(&self, sid: &str) -> Result<Vec<AgentMeta>, String> {
        let sid = sid.to_string();
        self.call(move |core| core.collab_slate(&sid))
    }

    fn compact(&self, sid: &str) -> Result<Advance, String> {
        ConductorHandle::compact(self, sid)
    }

    fn rewind(&self, sid: &str, keep_id: u64) -> Result<Vec<serde_json::Value>, String> {
        let sid = sid.to_string();
        self.call(move |core| core.rewind(&sid, keep_id))
    }

    fn update_task(&self, sid: &str, text: &str) -> Result<Vec<serde_json::Value>, String> {
        let sid = sid.to_string();
        let text = text.to_string();
        self.call(move |core| core.update_task(&sid, &text))
    }

    fn pending(&self, sid: &str) -> Result<Option<Pending>, String> {
        let sid = sid.to_string();
        self.call(move |core| core.collab_pending(&sid))
    }

    fn config(&self, sid: &str) -> Result<SessionConfig, String> {
        let sid = sid.to_string();
        self.call(move |core| core.session_config(&sid))
    }

    fn edit(&self, sid: &str, edit: SessionEdit) -> Result<(), String> {
        let sid = sid.to_string();
        self.call(move |core| core.edit_session(&sid, edit))
    }

    fn upload(&self, sid: &str, name: &str, bytes: &[u8], overwrite: bool) -> Result<bool, String> {
        let sid = sid.to_string();
        let name = name.to_string();
        let bytes = bytes.to_vec();
        self.call(move |core| core.work_upload(&sid, &name, &bytes, overwrite))
    }

    fn files(&self, sid: &str) -> Result<FilesView, String> {
        let sid = sid.to_string();
        self.call(move |core| core.files_view(&sid))
    }

    fn exists(&self, sid: &str) -> Result<bool, String> {
        let sid = sid.to_string();
        self.call(move |core| Ok(core.session_exists(&sid)))
    }

    fn unique_work_name(&self, base: &str, fallback: &str) -> Result<String, String> {
        let (base, fallback) = (base.to_string(), fallback.to_string());
        self.call(move |core| Ok(core.unique_work_name(&base, &fallback)))
    }

    /// 停止**不走命令队列**：直接置位取消标志，所以生成期间照样立刻生效。
    fn stop(&self, sid: &str) -> bool {
        self.jobs.stop(sid)
    }

    fn is_running(&self, sid: &str) -> bool {
        self.jobs.is_running(sid)
    }

    fn session_views(&self, history: &[HistoryView]) -> Result<Vec<SessionView>, String> {
        let history = history.to_vec();
        self.call(move |core| Ok(core.session_views(&history)))
    }
}

impl RegistryOps for ConductorHandle {
    fn providers(&self) -> Result<Vec<ProviderView>, String> {
        self.call(|core| Ok(core.registry().provider_views()))
    }
    fn upsert_provider(&self, id: &str, base_url: &str, api_key: &str) -> Result<(), String> {
        let (id, base_url, api_key) = (id.to_string(), base_url.to_string(), api_key.to_string());
        self.call(move |core| {
            core.registry_mut()
                .provider_upsert(&id, &base_url, &api_key)
        })
    }
    fn remove_provider(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.registry_mut().provider_remove(&id))
    }
    fn models(&self) -> Result<Vec<ModelView>, String> {
        self.call(|core| Ok(core.registry().model_views()))
    }
    fn core_model(&self) -> Result<Option<String>, String> {
        self.call(|core| Ok(core.registry().core_model()))
    }
    fn upsert_model(
        &self,
        id: &str,
        name: &str,
        api_model: &str,
        provider: &str,
        note: &str,
        context: u64,
    ) -> Result<(), String> {
        let (id, name, api_model, provider, note) = (
            id.to_string(),
            name.to_string(),
            api_model.to_string(),
            provider.to_string(),
            note.to_string(),
        );
        self.call(move |core| {
            core.registry_mut()
                .model_upsert(&id, &name, &api_model, &provider, &note, context)
        })
    }
    fn remove_model(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.registry_mut().model_remove(&id))
    }
    fn set_core_model(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        self.call(move |core| core.registry_mut().core_set_model(&id))
    }
    fn agents(&self) -> Result<Vec<AgentView>, String> {
        self.call(|core| Ok(core.registry().agent_views()))
    }

    fn pick_agents(&self, names: &[String]) -> Result<Vec<AgentView>, String> {
        let names = names.to_vec();
        self.call(move |core| core.registry().pick_agents(&names))
    }
    fn upsert_agent(
        &self,
        name: &str,
        modules: &[String],
        model: &str,
        note: &str,
    ) -> Result<(), String> {
        let (name, modules, model, note) = (
            name.to_string(),
            modules.to_vec(),
            model.to_string(),
            note.to_string(),
        );
        // 模块存在性按**清单**校验：清单归 workspace，登记处只认事实，所以清单由这里取一份给它。
        self.call(move |core| {
            let roster = core.scan();
            core.registry_mut()
                .agent_upsert(&name, &modules, &model, &note, &roster)
        })
    }
    fn remove_agent(&self, name: &str) -> Result<bool, String> {
        let name = name.to_string();
        self.call(move |core| core.registry_mut().agent_remove(&name))
    }
    fn settings(&self) -> Result<AppSettings, String> {
        self.call(|core| Ok(core.registry().app_settings()))
    }
    fn set_settings(&self, app: AppSettings) -> Result<(), String> {
        self.call(move |core| core.registry_mut().set_app_settings(app))
    }
    fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String> {
        let provider_id = provider_id.to_string();
        self.call(move |core| core.registry().discover_models(&provider_id))
    }
    fn probe_model_tools(
        &self,
        id: &str,
    ) -> Result<crate::capabilities::llm::api::ProbeOutcome, String> {
        let id = id.to_string();
        self.call(move |core| core.registry_mut().probe_model_tools(&id))
    }
    fn probe_replay_shape(
        &self,
        id: &str,
    ) -> Result<crate::capabilities::llm::api::ReplayReport, String> {
        let id = id.to_string();
        self.call(move |core| core.registry().probe_replay_shape(&id))
    }
}

// ---------- 归各能力自己的入站契约（本文件只实现队列代理） ----------

impl crate::capabilities::session::api::HistoryOps for ConductorHandle {
    fn list(&self) -> Result<Vec<HistoryView>, String> {
        self.call(|core| Ok(core.history_list()))
    }
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let name = name.to_string();
        self.call(move |core| core.history_open(&name))
    }
    fn delete(&self, name: &str) -> Result<bool, String> {
        let name = name.to_string();
        self.call(move |core| core.history_delete(&name))
    }
}

impl crate::capabilities::workspace::api::WorkspaceOps for ConductorHandle {
    fn roster(&self) -> Result<Roster, String> {
        self.call(|core| Ok(core.scan()))
    }
}

impl ConductorOps for ConductorHandle {
    fn runtime_report(&self, tier: Tier) -> Result<RuntimeReport, String> {
        self.call(move |core| Ok(core.runtime_report(tier)))
    }
    fn suggest_models(&self, task: &str, mode: WorkMode) -> Result<Vec<AgentSuggestion>, String> {
        let task = task.to_string();
        let (agents, rows) = self.call(move |core| core.suggest_models(&task, mode))?;
        // 核心的行**照推**（推是底层收发消息的统一定律，一次推荐也不例外），推到系统会话：
        // 它不在任何会话表里，前端因此不会为它建标签页；落盘策略是 Drop（只推不留）。
        if !rows.is_empty() {
            self.bus.push(
                crate::capabilities::conductor::service::SYSTEM_SID_SUGGEST,
                &rows,
            );
        }
        Ok(agents)
    }
}

// ---------- 日志能力 ----------

impl LogOps for ConductorHandle {
    fn info(&self, at: &str, msg: &str) {
        self.log.info(at, msg)
    }
    fn warn(&self, at: &str, msg: &str) {
        self.log.warn(at, msg)
    }
    fn error(&self, at: &str, msg: &str) {
        self.log.error(at, msg)
    }
}
