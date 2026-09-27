//! 会话的**成员工具面**：一个成员这一回合能调什么（内置工具 + 自己模块的外部工具）。
//!
//! 它是会话侧的数据（`MemberTools.observations` 是观察账本、`sandbox` 是它的根），
//! 所以本文件与它的方法都在会话能力里（`tools` 只管工具总表与角色发放，见 tools 能力）。

use crate::capabilities::tools::api::ToolExec;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

/// 一个模块的外部工具环境：模块目录（外部工具进程的 cwd）+ 它声明的工具表。
/// cwd 必须落在声明它的模块里（命令形如 python tools/x.py，是相对模块根写的）。
#[derive(Debug, Clone)]
pub struct ModuleTools {
    pub root: PathBuf,
    /// 工具名 → 启动命令（模块作者在 module.yaml 的 tools.<名字>.command 里声明）。
    pub commands: BTreeMap<String, String>,
    /// 工具名 → 参数契约（只含**声明了** params 的工具；没声明的工具不校验、不进提示词）。
    pub books: BTreeMap<String, crate::capabilities::tools::api::ToolSchema>,
    /// 声明了 parallel 的工具名（只读、无副作用；同一回复里的多个可并发调用会真的并发跑）。
    pub parallel: BTreeSet<String>,
}

/// 成员的工具执行环境：来自 module.yaml（按模块分组的放行表）+ 注入的执行端口 + 本成员的沙箱。
pub struct MemberTools {
    /// 这条通道的工具调用形态：envelope = 手写信封（任何供应商都能用）；native = 供应商结构化槽位。
    /// **两套互斥**：native 就不解析信封、正文里的信封也不执行（但如实记失败行）。
    pub mode: crate::capabilities::llm::api::ToolMode,
    /// 模块 id → 该模块的（目录, 工具表）；内置 read/write 不走这里。
    pub modules: BTreeMap<String, ModuleTools>,
    /// 本次会话的观察账本（哪些文件完整读过 / 由核心写过）：改动前的证据（见 crate::capabilities::tools::api::Observations）。
    pub observations: crate::capabilities::tools::api::Observations,
    /// llm 用例面（**不持它的端口**，R12）：手写信封不合法时问它能不能按无歧义的写法修好。
    pub llm: Arc<dyn crate::capabilities::llm::api::Llm + Send + Sync>,
    /// 运行日志：模型输出被长度截断这类"看不见的事实"要落盘，供事后确定问题。
    pub log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 工具执行面（**不持它的端口**，R12）：跑外部/内置工具都走它。
    pub tools: Arc<dyn ToolExec + Send + Sync>,
    /// 本成员的沙箱：内置文件工具的寻址与越界依据（权限收口在 conductor）。
    pub sandbox: crate::capabilities::workspace::api::Sandbox,
    /// 内置工具的参数契约（来自 `systools/tools.yaml` 的 tools）：说明与校验都按它来。
    /// 它属于**工具面**，不属于沙箱——沙箱只管路径。
    pub builtin_tools: crate::capabilities::tools::api::ToolBook,
    /// 模块 id → 它缺的运行包能力（本档位下该模块的工具不执行；空表 = 都能执行）。
    pub unavailable: BTreeMap<String, Vec<String>>,
    /// 本成员工具进程的围栏（可达范围 + 断网）：策略在 conductor 派生，机制在 ToolRunner 适配层安装。
    pub fence: crate::capabilities::tools::api::FenceSpec,
    /// **回复 id 计数器**：一次模型回复一个号，跨重启单调（重建时按转录里的最大值续号）。
    /// 转录行靠它分组（哪几行属于同一次回复），会话靠它按回复原子回档。
    pub reply_seq: u64,
    /// 这个席位**可以调的系统工具 id**（由角色表发放：讨论席 = discussant、执行席 = executor）。
    /// 存在的理由：把"谁能用哪些工具"变成**校验**，而不是提示词里的一句话。
    pub allowed: Vec<String>,
    /// 这个席位**能不能用它自己模块的工具**（角色表的 module_tools；执行席是，讨论席不是）。
    pub with_modules: bool,
    /// 工具说明块的素材（patch 语法 / 模块工具 / 模块工具参数）：装配期算一次，随回合注入。
    pub notes: crate::capabilities::tools::api::ToolNotes,
}

impl MemberTools {
    /// 取下一个回复 id（一次模型回复调用一次）。
    pub(crate) fn next_reply(&mut self) -> u64 {
        self.reply_seq += 1;
        self.reply_seq
    }

    /// **本回合的工具说明块**：核心按这一回合的身份（ids）现渲染，只列这一回合真能调的。
    ///
    /// 为什么不是系统提示里的整本总表：模型会照着给的清单去调工具，列出必然被拒的等于请它去撞墙；
    /// 总表只该留在核心手里当校验判据（见 docs/architecture/tools-and-roles.md 二、三之二）。
    /// 为什么随回合：同一个 agent 会话会用两种身份干活（说话 / 干活），能用的工具随回合变。
    /// 空串 = 这一回合没有可用工具（调用方不注入空块）。
    pub(crate) fn tools_block(&self, ids: &[String], with_modules: bool) -> String {
        let mut parts: Vec<String> = Vec::new();
        for id in ids {
            if let Some(schema) = self.builtin_tools.get(id) {
                parts.push(format!("{}\n{}", id, schema.render_for_prompt()));
            }
        }
        // patch 是自由格式工具：它不在参数清单里，写法跟一段补丁正文（只有拿到它的席位才给）。
        if ids
            .iter()
            .any(|i| i == crate::capabilities::tools::api::PATCH)
        {
            parts.push(self.notes.patch_guide.clone());
        }
        if with_modules {
            parts.push(self.notes.module_tools.clone());
            parts.push(self.notes.module_tool_params.clone());
        }
        if parts.is_empty() {
            return String::new();
        }
        self.sandbox.texts.render(
            &self.sandbox.texts.tools_this_turn,
            &[("tools", parts.join("\n"))],
        )
    }
}

/// 放行表：模块 id → 该模块的（目录, 工具表）。
/// **包含没有声明任何工具的模块**（命令表为空）——这样报错能区分「没有这个模块」与「这个模块没有这个工具」。
/// 跨模块同名工具不再冲突：模块内名字唯一由 map 保证，跨模块由信封里的 module 消歧。
pub fn tool_table(
    modules: &[crate::capabilities::workspace::api::Module],
) -> BTreeMap<String, ModuleTools> {
    modules
        .iter()
        .map(|m| {
            (
                m.manifest.id.clone(),
                ModuleTools {
                    root: m.root.clone(),
                    commands: m
                        .manifest
                        .tools
                        .iter()
                        .map(|(name, decl)| (name.clone(), decl.command.clone()))
                        .collect(),
                    books: m
                        .manifest
                        .tools
                        .iter()
                        .filter_map(|(name, decl)| decl.schema().map(|s| (name.clone(), s)))
                        .collect(),
                    parallel: m
                        .manifest
                        .tools
                        .iter()
                        .filter(|(_, decl)| decl.parallel)
                        .map(|(name, _)| name.clone())
                        .collect(),
                },
            )
        })
        .collect()
}
