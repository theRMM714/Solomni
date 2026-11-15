//! 目的：代理工具的执行面——按声明校验参数、判授权、查幂等账，再把动作交给 `ProxyHost` 宿主。
//! 管：六个代理工具的执行流程与幂等账；成员侧执行者 `ProxyHandler`（装进代理会话的那一回合）。
//! 不管：工具逻辑的纯规则（`domain/proxy.rs`）；会话机制本身（宿主实现把它落到核心线程）。
//! 联动：端口见 `ports.rs`；工具面名单与 callers 在 `systools/tools.yaml`；装配是 `conductor` 的队列桥旁。

use crate::capabilities::proxy::domain::proxy as d;
use crate::capabilities::proxy::ports::ProxyHost;
use crate::capabilities::tools::api::{ToolBook, ToolOutcome, arg_fault_text};
use std::collections::BTreeMap;
use std::sync::Arc;

/// 目的：代理工具的执行器：持有宿主端口、工具声明书、回执文案与幂等账本。
pub struct ProxyTools {
    host: Arc<dyn ProxyHost + Send + Sync>,
    book: ToolBook,
    /// 幂等账本：工具/request_id → 上一次的结果（重放直接回同一结果，不再动宿主）。
    done: BTreeMap<String, ToolOutcome>,
}

impl ProxyTools {
    pub fn new(host: Arc<dyn ProxyHost + Send + Sync>, book: ToolBook) -> ProxyTools {
        ProxyTools {
            host,
            book,
            done: BTreeMap::new(),
        }
    }

    /// 目的：执行一次代理工具调用：name 必须是六个代理工具之一；args_json 是模型给的参数。
    ///  参数先按总表声明校验（形状只有一份真相），再做语义校验、授权与宿主调用。
    pub fn call(&mut self, ctx: &d::ProxyCall, name: &str, args_json: &str) -> ToolOutcome {
        if !d::is_proxy_tool(name) {
            return deny(format!("不是代理工具：{}", name));
        }
        let Some(schema) = self.book.get(name) else {
            return deny(format!("工具总表里没有这个工具：{}", name));
        };
        let value: serde_json::Value = match serde_json::from_str(args_json) {
            Ok(v) => v,
            Err(e) => return deny(format!("{} 的参数不是合法 JSON：{}", name, e)),
        };
        if let Err(fault) = schema.check(&value) {
            return deny(arg_fault_text(name, schema, &fault));
        }
        let mut value = value;
        schema.apply_defaults(&mut value);
        if name == d::CATALOG {
            self.catalog(ctx, &value)
        } else if name == d::CREATE {
            self.create(ctx, &value)
        } else if name == d::SEND {
            self.send(ctx, &value)
        } else if name == d::OBSERVE {
            self.observe(ctx, &value)
        } else if name == d::MESSAGES {
            self.messages(ctx, &value)
        } else {
            self.control(ctx, &value)
        }
    }

    fn catalog(&self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::CatalogArgs = match parse_arg(d::CATALOG, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        let scope = match d::catalog_scope(&args) {
            Ok(s) => s,
            Err(e) => return deny(e),
        };
        if let Err(e) = d::authorize(ctx, d::CATALOG, None, None) {
            return deny(e);
        }
        match self.host.catalog(scope) {
            Ok(c) => match d::render_catalog(scope, &c) {
                Ok(text) => ok(text),
                Err(e) => deny(e),
            },
            Err(e) => deny(e),
        }
    }

    fn create(&mut self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::CreateArgs = match parse_arg(d::CREATE, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        if args.request_id.trim().is_empty() {
            return deny("request_id 不能为空（幂等标识）".to_string());
        }
        let key = idem_key(d::CREATE, &args.request_id);
        if let Some(prev) = self.done.get(&key) {
            return prev.clone();
        }
        if let Err(e) = d::authorize(ctx, d::CREATE, None, None) {
            return deny(e);
        }
        // 校验要对着登记处与模块清单做：核不过就整条拒绝，**不调用 create**（不留半成品）。
        let catalog = match self.host.catalog(d::CatalogScope::All) {
            Ok(c) => c,
            Err(e) => return deny(e),
        };
        let mut spec = match d::resolve_new_session(&args, &catalog) {
            Ok(s) => s,
            Err(e) => return deny(e),
        };
        // 父会话由机制从调用上下文填，不接受模型自参。
        spec.parent = ctx.parent.clone();
        for a in &spec.agents {
            if let Some(m) = &a.model
                && let Err(e) = d::authorize(ctx, d::CREATE, Some(m), None)
            {
                return deny(e);
            }
        }
        match self.host.create_session(&spec) {
            Ok(created) => {
                let out = json_ok(&created);
                self.done.insert(key, out.clone());
                out
            }
            // 创建失败**不记账**：重放会再试一次，而不是把失败当成已完成。
            Err(e) => deny(e),
        }
    }

    fn send(&mut self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::SendArgs = match parse_arg(d::SEND, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        if args.request_id.trim().is_empty() {
            return deny("request_id 不能为空（幂等标识）".to_string());
        }
        let key = idem_key(d::SEND, &args.request_id);
        if let Some(prev) = self.done.get(&key) {
            return prev.clone();
        }
        if let Err(e) = d::authorize(ctx, d::SEND, None, None) {
            return deny(e);
        }
        let (targets, relayed) = match d::relay(&args, ctx) {
            Ok(x) => x,
            Err(e) => return deny(e),
        };
        for t in &targets {
            if let Err(e) = d::authorize(ctx, d::SEND, None, Some(t)) {
                return deny(e);
            }
        }
        // 多目标：逐个记录成功与失败，部分成功照实回报，不产生未记录的隐式发送。
        let mut sent: Vec<String> = Vec::new();
        let mut failed: Vec<serde_json::Value> = Vec::new();
        for t in &targets {
            match self.host.send(t, &relayed) {
                Ok(()) => sent.push(t.clone()),
                Err(e) => failed.push(serde_json::json!({ "target": t, "why": e })),
            }
        }
        let out = ToolOutcome {
            ok: failed.is_empty(),
            output: serde_json::json!({
                "sent": sent,
                "failed": failed,
            })
            .to_string(),
        };
        self.done.insert(key, out.clone());
        out
    }

    fn observe(&self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::ObserveArgs = match parse_arg(d::OBSERVE, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        let (session, view, since) = match d::observe_request(&args) {
            Ok(x) => x,
            Err(e) => return deny(e),
        };
        if let Err(e) = d::authorize(ctx, d::OBSERVE, None, Some(&session)) {
            return deny(e);
        }
        match self.host.observe(&session, view, since.as_deref()) {
            Ok(s) => json_ok(&s),
            Err(e) => deny(e),
        }
    }

    /// 消息倒查：只读，不改任何状态，也不进幂等账。
    fn messages(&self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::MessagesArgs = match parse_arg(d::MESSAGES, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        let (session, from, count) = match d::messages_request(&args) {
            Ok(x) => x,
            Err(e) => return deny(e),
        };
        if let Err(e) = d::authorize(ctx, d::MESSAGES, None, Some(&session)) {
            return deny(e);
        }
        match self.host.messages(&session, from, count) {
            Ok(page) => json_ok(&page),
            Err(e) => deny(e),
        }
    }

    fn control(&mut self, ctx: &d::ProxyCall, value: &serde_json::Value) -> ToolOutcome {
        let args: d::ControlArgs = match parse_arg(d::CONTROL, value) {
            Ok(a) => a,
            Err(e) => return deny(e),
        };
        let (session, action, reason, request_id) = match d::control_request(&args) {
            Ok(x) => x,
            Err(e) => return deny(e),
        };
        let key = idem_key(d::CONTROL, &request_id);
        if let Some(prev) = self.done.get(&key) {
            return prev.clone();
        }
        if let Err(e) = d::authorize(ctx, d::CONTROL, None, Some(&session)) {
            return deny(e);
        }
        match self.host.control(&session, action, &reason) {
            Ok(state) => {
                let out = json_ok(&state);
                self.done.insert(key, out.clone());
                out
            }
            Err(e) => deny(e),
        }
    }
}

fn parse_arg<T: serde::de::DeserializeOwned>(
    name: &str,
    value: &serde_json::Value,
) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|e| format!("{} 的参数形状不对：{}", name, e))
}

fn idem_key(tool: &str, request_id: &str) -> String {
    format!("{}/{}", tool, request_id.trim())
}

fn ok(text: String) -> ToolOutcome {
    ToolOutcome {
        ok: true,
        output: text,
    }
}

fn deny(text: String) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: text,
    }
}

fn json_ok<T: serde::Serialize>(v: &T) -> ToolOutcome {
    match serde_json::to_string(v) {
        Ok(s) => ok(s),
        Err(e) => deny(format!("回执序列化失败：{}", e)),
    }
}

/// 目的：代理工具的**成员侧执行者**：把 `core_proxy` 会话那一回合里的代理工具调用接到 `ProxyTools` 上。
///  装配进代理会话的 `MemberTools.handlers` 之后，代理会话就是一个**普通成员会话**——
///  `converse_with` / 转录 / 回档 / 压缩 / 停止全部复用，循环里没有任何“代理专用”分支。
///  执行经队列桥回核心线程（`ProxyHost`），核心状态的所有权不变。
pub struct ProxyHandler {
    tools: std::sync::Mutex<ProxyTools>,
    ctx: d::ProxyCall,
}

impl ProxyHandler {
    pub fn new(
        host: Arc<dyn ProxyHost + Send + Sync>,
        book: ToolBook,
        ctx: d::ProxyCall,
    ) -> ProxyHandler {
        ProxyHandler {
            tools: std::sync::Mutex::new(ProxyTools::new(host, book)),
            ctx,
        }
    }
}

impl crate::kernel::ports::ToolHandler for ProxyHandler {
    fn owns(&self, name: &str) -> bool {
        d::is_proxy_tool(name)
    }

    fn run(
        &self,
        _ctx: &crate::kernel::ports::ToolCtx,
        name: &str,
        args_json: &str,
    ) -> ToolOutcome {
        let mut tools = self.tools.lock().unwrap_or_else(|e| e.into_inner());
        tools.call(&self.ctx, name, args_json)
    }
}
