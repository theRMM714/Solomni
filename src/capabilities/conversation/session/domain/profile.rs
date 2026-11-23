//! 目的：会话类型表——一个会话类型就是一串机制与它们用到的身份角色；装配期的规则校验在这里。
//! 管：`Profiles` / `Profile` / `MechDecl` 的形状与校验（与形态一一对应、机制已注册、产物按顺序接得上、角色在册）。
//! 不管：yaml 解析（在 `detail/yaml_profiles.rs`）；机制实现（在各机制能力）；会话装配（在 `turn` 的重建里）。
//! 联动：组合根装载并在装配期调用 `validate` / `check_roles`；提示词册子的会话类型以 `ids` 为口径。

use serde::Deserialize;

use crate::capabilities::conversation::session::domain::mechanism::REGISTERED;
use crate::capabilities::conversation::session::domain::mode::Mode;

/// 目的：会话开场那条输入的产物名：第一条机制的 `takes` 只能接它。
pub const SESSION_START: &str = "text";

/// 目的：一个会话类型的成员数：`one` 只有一个 agent，`many` 至少两个、必须有机制调度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agents {
    One,
    Many,
}

/// 目的：会话类型里串着的一个机制：谁被拉进来、吃什么产物、交出什么产物。
#[derive(Debug, Clone, Deserialize)]
pub struct MechDecl {
    /// 目的：机制的注册 id（必须在 `mechanism::REGISTERED` 里）。
    pub id: String,
    /// 目的：这个机制用到的身份角色（都会被拉进会话，各自获得该机制的身份块）。
    pub roles: Vec<String>,
    /// 目的：进来吃的产物类型名：会话开场那条输入，或更早某个机制的 `gives`。
    pub takes: String,
    /// 目的：交出去的产物类型名：后面的机制按它接线。
    pub gives: String,
}

/// 目的：一个会话类型：成员数、身份角色，以及按顺序串起来的机制（空 = 没有机制，直接轮循环）。
#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    /// 目的：会话类型名——必须与 `Mode` 的三个词一一对应。
    pub id: String,
    /// 目的：成员数。
    pub agents: Agents,
    /// 目的：这个类型用到的全部身份角色（机制的 roles 是它的子集）。
    pub roles: Vec<String>,
    /// 目的：按顺序串起来的机制；空表示没有人调度，直接轮循环。
    #[serde(default)]
    pub mechanisms: Vec<MechDecl>,
}

/// 目的：会话类型总表（`systools/profiles.yaml` 的顶层形状）。
#[derive(Debug, Clone, Deserialize)]
pub struct Profiles {
    /// 目的：全部会话类型。
    pub profiles: Vec<Profile>,
}

impl Profiles {
    /// 目的：全部会话类型名——提示词册子拿它校验 `session` 字段。
    pub fn ids(&self) -> Vec<String> {
        self.profiles.iter().map(|p| p.id.clone()).collect()
    }

    /// 目的：装配期校验：每个形态恰好一个类型、机制都注册过、产物按顺序接得上、角色与成员数自洽。
    /// 返回：`Ok(())` 表示这张表可以拿去装配。
    /// 错误：文案点名 `systools/profiles.yaml` 与出问题的那一项。
    pub fn validate(&self) -> Result<(), String> {
        if self.profiles.is_empty() {
            return Err("会话类型表是空的（systools/profiles.yaml）".to_string());
        }
        let mut seen: Vec<&str> = Vec::new();
        for p in &self.profiles {
            if seen.contains(&p.id.as_str()) {
                return Err(format!("会话类型重复：{}（systools/profiles.yaml）", p.id));
            }
            if Mode::parse(&p.id).is_err() {
                return Err(format!(
                    "会话类型不是会话形态：{}（只能是 single / collab / proxy，systools/profiles.yaml）",
                    p.id
                ));
            }
            seen.push(&p.id);
            if p.roles.is_empty() {
                return Err(format!(
                    "会话类型 {} 没有声明身份角色（systools/profiles.yaml）",
                    p.id
                ));
            }
            if p.agents == Agents::Many && p.mechanisms.is_empty() {
                return Err(format!(
                    "会话类型 {} 是多个 agent，却没有机制调度他们（systools/profiles.yaml）",
                    p.id
                ));
            }
            let mut made: Vec<String> = vec![SESSION_START.to_string()];
            let mut mechs: Vec<&str> = Vec::new();
            for m in &p.mechanisms {
                if !REGISTERED.contains(&m.id.as_str()) {
                    return Err(format!(
                        "会话类型 {} 的机制没注册：{}（已注册：{}；systools/profiles.yaml）",
                        p.id,
                        m.id,
                        REGISTERED.join(" / ")
                    ));
                }
                if mechs.contains(&m.id.as_str()) {
                    return Err(format!(
                        "会话类型 {} 里同一个机制出现了两次：{}（systools/profiles.yaml）",
                        p.id, m.id
                    ));
                }
                mechs.push(&m.id);
                if m.roles.is_empty() {
                    return Err(format!(
                        "会话类型 {} 的机制 {} 没有声明身份角色（systools/profiles.yaml）",
                        p.id, m.id
                    ));
                }
                if !made.contains(&m.takes) {
                    return Err(format!(
                        "会话类型 {} 的机制 {} 要吃 {}，但前面没有机制产出它（systools/profiles.yaml）",
                        p.id, m.id, m.takes
                    ));
                }
                if m.gives.trim().is_empty() {
                    return Err(format!(
                        "会话类型 {} 的机制 {} 没有交出产物（systools/profiles.yaml）",
                        p.id, m.id
                    ));
                }
                made.push(m.gives.clone());
            }
        }
        for mode in [Mode::Single, Mode::Collab, Mode::Proxy] {
            if !self.profiles.iter().any(|p| p.id == mode.as_str()) {
                return Err(format!(
                    "会话类型表缺一个会话形态：{}（systools/profiles.yaml）",
                    mode.as_str()
                ));
            }
        }
        Ok(())
    }

    /// 目的：装配期校验角色：每个会话类型与它每个机制引用的角色都得在角色表里。
    /// 参数：`known` = 角色表的全部角色名（组合根从工具能力问来）。
    /// 错误：逐个点名「会话类型 / 机制 → 角色」。
    pub fn check_roles(&self, known: &[String]) -> Result<(), String> {
        let has = |role: &str| known.iter().any(|r| r == role);
        for p in &self.profiles {
            for role in &p.roles {
                if !has(role) {
                    return Err(format!(
                        "会话类型 {} 引用的角色不在角色表里：{}（systools/roles.yaml）",
                        p.id, role
                    ));
                }
            }
            for m in &p.mechanisms {
                for role in &m.roles {
                    if !has(role) {
                        return Err(format!(
                            "会话类型 {} 的机制 {} 引用的角色不在角色表里：{}（systools/roles.yaml）",
                            p.id, m.id, role
                        ));
                    }
                    if !p.roles.iter().any(|r| r == role) {
                        return Err(format!(
                            "会话类型 {} 的机制 {} 用了类型里没声明的角色：{}（systools/profiles.yaml）",
                            p.id, m.id, role
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn mechanisms(id: &str) -> Vec<MechDecl> {
        vec![MechDecl {
            id: id.to_string(),
            roles: vec!["discussant".to_string()],
            takes: SESSION_START.to_string(),
            gives: "TaskChain".to_string(),
        }]
    }

    fn base() -> Profiles {
        Profiles {
            profiles: vec![
                Profile {
                    id: "single".to_string(),
                    agents: Agents::One,
                    roles: vec!["solo".to_string()],
                    mechanisms: Vec::new(),
                },
                Profile {
                    id: "collab".to_string(),
                    agents: Agents::Many,
                    roles: vec!["discussant".to_string()],
                    mechanisms: mechanisms("collab"),
                },
                Profile {
                    id: "proxy".to_string(),
                    agents: Agents::One,
                    roles: vec!["core_proxy".to_string()],
                    mechanisms: Vec::new(),
                },
            ],
        }
    }

    fn roles() -> Vec<String> {
        ["solo", "discussant", "executor", "core_proxy"]
            .iter()
            .map(|r| r.to_string())
            .collect()
    }

    #[test]
    fn the_shape_it_documents_validates() {
        let p = base();
        assert_eq!(p.validate(), Ok(()), "基准表必须合法");
        assert_eq!(p.check_roles(&roles()), Ok(()), "基准表的角色都在册");
        let collab = p
            .profiles
            .iter()
            .find(|x| x.id == Mode::Collab.as_str())
            .expect("协作类型");
        assert_eq!(collab.agents, Agents::Many);
        assert_eq!(collab.mechanisms[0].id, "collab");
        assert_eq!(p.ids(), vec!["single", "collab", "proxy"]);
    }

    #[test]
    fn a_profile_per_mode_and_only_the_three_words() {
        let mut p = base();
        p.profiles.remove(2);
        let err = p.validate().unwrap_err();
        assert!(err.contains("缺一个会话形态：proxy"), "{}", err);

        let mut p = base();
        p.profiles[0].id = "solo".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("不是会话形态：solo"), "{}", err);

        let mut p = base();
        p.profiles[2].id = "single".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("会话类型重复"), "{}", err);
    }

    #[test]
    fn mechanisms_must_be_registered_and_wired_in_order() {
        let mut p = base();
        p.profiles[1].mechanisms = mechanisms("review");
        let err = p.validate().unwrap_err();
        assert!(err.contains("机制没注册：review"), "{}", err);
        assert!(
            err.contains("已注册：collab"),
            "报错要给出已注册清单：{}",
            err
        );

        let mut p = base();
        p.profiles[1].mechanisms[0].takes = "plan".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("前面没有机制产出它"), "{}", err);

        // 第二个机制可以接第一个的产物。
        let mut p = base();
        p.profiles[1].mechanisms.push(MechDecl {
            id: "collab".to_string(),
            roles: vec!["discussant".to_string()],
            takes: "TaskChain".to_string(),
            gives: "Report".to_string(),
        });
        let err = p.validate().unwrap_err();
        assert!(err.contains("同一个机制出现了两次"), "{}", err);

        let mut p = base();
        p.profiles[1].mechanisms[0].gives = " ".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("没有交出产物"), "{}", err);
    }

    #[test]
    fn members_and_roles_must_be_self_consistent() {
        let mut p = base();
        p.profiles[1].mechanisms.clear();
        let err = p.validate().unwrap_err();
        assert!(err.contains("却没有机制调度"), "{}", err);

        let mut p = base();
        p.profiles[0].roles.clear();
        let err = p.validate().unwrap_err();
        assert!(err.contains("没有声明身份角色"), "{}", err);

        let mut p = base();
        p.profiles[1].mechanisms[0].roles.clear();
        let err = p.validate().unwrap_err();
        assert!(err.contains("机制 collab 没有声明身份角色"), "{}", err);

        // 机制用了类型里没声明的角色：装配错误，不静默多拉一个人进来。
        let mut p = base();
        p.profiles[1].mechanisms[0].roles = vec!["executor".to_string()];
        let err = p.check_roles(&roles()).unwrap_err();
        assert!(err.contains("用了类型里没声明的角色：executor"), "{}", err);

        // 角色表里没有这个名字：装配错误。
        let err = base().check_roles(&["solo".to_string()]).unwrap_err();
        assert!(err.contains("不在角色表里：discussant"), "{}", err);
    }

    #[test]
    fn an_empty_table_is_an_assembly_error() {
        let err = Profiles {
            profiles: Vec::new(),
        }
        .validate()
        .unwrap_err();
        assert!(err.contains("会话类型表是空的"), "{}", err);
    }
}
