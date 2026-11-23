//! 目的：会话类型表——一个会话类型就是一串流程与它们用到的身份角色；装配期的规则校验在这里。
//! 管：`Profiles` / `Profile` / `FlowDecl` 的形状与校验（与形态一一对应、流程已注册、core/members 的席位与角色自洽）。
//! 不管：yaml 解析（在 `detail/yaml_profiles.rs`）；流程实现（在各流程能力）；会话装配（在 `turn` 的重建里）。
//! 联动：组合根装载并在装配期调用 `validate` / `check_roles`；提示词册子的会话类型以 `ids` 为口径。

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::capabilities::conversation::session::domain::flow::REGISTERED;
use crate::capabilities::conversation::session::domain::mode::Mode;
use crate::capabilities::tools::api::Seat;

/// 目的：一个流程声明：跑哪个已注册的流程、核心席用哪些身份、成员席各要几个。
/// 约束：`core` 必须是核心席角色，`members` 必须是成员席角色；产物按声明顺序一条一条接（上一条的输出就是下一条的输入）。
#[derive(Debug, Clone, Deserialize)]
pub struct FlowDecl {
    /// 目的：流程的注册 id（必须在 `flow::REGISTERED` 里）。
    pub id: String,
    /// 目的：核心席用到的身份（核心通道出演，不进成员表）。
    #[serde(default)]
    pub core: Vec<String>,
    /// 目的：成员席要几员：角色 → 最少人数。
    #[serde(default)]
    pub members: BTreeMap<String, usize>,
}

/// 目的：一个会话类型：身份角色，以及按顺序串起来的流程（空 = 没有流程，直接轮循环）。
#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    /// 目的：会话类型名——必须与 `Mode` 的三个词一一对应。
    pub id: String,
    /// 目的：这个类型用到的全部身份角色（流程里的角色是它的子集）。
    pub roles: Vec<String>,
    /// 目的：按顺序串起来的流程；空表示没有人调度，直接轮循环。
    #[serde(default)]
    pub flows: Vec<FlowDecl>,
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

    /// 目的：一个会话类型按顺序要跑的流程 id（装配按它逐段造；没有这个类型返回 `None`）。
    pub fn flows_of(&self, id: &str) -> Option<Vec<String>> {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.flows.iter().map(|f| f.id.clone()).collect())
    }

    /// 目的：装配期校验：每个形态恰好一个类型、流程都注册过且不重复、每个流程有人出演、用到的角色都在类型里。
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
            let mut flows: Vec<&str> = Vec::new();
            for f in &p.flows {
                if !REGISTERED.contains(&f.id.as_str()) {
                    return Err(format!(
                        "会话类型 {} 的流程没注册：{}（已注册：{}；systools/profiles.yaml）",
                        p.id,
                        f.id,
                        REGISTERED.join(" / ")
                    ));
                }
                if flows.contains(&f.id.as_str()) {
                    return Err(format!(
                        "会话类型 {} 里同一个流程出现了两次：{}（systools/profiles.yaml）",
                        p.id, f.id
                    ));
                }
                flows.push(&f.id);
                if f.core.is_empty() && f.members.is_empty() {
                    return Err(format!(
                        "会话类型 {} 的流程 {} 没有声明任何身份（systools/profiles.yaml）",
                        p.id, f.id
                    ));
                }
                for (role, min) in &f.members {
                    if *min == 0 {
                        return Err(format!(
                            "会话类型 {} 的流程 {} 给 {} 要 0 个成员（systools/profiles.yaml）",
                            p.id, f.id, role
                        ));
                    }
                }
                for role in f.core.iter().chain(f.members.keys()) {
                    if !p.roles.iter().any(|r| r == role) {
                        return Err(format!(
                            "会话类型 {} 的流程 {} 用了类型里没声明的角色：{}（systools/profiles.yaml）",
                            p.id, f.id, role
                        ));
                    }
                }
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

    /// 目的：装配期校验角色与席位：类型与流程引用的角色都得在角色表里，且核心席 / 成员席不能放反。
    /// 参数：`seats` = 角色表的全部角色与坐席（组合根从工具能力问来）。
    /// 错误：逐个点名「会话类型 / 流程 → 角色」。
    pub fn check_roles(&self, seats: &[(String, Seat)]) -> Result<(), String> {
        let seat_of = |role: &str| {
            seats
                .iter()
                .find(|(name, _)| name == role)
                .map(|(_, seat)| *seat)
        };
        for p in &self.profiles {
            for role in &p.roles {
                if seat_of(role).is_none() {
                    return Err(format!(
                        "会话类型 {} 引用的角色不在角色表里：{}（systools/roles.yaml）",
                        p.id, role
                    ));
                }
            }
            for f in &p.flows {
                for role in &f.core {
                    match seat_of(role) {
                        None => {
                            return Err(format!(
                                "会话类型 {} 的流程 {} 引用的角色不在角色表里：{}（systools/roles.yaml）",
                                p.id, f.id, role
                            ));
                        }
                        Some(Seat::Member) => {
                            return Err(format!(
                                "会话类型 {} 的流程 {} 把成员席角色 {} 放在核心席（systools/roles.yaml）",
                                p.id, f.id, role
                            ));
                        }
                        Some(Seat::Core) => {}
                    }
                    if !p.roles.iter().any(|r| r == role) {
                        return Err(format!(
                            "会话类型 {} 的流程 {} 用了类型里没声明的角色：{}（systools/profiles.yaml）",
                            p.id, f.id, role
                        ));
                    }
                }
                for role in f.members.keys() {
                    match seat_of(role) {
                        None => {
                            return Err(format!(
                                "会话类型 {} 的流程 {} 引用的角色不在角色表里：{}（systools/roles.yaml）",
                                p.id, f.id, role
                            ));
                        }
                        Some(Seat::Core) => {
                            return Err(format!(
                                "会话类型 {} 的流程 {} 把核心席角色 {} 放在成员席（systools/roles.yaml）",
                                p.id, f.id, role
                            ));
                        }
                        Some(Seat::Member) => {}
                    }
                    if !p.roles.iter().any(|r| r == role) {
                        return Err(format!(
                            "会话类型 {} 的流程 {} 用了类型里没声明的角色：{}（systools/profiles.yaml）",
                            p.id, f.id, role
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

    fn flow(id: &str) -> FlowDecl {
        FlowDecl {
            id: id.to_string(),
            core: vec!["planner".to_string()],
            members: BTreeMap::from([("discussant".to_string(), 2)]),
        }
    }

    fn base() -> Profiles {
        Profiles {
            profiles: vec![
                Profile {
                    id: "single".to_string(),
                    roles: vec!["solo".to_string()],
                    flows: Vec::new(),
                },
                Profile {
                    id: "collab".to_string(),
                    roles: vec![
                        "discussant".to_string(),
                        "executor".to_string(),
                        "planner".to_string(),
                        "core_proxy".to_string(),
                    ],
                    flows: vec![flow("discuss"), flow("execute")],
                },
                Profile {
                    id: "proxy".to_string(),
                    roles: vec!["core_proxy".to_string()],
                    flows: Vec::new(),
                },
            ],
        }
    }

    fn seats() -> Vec<(String, Seat)> {
        [
            ("solo", Seat::Member),
            ("discussant", Seat::Member),
            ("executor", Seat::Member),
            ("core_proxy", Seat::Core),
            ("planner", Seat::Core),
        ]
        .iter()
        .map(|(role, seat)| (role.to_string(), *seat))
        .collect()
    }

    #[test]
    fn the_shape_it_documents_validates() {
        let p = base();
        assert_eq!(p.validate(), Ok(()), "基准表必须合法");
        assert_eq!(p.check_roles(&seats()), Ok(()), "基准表的角色都在册");
        let collab = p
            .profiles
            .iter()
            .find(|x| x.id == Mode::Collab.as_str())
            .expect("协作类型");
        assert_eq!(collab.flows[0].id, "discuss");
        assert_eq!(collab.flows[0].members["discussant"], 2);
        assert_eq!(collab.flows[1].id, "execute");
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
    fn flows_must_be_registered_and_declared_once() {
        let mut p = base();
        p.profiles[1].flows = vec![flow("review")];
        let err = p.validate().unwrap_err();
        assert!(err.contains("流程没注册：review"), "{}", err);
        assert!(
            err.contains("已注册：discuss / execute"),
            "报错要给出已注册清单：{}",
            err
        );

        let mut p = base();
        p.profiles[1].flows.push(flow("discuss"));
        let err = p.validate().unwrap_err();
        assert!(err.contains("同一个流程出现了两次"), "{}", err);
    }

    #[test]
    fn every_flow_needs_a_seat_and_declared_roles() {
        let mut p = base();
        p.profiles[1].flows[0].core.clear();
        p.profiles[1].flows[0].members.clear();
        let err = p.validate().unwrap_err();
        assert!(err.contains("没有声明任何身份"), "{}", err);

        let mut p = base();
        p.profiles[1].flows[0]
            .members
            .insert("executor".to_string(), 0);
        let err = p.validate().unwrap_err();
        assert!(err.contains("要 0 个成员"), "{}", err);

        let mut p = base();
        p.profiles[1].flows[0]
            .members
            .insert("executor".to_string(), 1);
        p.profiles[1].roles.retain(|r| r != "executor");
        let err = p.validate().unwrap_err();
        assert!(err.contains("用了类型里没声明的角色：executor"), "{}", err);
    }

    #[test]
    fn seats_must_match_the_role_table() {
        // 成员席角色放进核心席：装配错误。
        let mut p = base();
        p.profiles[1].flows[0].core = vec!["discussant".to_string()];
        let err = p.check_roles(&seats()).unwrap_err();
        assert!(err.contains("成员席角色 discussant 放在核心席"), "{}", err);

        // 核心席角色放进成员席：同样是装配错误。
        let mut p = base();
        p.profiles[1].flows[0]
            .members
            .insert("planner".to_string(), 1);
        let err = p.check_roles(&seats()).unwrap_err();
        assert!(err.contains("核心席角色 planner 放在成员席"), "{}", err);

        // 角色表里没有这个名字：装配错误。
        let mut p = base();
        p.profiles[1].roles.push("reviewer".to_string());
        let err = p.check_roles(&seats()).unwrap_err();
        assert!(err.contains("不在角色表里：reviewer"), "{}", err);
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
