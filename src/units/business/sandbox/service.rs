//! 目的：沙箱注册表——按策略 id 找实现，并把声明翻成事实。
//! 管：`SandboxPolicies`（登记、查找、事实、开执行句柄）。
//! 不管：策略内部怎么算（实现方）；会话怎么建（`work`）。
//! 联动：组合根在装配期登记；消费方只经本单元的 `api` 取。

use crate::units::business::sandbox::domain::decl::{HOST_POLICY, SandboxDecl};
use crate::units::business::sandbox::domain::exec::Sandbox;
use crate::units::business::sandbox::domain::facts::{PolicyDecl, SandboxFacts};
use crate::units::business::sandbox::domain::policy::{OpenRequest, SandboxPolicy};
use std::sync::Arc;

/// 目的：执行环境策略的注册表（键是策略 id）。
#[derive(Clone)]
pub struct SandboxPolicies {
    policies: Vec<Arc<dyn SandboxPolicy>>,
}

impl SandboxPolicies {
    /// 目的：按登记顺序建表；第一项是缺省策略。
    pub fn new(policies: Vec<Arc<dyn SandboxPolicy>>) -> SandboxPolicies {
        SandboxPolicies { policies }
    }

    /// 目的：按 id 找策略实现。
    pub fn of(&self, id: &str) -> Option<Arc<dyn SandboxPolicy>> {
        self.policies.iter().find(|p| p.decl().id == id).cloned()
    }

    /// 目的：登记的策略 id（诊断与报错用）。
    pub fn ids(&self) -> Vec<String> {
        self.policies.iter().map(|p| p.decl().id).collect()
    }

    /// 目的：登记的策略自我介绍（呈现层的选择器照抄，不自己编名字与说明）。
    pub fn decls(&self) -> Vec<PolicyDecl> {
        self.policies.iter().map(|p| p.decl()).collect()
    }

    /// 目的：按策略自己的规则校验声明（建会话时先过这一关，非法参数当场拒）。
    /// 错误：策略 id 没登记，或策略自己报的问题（多条用「；」连起来）。
    pub fn check(&self, decl: &SandboxDecl) -> Result<(), String> {
        match self.of(&decl.policy) {
            Some(p) => {
                let problems = p.check(decl);
                if problems.is_empty() {
                    Ok(())
                } else {
                    Err(problems.join("；"))
                }
            }
            None => Err(format!(
                "没有这个执行环境策略：{}（登记的有：{}）",
                decl.policy,
                self.ids().join(" / ")
            )),
        }
    }

    /// 目的：缺省策略 id（第一项；空表时是本机）。
    pub fn default_id(&self) -> String {
        self.policies
            .first()
            .map(|p| p.decl().id)
            .unwrap_or_else(|| HOST_POLICY.to_string())
    }

    /// 目的：按声明开一席的执行句柄。
    /// 错误：策略 id 没登记，或策略自己报的问题。
    pub fn open(&self, req: &OpenRequest<'_>) -> Result<Arc<dyn Sandbox>, String> {
        match self.of(&req.decl.policy) {
            Some(p) => p.open(req),
            None => Err(format!(
                "没有这个执行环境策略：{}（登记的有：{}）",
                req.decl.policy,
                self.ids().join(" / ")
            )),
        }
    }

    /// 目的：声明对应的事实。
    /// 错误：策略 id 没登记，或策略自己的参数不合法。
    pub fn facts(&self, decl: &SandboxDecl) -> Result<SandboxFacts, String> {
        match self.of(&decl.policy) {
            Some(p) => p.facts(decl),
            None => Err(format!(
                "没有这个执行环境策略：{}（登记的有：{}）",
                decl.policy,
                self.ids().join(" / ")
            )),
        }
    }
}
