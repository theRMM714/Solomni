//! 目的：一次模型调用的通道参数与思考档——流式与预算取值、思考片段、树上可用思考档与按根设置。
//! 管：思考档视图、`set_thinking`、`set_context_freshness`；`llm_opts` / `llm_opts_for` / 思考片段转调 turn（判据只有一处）。
//! 不管：通道本身（llm 能力）、会话配置落盘（`config.rs`）。
//! 联动：api 层的设置接口改思考档；调用参数与会话档位的判据在 `turn::api`。
//!   本方法族只碰 registry / history / running / sessions 这四个字段。

use super::*;
use crate::units::conversation::session::api::{subtree_of, work_root};

impl Work {
    /// 目的：这条工作里思考档位要作用到哪些模型（每个 agent 的模型；没有 agent 的会话用核心默认）。
    /// 约束：去重；核心内部编排不跟会话档，因此在有 agent 名单的工作里不额外塞核心模型。
    pub(crate) fn thinking_models_in_tree(
        &self,
        registry: &dyn Registry,
        root: &str,
    ) -> Vec<String> {
        let core = registry.core_model();
        let mut out: Vec<String> = Vec::new();
        let push = |m: String, out: &mut Vec<String>| {
            if !m.is_empty() && !out.contains(&m) {
                out.push(m);
            }
        };
        for sid in subtree_of(self.history.as_ref(), root) {
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
    pub(crate) fn thinking_view_for_root(
        &self,
        registry: &dyn Registry,
        root: &str,
    ) -> ThinkingView {
        let level = self
            .history
            .meta(root)
            .ok()
            .and_then(|m| m.thinking.clone());
        let catalog = &registry.app().thinking.levels;
        let models = self.thinking_models_in_tree(registry, root);
        let views = registry.model_views();
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
                    .all(|m| registry.thinking_supports(Some(m), &l.id))
                {
                    options.push(OptionView {
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
                .all(|m| registry.thinking_supports(Some(m), l)),
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
    pub(crate) fn set_thinking(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        sid: &str,
        level: &str,
    ) -> Result<(), String> {
        let level = level.trim();
        let root = work_root(self.history.as_ref(), sid)?;
        for s in subtree_of(self.history.as_ref(), &root) {
            if turn.is_running(&s) {
                return Err(Turn::running_refusal(&s));
            }
        }
        if !level.is_empty() {
            let catalog = &registry.app().thinking.levels;
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
                .thinking_models_in_tree(registry, &root)
                .into_iter()
                .filter(|m| !registry.thinking_supports(Some(m), level))
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
        self.record_config(turn, registry, &root, &meta);
        // 内存里的会话按旧参数装着：整棵丢掉，下一次访问按新档位从转录重建。
        for s in subtree_of(self.history.as_ref(), &root) {
            turn.take(&s);
        }
        Ok(())
    }

    /// 目的：切换这条工作的**过期内容替换**开关与保底：写在工作根 meta 上，整棵子树共享。
    /// 约束：整棵 work tree 有会话在生成时拒绝（发送视图在回合内不换）；切换后丢掉内存里的会话，
    ///   下一次访问按新开关从转录重建（开关是落盘事实，回放与实时必须同一条请求）。
    ///   保底 percent 缺省沿用现值；只接受 0..=90（100% 等于永不替换，开关会形同关闭）。
    pub(crate) fn set_context_freshness(
        &mut self,
        turn: &mut Turn,
        registry: &dyn Registry,
        sid: &str,
        on: bool,
        keep_percent: Option<u8>,
    ) -> Result<(), String> {
        let root = work_root(self.history.as_ref(), sid)?;
        for s in subtree_of(self.history.as_ref(), &root) {
            if turn.is_running(&s) {
                return Err(Turn::running_refusal(&s));
            }
        }
        let mut meta = self.history.meta(&root)?;
        let keep = keep_percent.unwrap_or(meta.freshness_keep_percent);
        if keep > 90 {
            return Err("过期内容保底最多 90%（100% 等于永不替换）".to_string());
        }
        if meta.context_freshness == on && meta.freshness_keep_percent == keep {
            return Ok(());
        }
        meta.context_freshness = on;
        meta.freshness_keep_percent = keep;
        self.history.save_meta(&meta)?;
        self.record_config(turn, registry, &root, &meta);
        for s in subtree_of(self.history.as_ref(), &root) {
            turn.take(&s);
        }
        Ok(())
    }
}
