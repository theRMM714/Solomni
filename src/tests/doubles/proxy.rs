//! 目的：核心代理工具的宿主替身——只按用例给的答案回答，并记下每次动作。
//! 管：目录、建会话、转发、观察、控制与各自的失败注入。
//! 不管：真实宿主（`src/capabilities/conductor/detail/`）。
//! 联动：`src/capabilities/conductor/ports.rs` 的 `ProxyHost`。

use super::*;

use crate::capabilities::conductor::domain::proxy as dproxy;

use crate::capabilities::conductor::ports::ProxyHost;

/// 目的：代理工具的宿主替身：只按用例给的答案回答，并把每次动作记进日志（用户可见的“未授权 = 根本没碰宿主”就靠这份日志钉住）。
pub(crate) struct FakeProxyHost {
    pub(crate) log: Mutex<Vec<String>>,
    pub(crate) facts: dproxy::Catalog,
    pub(crate) fail_create: Mutex<Option<String>>,
    pub(crate) fail_send: Mutex<Vec<String>>,
    pub(crate) fail_control: Mutex<Vec<String>>,
    pub(crate) created: Mutex<Vec<dproxy::NewSession>>,
    pub(crate) relayed: Mutex<Vec<dproxy::Relayed>>,
    pub(crate) events: Mutex<Vec<String>>,
}

impl FakeProxyHost {
    pub(crate) fn new() -> FakeProxyHost {
        FakeProxyHost {
            log: Mutex::new(Vec::new()),
            facts: FakeProxyHost::facts(),
            fail_create: Mutex::new(None),
            fail_send: Mutex::new(Vec::new()),
            fail_control: Mutex::new(Vec::new()),
            created: Mutex::new(Vec::new()),
            relayed: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    fn facts() -> dproxy::Catalog {
        dproxy::Catalog {
            agents: vec![dproxy::AgentFact {
                name: "a".to_string(),
                modules: vec!["m1".to_string()],
                model: Some("gpt".to_string()),
                note: "已存".to_string(),
                chara: "人设".to_string(),
            }],
            modules: vec![
                dproxy::ModuleFact {
                    id: "m1".to_string(),
                    tools: vec!["t".to_string()],
                },
                dproxy::ModuleFact {
                    id: "m2".to_string(),
                    tools: Vec::new(),
                },
            ],
            models: vec![
                dproxy::ModelFact {
                    id: "gpt".to_string(),
                    name: "GPT".to_string(),
                    tools: "native".to_string(),
                },
                // 第二个模型：让“模型越出授权范围”能被单独钉住（否则会先被“无此模型”挡下）。
                dproxy::ModelFact {
                    id: "other".to_string(),
                    name: "Other".to_string(),
                    tools: "envelope".to_string(),
                },
            ],
        }
    }

    /// 目的：记下的每一次宿主动作（断言“未授权/越界时什么都没发生”）。
    pub(crate) fn calls(&self) -> Vec<String> {
        self.log.lock().expect("锁").clone()
    }

    pub(crate) fn created(&self) -> Vec<dproxy::NewSession> {
        self.created.lock().expect("锁").clone()
    }

    pub(crate) fn relayed(&self) -> Vec<dproxy::Relayed> {
        self.relayed.lock().expect("锁").clone()
    }

    /// 目的：预置一条消息（观察的计数与消息倒查都读它）。
    pub(crate) fn add_event(&self, text: &str) {
        self.events.lock().expect("锁").push(text.to_string());
    }

    pub(crate) fn fail_create(&self, why: &str) {
        *self.fail_create.lock().expect("锁") = Some(why.to_string());
    }

    pub(crate) fn clear_fail_create(&self) {
        *self.fail_create.lock().expect("锁") = None;
    }

    pub(crate) fn fail_send(&self, target: &str) {
        self.fail_send.lock().expect("锁").push(target.to_string());
    }

    pub(crate) fn fail_control(&self, action: &str) {
        self.fail_control
            .lock()
            .expect("锁")
            .push(action.to_string());
    }
}

impl ProxyHost for FakeProxyHost {
    fn catalog(&self, _scope: dproxy::CatalogScope) -> Result<dproxy::Catalog, String> {
        self.log.lock().expect("锁").push("catalog".to_string());
        Ok(self.facts.clone())
    }

    fn create_session(&self, spec: &dproxy::NewSession) -> Result<dproxy::Created, String> {
        self.log
            .lock()
            .expect("锁")
            .push(format!("create:{}", spec.request_id));
        if let Some(why) = self.fail_create.lock().expect("锁").clone() {
            return Err(why);
        }
        self.created.lock().expect("锁").push(spec.clone());
        Ok(dproxy::Created {
            session: format!("work-{}", spec.request_id),
            agents: spec.agents.iter().map(|a| a.name.clone()).collect(),
        })
    }

    fn send(&self, target: &str, msg: &dproxy::Relayed) -> Result<(), String> {
        self.log
            .lock()
            .expect("锁")
            .push(format!("send:{}:{}", target, msg.kind.as_str()));
        self.relayed.lock().expect("锁").push(msg.clone());
        if self
            .fail_send
            .lock()
            .expect("锁")
            .iter()
            .any(|t| t == target)
        {
            return Err(format!("目标 {} 不存在或已关闭", target));
        }
        Ok(())
    }

    fn observe(
        &self,
        session: &str,
        view: dproxy::ObserveView,
        since: Option<&str>,
    ) -> Result<dproxy::Snapshot, String> {
        self.log.lock().expect("锁").push(format!(
            "observe:{}:{:?}:{}",
            session,
            view,
            since.unwrap_or("")
        ));
        // 观察只回元信息：**不回消息正文**（正文走 messages 倒查）。
        let count = self.events.lock().expect("锁").len();
        Ok(dproxy::Snapshot {
            session: session.to_string(),
            state: "running".to_string(),
            pending: None,
            artifacts: None,
            message_count: Some(count),
            cursor: Some(count.to_string()),
            new_messages: None,
        })
    }

    fn messages(
        &self,
        session: &str,
        from: usize,
        count: usize,
    ) -> Result<dproxy::MessagesPage, String> {
        self.log
            .lock()
            .expect("锁")
            .push(format!("messages:{}:{}:{}", session, from, count));
        let events = self.events.lock().expect("锁").clone();
        let total = events.len();
        let mut out = Vec::new();
        let mut i = from;
        while i < total && out.len() < count {
            let idx = total - 1 - i;
            out.push(dproxy::MessageLine {
                id: idx as u64 + 1,
                speaker: "a".to_string(),
                verb: String::new(),
                kind: "msg".to_string(),
                text: events[idx].clone(),
            });
            i += 1;
        }
        let next = if i < total { Some(i) } else { None };
        Ok(dproxy::MessagesPage {
            session: session.to_string(),
            messages: out,
            next,
        })
    }

    fn control(
        &self,
        session: &str,
        action: dproxy::ControlAction,
        _reason: &str,
    ) -> Result<dproxy::ControlState, String> {
        self.log
            .lock()
            .expect("锁")
            .push(format!("control:{}:{}", session, action.as_str()));
        if self
            .fail_control
            .lock()
            .expect("锁")
            .iter()
            .any(|a| a == action.as_str())
        {
            return Err(format!(
                "会话 {} 不能 {}：当前状态不允许",
                session,
                action.as_str()
            ));
        }
        Ok(dproxy::ControlState {
            session: session.to_string(),
            action,
            state: match action {
                dproxy::ControlAction::Stop => "stopped",
                dproxy::ControlAction::Continue => "active",
                dproxy::ControlAction::Close => "closed",
            }
            .to_string(),
        })
    }
}

// ---------- 常驻服务（residents）的假适配器 ----------
/// 目的：常驻服务的假适配器——不拉起任何进程，记录调用并按固定脚本提供操作。
pub(crate) struct FakeServiceAdapter {
    pub calls: Arc<Mutex<Vec<String>>>,
    pub(crate) envs: Mutex<Vec<(String, String)>>,
    pub(crate) paths: Mutex<Vec<Option<std::ffi::OsString>>>,
    pub(crate) roots: Mutex<Vec<Vec<std::path::PathBuf>>>,
}
