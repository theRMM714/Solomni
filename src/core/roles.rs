//! 角色（身份）：**场景绑定的身份**，带提示词与系统工具面。
//!
//! 两个真相源各一张表（见 docs/architecture/tools-and-roles.md）：
//! - `systools/tools.yaml`：工具**是什么**（说明、参数契约、能否并发、能力）；
//! - `systools/roles.yaml`：这个**身份有什么**（引用的系统工具 id + 提示词）。
//!
//! 代码里**不得**出现"哪个角色能调哪个工具"的判断：只有两个动作——按角色组装工具面、按表校验调用。

use serde::Deserialize;
use std::collections::BTreeMap;

/// 一个角色：引用的系统工具 id + 提示词（提示词与工具面**同处声明**）。
///
/// 为什么同处：**信封模式**下该角色能用的信封清单要渲染进提示词——提示词与工具面分开声明必然漂。
#[derive(Debug, Clone, Deserialize)]
pub struct RoleDecl {
    /// 提示词在册子里的位置（`prompts/roles/<名字>.yaml`）。
    #[serde(default)]
    pub prompt: String,
    /// 这个角色能用的**系统工具 id**（模块工具按成员归属自动获得，不在这里）。
    #[serde(default)]
    pub tools: Vec<String>,
    /// 这个身份**能不能用它自己所属模块的工具**（默认否）。
    /// 为什么要在表里：讨论席拿不到干活的手段（模块工具一律拒绝），执行席才发。
    /// 少了这一格，"本回合该不该把模块工具写进提示词"就只能在代码里各判一次，迟早漂。
    #[serde(default)]
    pub module_tools: bool,
}

/// 角色表（key = 角色 id）。
pub type RoleTable = BTreeMap<String, RoleDecl>;

/// 系统工具与角色（装配期读 `systools/` 得来）。
#[derive(Debug, Clone, Default)]
pub struct SystemTools {
    pub tools: crate::core::schema::ToolBook,
    pub roles: RoleTable,
}

impl SystemTools {
    /// 按角色组装工具面：角色引用的 id 逐个解析成工具声明（顺序即角色表里的顺序）。
    ///
    /// 未知名一律**如实报错**（不静默跳过）：悬空引用是装配错误，不是运行期可以忽略的小事。
    pub fn tool_face(
        &self,
        role: &str,
    ) -> Result<Vec<(&str, &crate::core::schema::ToolSchema)>, String> {
        let decl = self
            .roles
            .get(role)
            .ok_or_else(|| format!("角色表里没有这个角色：{}", role))?;
        let mut out = Vec::new();
        for id in &decl.tools {
            let schema = self
                .tools
                .get(id)
                .ok_or_else(|| format!("角色 {} 引用了不存在的系统工具：{}", role, id))?;
            out.push((id.as_str(), schema));
        }
        Ok(out)
    }

    /// 这个身份能不能用它自己模块的工具（论据：角色表的 module_tools）。
    pub fn allows_module_tools(&self, role: &str) -> bool {
        self.roles
            .get(role)
            .map(|d| d.module_tools)
            .unwrap_or(false)
    }

    /// 这个角色能不能调这个工具（越权校验的唯一判据）。
    pub fn allows(&self, role: &str, tool: &str) -> bool {
        self.roles
            .get(role)
            .map(|d| d.tools.iter().any(|t| t == tool))
            .unwrap_or(false)
    }

    /// 悬空引用（角色引用了总表里没有的 id）与缺能力的工具：都返回可读的原因，空 = 一切正常。
    /// 由测试门禁消费——两张表不漂靠它，不靠人看。
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, schema) in &self.tools {
            if schema.capability.trim().is_empty() {
                out.push(format!("工具 {} 没声明 capability（权限收口依据）", id));
            }
        }
        for (role, decl) in &self.roles {
            if decl.prompt.trim().is_empty() {
                out.push(format!("角色 {} 没声明提示词", role));
            }
            for id in &decl.tools {
                if !self.tools.contains_key(id) {
                    out.push(format!("角色 {} 引用了不存在的系统工具：{}", role, id));
                }
            }
        }
        out
    }
}
