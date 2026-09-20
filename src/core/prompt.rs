//! 提示词渲染层：{{key}} 占位替换，纯逻辑。
//! 册子文本来自 PromptSource（文件机制在适配层）；缺键/缺变量报错，不静默。
//! 提示词是最不稳定的文本：改文案只动 prompts.yaml，不改代码。

use serde::Deserialize;

/// 渲染时的变量表。
pub type Vars<'a> = &'a [(&'a str, String)];

/// 渲染单段提示词：替换 {{name}}；遇到未提供的变量 = 错误（如实，不静默）。
pub fn render(template: &str, vars: Vars) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end_rel) = template[i + 2..].find("}}") {
                let key = template[i + 2..i + 2 + end_rel].trim();
                let value = vars
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| format!("提示词变量缺失：{}", key))?;
                out.push_str(&value);
                i += 2 + end_rel + 2;
                continue;
            }
        }
        // 字面 {{（非占位）按原样保留；JSON 示例中的花括号不受影响（单括号）。
        let ch_len = template[i..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
        out.push_str(&template[i..i + ch_len]);
        i += ch_len;
    }
    Ok(out)
}

/// 提示词册（prompts.yaml 的内存形态）。
#[derive(Debug, Clone, Deserialize)]
pub struct Prompts {
    pub core: CorePrompts,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CorePrompts {
    pub chat_protocol: String,
    pub discuss: DiscussPrompts,
    pub synthesize: SynthPrompts,
    pub execute: ExecutePrompts,
    pub review: ReviewPrompts,
    pub rerun: RerunPrompts,
    pub slate: SlatePrompts,
    pub suggest_models: SuggestPrompts,
    /// 一个 agent 的职责提示词（由它的模块合成为一份能力包）。
    pub agent: AgentPrompts,
    /// @ 引用的两句说明文案。
    pub refs: RefsPrompts,
    /// 工具调用约定：手写信封（envelope 形态）。
    pub tool_calling_envelope: String,
    /// 工具调用约定：原生工具调用（native 形态）；与上一条**互斥**，一个通道只用一套。
    pub tool_calling_native: String,
    /// 工具与路径相关的**模型侧文案**（回执、失败说明、清单行）；改文案只改册子。
    pub tool_texts: ToolTexts,
    /// 内置文件工具说明块；变量：work_name, agent, work_root, sandbox_root, module_roots, tool_params, patch_guide
    pub sys_tools: String,
    /// patch 通道的写法说明（模型侧）；变量：work_root, sandbox_root
    pub patch_guide: String,
    /// 内置工具的参数契约（prompts.yaml 的 builtin_tools）：模型说明与调用校验的唯一来源。
    pub builtin_tools: crate::core::schema::ToolBook,
    /// 登记处还没有 agent 时的说明（拟名单的 {{agents}} 取值）。
    pub no_agents: String,
    /// agent 没有指定模型时的说明（拟名单清单里用）。
    pub no_model: String,
    /// 无成员模块目录时的说明（module_dirs 的取值）。
    pub no_module_dirs: String,
    /// 模块未声明外部工具时的说明（module_tools 的取值）。
    pub no_module_tools: String,
    /// 模块工具参数段的小标题（module_tool_params 的取值）。
    pub module_tool_params_header: String,
    /// 模块没有声明任何工具参数时的说明（module_tool_params 的取值）。
    pub no_module_tool_params: String,
}

/// 工具与路径的模型侧文案：核心拼回执、失败说明与清单行时从这里取。
/// 随沙箱/工具环境注入 core 的纯逻辑（与 RefsPrompts 同一套做法），本身不是状态。
#[derive(Debug, Clone, Deserialize)]
pub struct ToolTexts {
    // —— 路径校验（workspace::resolve）——
    pub path_empty: String,
    /// 变量：path
    pub path_need_absolute: String,
    /// 变量：path
    pub path_empty_segment: String,
    /// 变量：path
    pub path_cur_dir: String,
    /// 变量：path
    pub path_parent_dir: String,
    /// 变量：path
    pub path_outside_roots: String,
    /// 变量：why, roots
    pub roots_wrapper: String,
    /// 变量：root
    pub roots_shared: String,
    /// 变量：root
    pub roots_private: String,
    /// 变量：id, root
    pub roots_module: String,
    // —— 内置工具回执（systool）——
    /// 变量：error
    pub bad_args_json: String,
    // 参数不符：why 说事实、signature 是工具签名（都由 builtin_tools 的声明产生）
    /// 变量：why, signature
    pub arg_fault: String,
    pub arg_not_object: String,
    /// 变量：name
    pub arg_missing: String,
    /// 变量：name, want
    pub arg_wrong_type: String,
    /// 变量：name
    pub arg_empty: String,
    /// 变量：name, min
    pub arg_too_small: String,
    /// 变量：name, max
    pub arg_too_big: String,
    /// 变量：name
    pub arg_unknown: String,
    /// 变量：name
    pub unknown_builtin: String,
    /// 变量：path, bytes, text
    pub read_header: String,
    /// 变量：path, chars
    pub write_header: String,
    // —— edit：精确替换与"没找到/多处命中"的如实回报（第二层）——
    /// 变量：path, n, lines
    pub edit_header: String,
    pub edit_same: String,
    /// 变量：total
    pub edit_no_match: String,
    /// 变量：line, actual
    pub edit_no_match_near: String,
    /// 变量：n, lines
    pub edit_multi: String,
    /// 变量：path
    pub edit_file_cut: String,
    /// 变量：path
    pub edit_file_lossy: String,
    // —— 改动前的"读过"证据：不满足就拒绝并给出改法 ——
    // —— patch：自由格式补丁通道的解析与应用回执 ——
    /// 变量：blocks, files, lines
    pub patch_header: String,
    /// 变量：path, chars
    pub patch_block_add: String,
    /// 变量：path, chars
    pub patch_block_overwrite: String,
    /// 变量：path, n
    pub patch_block_update: String,
    /// 变量：n, why
    pub patch_block_fault: String,
    /// 变量：n, k, path, why
    pub patch_edit_fault: String,
    /// 变量：n, error
    pub patch_write_failed: String,
    /// 变量：total
    pub patch_not_found: String,
    /// 变量：n, lines
    pub patch_multiple: String,
    /// 变量：line, actual
    pub patch_near: String,
    /// 变量：path
    pub patch_file_missing: String,
    // patch 的解析失败（每一类各说各的事实）
    pub patch_no_blocks: String,
    /// 变量：line, text
    pub patch_unknown_marker: String,
    /// 变量：line, marker
    pub patch_missing_path: String,
    /// 变量：line
    pub patch_empty_add: String,
    /// 变量：line
    pub patch_empty_update: String,
    /// 变量：line
    pub patch_empty_search: String,
    /// 变量：line
    pub patch_missing_end: String,
    /// 变量：line
    pub patch_search_no_replace: String,
    /// 变量：line
    pub patch_replace_no_search: String,
    /// 变量：path
    pub write_need_read: String,
    /// 变量：path
    pub write_stale: String,
    /// 变量：path, why
    pub write_partial: String,
    /// 变量：from, to, total
    pub read_partial_range: String,
    pub read_partial_cut: String,
    pub read_partial_lossy: String,
    pub read_partial_none: String,
    /// 变量：mark, id
    pub write_module_note: String,
    /// 变量：path, keyword, mode
    pub search_header: String,
    pub search_mode_sensitive: String,
    pub search_mode_insensitive: String,
    pub search_no_hits: String,
    /// 变量：hits, total
    pub search_summary: String,
    /// 变量：n, line
    pub search_hit_line: String,
    /// 变量：limit
    pub search_truncated: String,
    /// 变量：n, line
    pub read_line: String,
    /// 变量：limit（拼在行尾）
    pub read_line_capped: String,
    /// 变量：from, to, total, next
    pub read_more: String,
    /// 变量：from, to
    pub read_more_cut: String,
    /// 变量：total
    pub read_end: String,
    /// 变量：total
    pub read_past_end: String,
    pub search_truncated_bytes: String,
    pub lossy_note: String,
    // —— 外部工具分派（engine）——
    /// 变量：tool
    pub no_module_field: String,
    /// 变量：module
    pub unknown_module: String,
    /// 变量：module, tool
    pub module_lacks_tool: String,
    /// 变量：module, capability
    pub module_unavailable: String,
    /// 变量：why, tools
    pub available_wrapper: String,
    // —— 工具循环与讨论（engine）——
    /// 变量：label, output
    pub tool_result_wrapper: String,
    /// 变量：n
    pub tool_cap: String,
    // 工具信封不合法：按判定出的类别给各自改法
    /// 变量：what（修好并执行时如实标注在工具回执最前面）
    pub envelope_repaired: String,
    /// 变量：missing（还差哪些收尾字符）
    pub malformed_unclosed_brace: String,
    /// 原生通道下"正文写了信封"的如实回报
    pub native_no_envelope: String,
    /// 变量：n, missing（一段回复里起了多段信封）
    pub malformed_multi: String,
    pub malformed_unclosed_string: String,
    /// 变量：missing（与其它类别叠加时的附带说明）
    pub malformed_missing_tail: String,
    pub malformed_cut_string: String,
    /// 供应商说是长度截断时追加（改法完全不同：分次写，而不是查 JSON）
    pub malformed_truncated: String,
    /// 变量：what, line
    pub malformed_control: String,
    /// 变量：why
    pub malformed_syntax: String,
    /// 变量：why
    pub malformed_shape: String,
    pub control_lf: String,
    pub control_cr: String,
    pub control_tab: String,
    /// 变量：code
    pub control_other: String,
    pub discuss_degraded: String,
    /// 追加在回复行末尾（该行重建后进上下文）
    pub stopped_suffix: String,
    /// 追加在回复行末尾（该行被供应商按长度截断）
    pub truncated_suffix: String,
    // —— 工具进程的收尾标记（机制侧拼进工具回执；它们随 [工具结果] 进模型上下文）——
    /// 工具进程 stderr 段的头。
    pub tool_stderr_header: String,
    /// 超时被杀（连同它拉起的整棵进程树）。
    pub tool_timeout: String,
    /// 围栏没装上（命令未执行）。
    pub tool_fence_failed: String,
    /// 变量：chars, limit
    pub tool_truncated: String,
    // —— 拟名单/推荐时给模型看的清单行 ——
    /// 变量：id, brief
    pub module_listing_line: String,
    /// 变量：id, tools
    pub module_tools_line: String,
    /// 变量：module, tool, signature
    pub module_tool_params_line: String,
    /// 变量：id, root
    pub module_root_line: String,
    /// 变量：name, modules, model, note
    pub agent_listing_line: String,
    /// 变量：id, name, api_model, note
    pub model_listing_line: String,
    /// 工具/模块清单里的分隔符（模型看到的清单行用它拼接）。
    pub tool_list_separator: String,
}

impl ToolTexts {
    /// 渲染一条模型侧文案。缺变量 = 装配错误，直接暴露（禁止静默兜底）。
    pub fn render(&self, template: &str, vars: Vars) -> String {
        render(template, vars).expect("工具文案变量由调用方保证（缺变量属于装配错误）")
    }

    /// 未闭合的修法：内容写完只是少了收尾括号 → 直接说还差什么；断在字符串中间 → 才谈"分次写"。
    fn unclosed_report(&self, tail: &crate::core::envelope::Tail) -> String {
        // 一段回复里起了两段信封：这不是"补个括号"能救的（末尾补括号补不到中间那段），
        // 而且补哪一段都是猜——如实说清，让模型只发一段。
        if tail.envelopes > 1 {
            return self.render(
                &self.malformed_multi,
                &[
                    ("n", tail.envelopes.to_string()),
                    ("missing", tail.missing.clone()),
                ],
            );
        }
        if tail.in_string {
            self.malformed_unclosed_string.clone()
        } else {
            self.render(
                &self.malformed_unclosed_brace,
                &[("missing", tail.missing.clone())],
            )
        }
    }

    /// 附带说明：信封还差什么（与其它类别叠加时用）。
    fn tail_note(&self, tail: &crate::core::envelope::Tail) -> String {
        if tail.in_string {
            self.malformed_cut_string.clone()
        } else {
            self.render(
                &self.malformed_missing_tail,
                &[("missing", tail.missing.clone())],
            )
        }
    }

    /// 工具信封不合法的回执：按**判定出的类别**给出对应修法（类别由 envelope 判定，文案在这里）。
    pub fn malformed_report(&self, kind: &crate::core::envelope::Malformed) -> String {
        match kind {
            crate::core::envelope::Malformed::Unclosed(tail) => self.unclosed_report(tail),
            crate::core::envelope::Malformed::RawControl { ch, line, tail } => {
                let what = match ch {
                    '\n' => self.control_lf.clone(),
                    '\r' => self.control_cr.clone(),
                    '\t' => self.control_tab.clone(),
                    other => self.render(
                        &self.control_other,
                        &[("code", format!("{:04X}", *other as u32))],
                    ),
                };
                let mut out = self.render(
                    &self.malformed_control,
                    &[("what", what), ("line", line.to_string())],
                );
                // 同时还没闭合就一并说清（只说一处会让模型改错方向）
                if let Some(t) = tail {
                    out.push('\n');
                    out.push_str(&self.tail_note(t));
                }
                out
            }
            crate::core::envelope::Malformed::Syntax(why) => {
                self.render(&self.malformed_syntax, &[("why", why.clone())])
            }
            crate::core::envelope::Malformed::Shape(why) => {
                self.render(&self.malformed_shape, &[("why", why.clone())])
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefsPrompts {
    /// 引用了别的 agent 的沙箱；变量：agent（两个模板都收到 agent 与 path，多余的变量被忽略）。
    pub foreign_sandbox: String,
    /// 协作里共读同一条引用；变量：path, agent。
    pub collab_sandbox: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscussPrompts {
    /// opener 变量：protocol, task
    pub opener: String,
    /// step 变量：transcript
    pub step: String,
    pub autonomy_note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SynthPrompts {
    pub system: String,
    /// user 变量：transcript
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutePrompts {
    /// user 变量：tasks
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewPrompts {
    pub system: String,
    /// user 变量：plan, reports
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RerunPrompts {
    /// user 变量：tasks, review, report
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlatePrompts {
    pub system: String,
    /// user 变量：agents, modules, models, task
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SuggestPrompts {
    pub system: String,
    /// user 变量：mode, agents, modules, models, task
    pub user: String,
    /// 形态描述（{{mode}} 的取值）：单 agent。
    pub mode_single: String,
    /// 形态描述（{{mode}} 的取值）：协作。
    pub mode_collab: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentPrompts {
    /// system 变量：agent, modules, sys_tools, module_tools, module_tool_params
    pub system: String,
}

impl Prompts {
    pub fn render<'a>(&self, template: &str, vars: Vars<'a>) -> String {
        render(template, vars)
            .expect("提示词渲染失败：变量缺失属于装配错误，须修复 prompts.yaml 或调用方")
    }
}
