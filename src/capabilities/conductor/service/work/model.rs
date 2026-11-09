//! 目的：一次模型调用的通道参数与思考档——流式与预算取值、思考片段、树上可用思考档与按根设置。
//! 管：`llm_opts` / `llm_opts_for`、思考片段与思考档视图、`set_thinking`、`set_context_freshness`。
//! 不管：通道本身（llm 能力）、会话配置落盘（`config.rs`）。
//! 联动：turn / flow 每次调用模型前取参数；api 层的设置接口改思考档。
//!   本方法族只碰 registry / history / running / sessions 这四个字段。

use super::*;

impl Conductor {
    /// 目的：本次模型调用的通道参数：**预算与"能不能流式"都取全局设置**（讨论、执行、验收、单 agent 共用一份）。
    ///  `want_stream` 是调用方这一次的意愿（呈现层按回包形状给）：**设置是上限，调用方可以在本次放弃流式**；
    ///  设置关掉时一律非流式。两处各判一次迟早会打架，所以判据只在这里。
    pub(crate) fn llm_opts(&self, want_stream: bool) -> crate::capabilities::llm::api::LlmOpts {
        let app = self.registry.app();
        crate::capabilities::llm::api::LlmOpts {
            stream: app.streaming && want_stream,
            timeout_secs: app.llm_timeout_secs,
            thinking: None,
        }
    }

    /// 目的：这条会话这一次生成要用的调用参数：全局流式/预算 + **按模型解析的思考片段**。
    /// 约束：只用于对话生成（单 agent / 代理 / 协作成员会话）；核心内部编排不走它。
    pub(crate) fn llm_opts_for(
        &self,
        sid: &str,
        want_stream: bool,
    ) -> Result<crate::capabilities::llm::api::LlmOpts, String> {
        let mut opts = self.llm_opts(want_stream);
        opts.thinking = self.thinking_fragment_for(sid)?;
        Ok(opts)
    }

    /// 目的：这条会话的模型在当前工作档位下要并入请求体的思考片段（None = 不发）。
    /// 约束：档位取**工作根** meta 的 `thinking`；模型取这条会话第一个 agent 的模型，没有 agent（代理）就用核心默认；
    ///  模型没登记该档 = 如实报错（不静默降级成供应商默认）。
    pub(crate) fn thinking_fragment_for(
        &self,
        sid: &str,
    ) -> Result<Option<std::sync::Arc<serde_json::Value>>, String> {
        let root = self.work_root(sid)?;
        let level = self.history.meta(&root)?.thinking;
        let model = self
            .history
            .meta(sid)
            .ok()
            .and_then(|m| m.agents.first().and_then(|a| a.model.clone()));
        Ok(self
            .registry
            .thinking_fragment(model.as_deref(), level.as_deref())?
            .map(std::sync::Arc::new))
    }

    /// 目的：这条工作里思考档位要作用到哪些模型（每个 agent 的模型；没有 agent 的会话用核心默认）。
    /// 约束：去重；核心内部编排不跟会话档，因此在有 agent 名单的工作里不额外塞核心模型。
    pub(crate) fn thinking_models_in_tree(&self, root: &str) -> Vec<String> {
        let core = self.registry.core_model();
        let mut out: Vec<String> = Vec::new();
        let push = |m: String, out: &mut Vec<String>| {
            if !m.is_empty() && !out.contains(&m) {
                out.push(m);
            }
        };
        for sid in self.subtree_of(root) {
            let Ok(meta) = self.history.meta(&sid) else {
                continue;
            };
            if meta.agents.is_empty() {
                if let Some(c) = &core {
                    push(c.clone(), &mut out);
                }
            } else {
                for a in &meta.agents {
                    if let Some(id) = a.model.clone().or_else(|| core.clone()) {
                        push(id, &mut out);
                    }
                }
            }
        }
        out
    }

    /// 目的：一条工作的思考强度视图：当前选择 + 这组模型共同的默认档 + 只列已映射的档。
    /// 约束：可选 = 所有相关模型都登记了该档；没有统一默认时由前端保留一个「跟随默认」状态项。
    pub(crate) fn thinking_view_for_root(&self, root: &str) -> ThinkingView {
        let level = self
            .history
            .meta(root)
            .ok()
            .and_then(|m| m.thinking.clone());
        let catalog = &self.registry.app().thinking.levels;
        let models = self.thinking_models_in_tree(root);
        let views = self.registry.model_views();
        let defaults: Vec<Option<String>> = models
            .iter()
            .map(|m| {
                views
                    .iter()
                    .find(|v| v.id.as_str() == m.as_str())
                    .and_then(|v| v.thinking.default.clone())
            })
            .collect();
        // 共同默认：所有模型都设了同一个默认档才成立；否则没有可替用户选中的单一档。
        let default_level = defaults.first().cloned().flatten().filter(|first| {
            defaults
                .iter()
                .all(|d| d.as_deref() == Some(first.as_str()))
        });
        let mut options = Vec::new();
        if !models.is_empty() {
            for l in catalog {
                if models
                    .iter()
                    .all(|m| self.registry.thinking_supports(Some(m), &l.id))
                {
                    options.push(ThinkingOptionView {
                        id: l.id.clone(),
                        label: l.id.clone(),
                    });
                }
            }
        }
        let label = match level.as_deref().or(default_level.as_deref()) {
            Some(l) => l.to_string(),
            None if defaults.iter().any(|d| d.is_some()) => "各模型默认".to_string(),
            None => "未设置".to_string(),
        };
        let current_ok = match &level {
            Some(l) => models
                .iter()
                .all(|m| self.registry.thinking_supports(Some(m), l)),
            None => true,
        };
        let note = if models.is_empty() {
            "登记处还没有可用模型：先登记供应商与模型".to_string()
        } else if !current_ok {
            "当前档位有模型没登记：下一次生成会如实报错；改模型登记或在状态栏换一档".to_string()
        } else if options.is_empty() {
            "这组模型没有共同登记的思考档：先到「模型登记」补映射".to_string()
        } else if level.is_none() && default_level.is_none() && defaults.iter().any(|d| d.is_some())
        {
            "各模型的默认思考档不同：切换后整棵工作树共用一个档".to_string()
        } else {
            String::new()
        };
        ThinkingView {
            level,
            default_level,
            label,
            options,
            note,
        }
    }

    /// 目的：切换这条工作的思考强度：写在工作根 meta 上，整棵子树共享。
    /// 约束：整棵 work tree 有会话在生成时拒绝；档位不在目录里、或任一相关模型没登记该档 = 拒绝并列出原因。
    pub(crate) fn set_thinking(&mut self, sid: &str, level: &str) -> Result<(), String> {
        let level = level.trim();
        let root = self.work_root(sid)?;
        for s in self.subtree_of(&root) {
            if self.running.contains(&s) {
                return Err(Self::running_refusal(&s));
            }
        }
        if !level.is_empty() {
            let catalog = &self.registry.app().thinking.levels;
            if !catalog.iter().any(|l| l.id == level) {
                return Err(format!(
                    "思考档位 {} 不在全局目录里（现有：{}）",
                    level,
                    catalog
                        .iter()
                        .map(|l| l.id.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ));
            }
            let missing: Vec<String> = self
                .thinking_models_in_tree(&root)
                .into_iter()
                .filter(|m| !self.registry.thinking_supports(Some(m), level))
                .collect();
            if !missing.is_empty() {
                return Err(format!(
                    "这些模型没有登记思考档位 {}：{}；先到「模型登记」补映射",
                    level,
                    missing.join("、")
                ));
            }
        }
        let mut meta = self.history.meta(&root)?;
        meta.thinking = if level.is_empty() {
            None
        } else {
            Some(level.to_string())
        };
        self.history.save_meta(&meta)?;
        self.record_config(&root, &meta);
        // 内存里的会话按旧参数装着：整棵丢掉，下一次访问按新档位从转录重建。
        for s in self.subtree_of(&root) {
            self.sessions.remove(&s);
        }
        Ok(())
    }

    /// 目的：切换这条工作的**过期内容替换**开关：写在工作根 meta 上，整棵子树共享。
    /// 约束：整棵 work tree 有会话在生成时拒绝（发送视图在回合内不换）；切换后丢掉内存里的会话，
    ///   下一次访问按新开关从转录重建（开关是落盘事实，回放与实时必须同一条请求）。
    pub(crate) fn set_context_freshness(&mut self, sid: &str, on: bool) -> Result<(), String> {
        let root = self.work_root(sid)?;
        for s in self.subtree_of(&root) {
            if self.running.contains(&s) {
                return Err(Self::running_refusal(&s));
            }
        }
        let mut meta = self.history.meta(&root)?;
        if meta.context_freshness == on {
            return Ok(());
        }
        meta.context_freshness = on;
        self.history.save_meta(&meta)?;
        self.record_config(&root, &meta);
        for s in self.subtree_of(&root) {
            self.sessions.remove(&s);
        }
        Ok(())
    }
}
