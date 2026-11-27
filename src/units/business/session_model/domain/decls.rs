//! 目的：会话形状的声明聚合——流程段表与会话类型表放在一处，跨表不变量只在这里。
//! 管：`SessionDecls`（两张表 + 装配期校验 + 按类型解析规格 / 最少人数 / 标签 / 已知判定）。
//! 不管：单张表自己的形状与规则（`mechanism.rs` / `profile.rs`）；装载（`detail/yaml_decls.rs`）；流程实现（collab 的注册表）。
//! 联动：组合根装载它、装配期调 `validate`；`work` / `collab` / `turn` 经 `Flows::decls()` 读它。

use super::mechanism::Loops;
use super::profile::{Profiles, SessionSpec};

/// 目的：一份会话形状声明的全量：段与角色（`loops.yaml`）+ 会话类型（`profiles.yaml`）。
/// 约束：跨表的引用（类型的 flows / loop 指向段）只在 `validate` 里判；单表规则留在各自的类型上。
#[derive(Debug, Clone)]
pub struct SessionDecls {
    /// 目的：段与角色表。
    loops: Loops,
    /// 目的：会话类型表。
    profiles: Profiles,
}

impl SessionDecls {
    /// 目的：把两张表合成一份声明（组合根的装载器造它）。
    pub fn new(loops: Loops, profiles: Profiles) -> SessionDecls {
        SessionDecls { loops, profiles }
    }

    /// 目的：没有任何声明（核实环境与不查段表的替身用：它们只挂 agent 来源，段表永不被读）。
    pub fn empty() -> SessionDecls {
        SessionDecls {
            loops: Loops::default(),
            profiles: Profiles::default(),
        }
    }

    /// 目的：段与角色表（装配一席的工具面按它取机制动作与抑制面）。
    pub fn loops(&self) -> &Loops {
        &self.loops
    }

    /// 目的：会话类型表（创建期查类型、视图里列类型）。
    pub fn profiles(&self) -> &Profiles {
        &self.profiles
    }

    /// 目的：装配期校验：两张表各自的规则加上跨表的引用与产物链。
    /// 返回：`Ok(())` 表示这份声明可以拿去装配。
    /// 错误：文案点名出问题的那一项与文件位置。
    pub fn validate(&self) -> Result<(), String> {
        self.loops.validate()?;
        self.profiles.validate(&self.loops)
    }

    /// 目的：把一个会话类型解析成落盘声明；没有这个类型返回 `None`。
    pub fn spec_of(&self, kind: &str) -> Option<SessionSpec> {
        self.profiles.spec_of(kind, &self.loops)
    }

    /// 目的：这个类型是不是表里的一个预设。
    pub fn known(&self, kind: &str) -> bool {
        self.profiles.known(kind)
    }

    /// 目的：这一串流程开跑至少要有几个 agent。
    pub fn min_agents(&self, flows: &[String]) -> usize {
        self.profiles.min_agents(flows, &self.loops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::business::session_model::domain::artifact::Kind;
    use crate::units::business::session_model::domain::mechanism::{Cast, Loop, Role};
    use crate::units::business::session_model::domain::profile::Profile;

    fn loops() -> Loops {
        Loops {
            roles: vec![
                Role {
                    id: "core".to_string(),
                    prompt: "roles/core".to_string(),
                    core: true,
                },
                Role {
                    id: "member".to_string(),
                    prompt: "roles/member".to_string(),
                    core: false,
                },
            ],
            loops: vec![Loop {
                id: "work".to_string(),
                builder: Some("work".to_string()),
                cast: vec![
                    Cast {
                        role: "core".to_string(),
                        min: 0,
                        tools: Vec::new(),
                    },
                    Cast {
                        role: "member".to_string(),
                        min: 2,
                        tools: Vec::new(),
                    },
                ],
                deny: Vec::new(),
                ask: Vec::new(),
                accepts: vec![Kind::Requirement],
                produces: None,
            }],
        }
    }

    fn profiles() -> Profiles {
        Profiles {
            profiles: vec![Profile {
                id: "team".to_string(),
                name: "组队".to_string(),
                hint: String::new(),
                loop_id: String::new(),
                flows: vec!["work".to_string()],
                auto: false,
                agent_visible: true,
                sources: Vec::new(),
                system_tools: Vec::new(),
            }],
        }
    }

    #[test]
    fn the_aggregate_checks_both_tables() {
        let d = SessionDecls::new(loops(), profiles());
        assert_eq!(d.validate(), Ok(()), "基准声明必须合法");
        assert_eq!(d.profiles().label_of("team"), "组队");
        assert!(d.known("team"));
        assert_eq!(d.min_agents(&["work".to_string()]), 2);
        let spec = d.spec_of("team").expect("声明的类型");
        assert_eq!(spec.flows, vec!["work"]);
    }

    #[test]
    fn a_flow_that_is_not_a_segment_is_refused() {
        let mut p = profiles();
        p.profiles[0].flows = vec!["ghost".to_string()];
        let err = SessionDecls::new(loops(), p).validate().unwrap_err();
        assert!(err.contains("流程没注册：ghost"), "{}", err);
    }
}
