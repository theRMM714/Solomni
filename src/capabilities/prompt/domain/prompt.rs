//! 提示词渲染层：{{key}} 占位替换，纯逻辑。
//! 册子文本来自 PromptSource（文件机制在适配层）；缺键/缺变量报错，不静默。
//! 提示词是最不稳定的文本：改文案只动 prompts/，不改代码。

use serde::de::DeserializeOwned;
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

/// 装配输入的**内存形态**：**只有提示词文本**（`prompts/`）。
/// 工具总表与角色表是**另一个能力的东西**（`systools/`，见 `capabilities/tools/`）：
/// 挂进这里就等于让提示词能力反过来依赖工具能力，两边成环。
///
/// **持有者只有本能力**（`service.rs` 一处，R4）。两块"被到处要的记录"（工具文案、`@` 文案）
/// 以 `Arc` 共享出去：留在册子里再逐处克隆，等于凭空多出几份深拷贝。
#[derive(Debug, Clone)]
pub struct Prompts {
    /// 按名字取的那些段（工具文案与 `@` 文案单独拎出来了）。**布局只有本能力知道**。
    pub(crate) core: CoreTexts,
    /// 工具与路径的模型侧文案（~100 条模板）：最常被 tools / workspace / collab 要。
    pub(crate) tools: std::sync::Arc<ToolTexts>,
    /// `@` 引用的两句说明文案。
    pub(crate) refs: std::sync::Arc<RefsPrompts>,
}

/// **一段核心提示词的名字**：别的能力按名字取段，不点册子内部结构。
///
/// 加/改一段提示词的步骤因此固定成两步：`prompts/` 里加键 → 这里加一个变体（缺了编译不过）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    /// 机制说明·单 agent 工作（这个系统怎么运转）。
    MechanismSingle,
    /// 机制说明·协作（讨论 → 任务链 → 节点执行 → 验收）。
    MechanismCollab,
    /// 机制说明·代理（全权、共用工作区、派完让出回合、停下叫醒）。
    MechanismProxy,
    ChatProtocol,
    DiscussOpener,
    DiscussStep,
    DiscussAutonomyNote,
    SynthesizeSystem,
    SynthesizeUser,
    ExecuteUser,
    ReviewSystem,
    ReviewUser,
    NodeReviewSystem,
    NodeReviewUser,
    SlateSystem,
    SlateUser,
    SlateModeSingle,
    SlateModeCollab,
    VerdictSystem,
    VerdictUser,
    /// 核心代理（core_proxy）的身份提示词：在用户授予的任务级授权范围内代用户决定与转达。
    ProxySystem,
    AgentSystem,
    ToolCallingEnvelope,
    ToolCallingNative,
    Env,
    PatchGuide,
    NoAgents,
    NoModel,
    NoModuleDirs,
    NoModuleTools,
    ModuleToolParamsHeader,
    NoModuleToolParams,
}

/// 把册子的多个文件合并成内存形态：各文件的**顶层键**合并后就是 `core:` 的内容。
///
/// **别的能力不按字段路径读册子**：它们要么按名字取一段
/// （`Prompt::text` / `Prompt::render`），要么拿走 `tools()` / `refs()` 那两块**共享记录**。
/// 所以"文件怎么分"与"某一段落在结构体的哪一格"都只有本能力知道——改版式不再牵动别的能力。
///
/// 三条如实报错（不静默）：**键在两个文件里重复**（拆分时最可能犯的错）、**缺键**、**类型不对**。
pub fn merge_book(docs: &[String]) -> Result<Prompts, String> {
    let mut merged = yaml_serde::Mapping::new();
    for doc in docs {
        let value: yaml_serde::Value =
            yaml_serde::from_str(doc).map_err(|e| format!("提示词册非法：{}", e))?;
        let map = match value {
            yaml_serde::Value::Mapping(m) => m,
            _ => return Err("提示词册非法：每个文件的最外层必须是一张键表".to_string()),
        };
        for (key, value) in map {
            if merged.insert(key.clone(), value).is_some() {
                return Err(format!("提示词册非法：键 {:?} 在两个文件里重复", key));
            }
        }
    }
    // 两块**被到处要的记录**单独拎出来共享（一个 Arc 走遍全进程，不再逐处深拷贝）。
    let tools = take_section::<ToolTexts>(&mut merged, "tool_texts")?;
    let refs = take_section::<RefsPrompts>(&mut merged, "refs")?;
    // 剩下的键就是"核心段"（`core:` 的内容）。
    let core: CoreTexts = yaml_serde::from_value(yaml_serde::Value::Mapping(merged))
        .map_err(|e| format!("提示词册缺键或类型不对：{}", e))?;
    Ok(Prompts {
        core,
        tools: std::sync::Arc::new(tools),
        refs: std::sync::Arc::new(refs),
    })
}

/// 从合并后的键表里取出一段反序列化（缺键 / 类型不对 = 装配错误，报出键名）。
fn take_section<T: DeserializeOwned>(
    merged: &mut yaml_serde::Mapping,
    key: &str,
) -> Result<T, String> {
    let value = merged
        .remove(yaml_serde::Value::String(key.to_string()))
        .ok_or_else(|| format!("提示词册缺键：{}", key))?;
    yaml_serde::from_value(value).map_err(|e| format!("提示词册的 {} 类型不对：{}", key, e))
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoreTexts {
    /// 机制说明**按会话类别各一份**：单 agent / 协作 / 代理。
    /// 每个身份块只拿自己那一份——AI 不知道机制，就只会写散文。
    pub mechanisms: MechanismTexts,
    pub chat_protocol: String,
    pub discuss: DiscussPrompts,
    pub synthesize: SynthPrompts,
    pub execute: ExecutePrompts,
    pub review: ReviewPrompts,
    /// 节点级验收（任务链：逐节点核对当前目标）。
    pub node_review: NodeReviewPrompts,
    pub slate: SlatePrompts,
    /// 判定用户对裁决的回应是否明确到可以开工/放行。
    pub verdict: VerdictPrompts,
    /// 核心代理（core_proxy）的身份提示词。
    pub proxy: ProxyPrompts,
    /// 一个 agent 的职责提示词（由它的模块合成为一份能力包）。
    pub agent: AgentPrompts,
    /// 工具调用约定：手写信封（envelope 形态）。
    pub tool_calling_envelope: String,
    /// 工具调用约定：原生工具调用（native 形态）；与上一条**互斥**，一个通道只用一套。
    pub tool_calling_native: String,
    /// **工作环境块**：真实根目录与路径规矩。变量：work_name, agent, work_root, sandbox_root, module_roots。
    /// 这里**不列工具**——能用哪些工具由核心按这一回合的身份从角色表现渲染、随回合注入。
    pub env: String,
    /// patch 通道的写法说明（模型侧）；变量：work_root, sandbox_root
    pub patch_guide: String,
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
/// 随沙箱/工具环境注入 conductor 的纯逻辑（与 RefsPrompts 同一套做法），本身不是状态。
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
    /// 变量：name
    pub tool_not_allowed: String,
    /// 本回合可用工具块的标题行；变量：tools（由核心按该回合的角色表现渲染）。
    pub tools_this_turn: String,
    /// 压缩回合的提示词（无变量）：让 AI 自己压，并调 compact 工具写下摘要。
    pub compact_prompt: String,
    /// 轮次边界给"上一轮没表态"的成员的提醒（无变量）。
    pub discuss_reminder: String,
    /// 变量：path, bytes, text
    pub read_header: String,
    /// read 遇到目录时的如实引导。变量：path
    pub read_is_dir: String,
    // —— list：列目录 ——
    /// 变量：path, count
    pub list_header: String,
    /// 变量：name, dir_mark, bytes
    pub list_row: String,
    pub list_dir_mark: String,
    pub list_empty: String,
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
pub struct VerdictPrompts {
    pub system: String,
    /// user 变量：kind, payload, text
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SynthPrompts {
    pub system: String,
    /// user 变量：transcript
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyPrompts {
    /// 核心代理的身份提示词（能用哪些工具由角色表按回合注入，不在这里列清单）。
    pub system: String,
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
pub struct NodeReviewPrompts {
    pub system: String,
    /// user 变量：nodes
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlatePrompts {
    pub system: String,
    /// user 变量：mode, agents, modules, models, task
    pub user: String,
    /// 形态描述（{{mode}} 的取值）：单 agent。
    pub mode_single: String,
    /// 形态描述（{{mode}} 的取值）：协作。
    pub mode_collab: String,
}

/// 三种会话类别的机制说明。每个身份块只取自己那一份
/// （会话类别 → 这一份的绑定见 `SessionParams::mechanisms`，按落盘形态派生）。
#[derive(Debug, Clone, Deserialize)]
pub struct MechanismTexts {
    pub single: String,
    pub collab: String,
    pub proxy: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentPrompts {
    /// system 变量：agent, modules, mechanism, env, tool_calling
    pub system: String,
}

impl CoreTexts {
    /// **按名字取一段原文**：册子的布局只在这里露面（新加一段 = 这里加一支 match）。
    pub fn segment(&self, seg: Segment) -> &str {
        match seg {
            Segment::MechanismSingle => &self.mechanisms.single,
            Segment::MechanismCollab => &self.mechanisms.collab,
            Segment::MechanismProxy => &self.mechanisms.proxy,
            Segment::ChatProtocol => &self.chat_protocol,
            Segment::DiscussOpener => &self.discuss.opener,
            Segment::DiscussStep => &self.discuss.step,
            Segment::DiscussAutonomyNote => &self.discuss.autonomy_note,
            Segment::SynthesizeSystem => &self.synthesize.system,
            Segment::SynthesizeUser => &self.synthesize.user,
            Segment::ExecuteUser => &self.execute.user,
            Segment::ReviewSystem => &self.review.system,
            Segment::ReviewUser => &self.review.user,
            Segment::NodeReviewSystem => &self.node_review.system,
            Segment::NodeReviewUser => &self.node_review.user,
            Segment::SlateSystem => &self.slate.system,
            Segment::SlateUser => &self.slate.user,
            Segment::SlateModeSingle => &self.slate.mode_single,
            Segment::SlateModeCollab => &self.slate.mode_collab,
            Segment::VerdictSystem => &self.verdict.system,
            Segment::VerdictUser => &self.verdict.user,
            Segment::ProxySystem => &self.proxy.system,
            Segment::AgentSystem => &self.agent.system,
            Segment::ToolCallingEnvelope => &self.tool_calling_envelope,
            Segment::ToolCallingNative => &self.tool_calling_native,
            Segment::Env => &self.env,
            Segment::PatchGuide => &self.patch_guide,
            Segment::NoAgents => &self.no_agents,
            Segment::NoModel => &self.no_model,
            Segment::NoModuleDirs => &self.no_module_dirs,
            Segment::NoModuleTools => &self.no_module_tools,
            Segment::ModuleToolParamsHeader => &self.module_tool_params_header,
            Segment::NoModuleToolParams => &self.no_module_tool_params,
        }
    }
}
