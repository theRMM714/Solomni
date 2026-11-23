//! 目的：会话类型表——一个会话类型就是一串流程与它们用到的身份角色；装配期的规则校验在这里。
//! 管：`Profiles` / `Profile` / `FlowDecl` / `SessionSpec` 的形状与校验（流程已注册、core/members 的席位与角色自洽）；
//!   会话创建时把选中的类型**解析成** `SessionSpec` 落进 meta，装配只看那份声明。
//! 不管：yaml 解析（在 `detail/yaml_profiles.rs`）；流程实现（在各流程能力）；会话装配（在 `turn` 的重建里）。
//! 联动：组合根装载并在装配期调用 `validate` / `check_roles`；提示词册子的会话类型以 `ids` 为口径。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::capabilities::conversation::session::domain::flow::REGISTERED;
use crate::capabilities::tools::api::Seat;

/// 目的：产品自带的单机类型 id（一个 agent、没有流程）。
pub const SINGLE: &str = "single";
/// 目的：产品自带的会议类型 id（多个 agent，讨论后执行）。
/// 约束：非测试代码不按它分支（按会话类型的流程走），只有产品自带的会话类型表与测试引用它。
#[allow(dead_code)]
pub const MEETING: &str = "collab";
/// 目的：产品自带的代理类型 id（核心代理在该会话里工作，可以建子会话）。
pub const PROXY: &str = "proxy";
/// 目的：核心代理坐的角色名（角色表里的名字；代理式会话没有流程、坐这个席）。
pub const PROXY_ROLE: &str = "core_proxy";
/// 目的：临时拼装用的会话类型 id（没落进类型表，声明随会话一起落盘）。
pub const CUSTOM: &str = "custom";
/// 目的：临时拼装的显示名（界面与转录里按这个名字显示，不用 id）。
pub const CUSTOM_NAME: &str = "临时组合";

/// 目的：一个流程声明：跑哪个已注册的流程、核心席用哪些身份、成员席各要几个。
/// 约束：`core` 必须是核心席角色，`members` 必须是成员席角色；产物按声明顺序一条一条接（上一条的输出就是下一条的输入）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// 目的：一条会话落盘的**解析后声明**：哪一类会话、这条会话自己坐哪个角色、按顺序跑哪些流程。
/// 约束：装配只看这份声明（不按类型名分岔）；`flows` 空 = 没有调度者，直接轮循环。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    /// 目的：会话类型 id（命令与落盘用；表里的预设 id，如 single / collab / proxy）。
    pub kind: String,
    /// 目的：会话类型的显示名（界面渲染用，如 单机 / 会议 / 代理）。
    #[serde(default)]
    pub name: String,
    /// 目的：这条会话自己的身份角色（流程在回合里另换身份，这里是它静止时的身份）。
    #[serde(default)]
    pub role: String,
    /// 目的：按顺序串起来的流程；空表示没有人调度，直接轮循环。
    #[serde(default)]
    pub flows: Vec<FlowDecl>,
}

impl SessionSpec {
    /// 目的：造一条没有流程的声明（单 agent 或代理会话）。
    pub fn plain(kind: &str, name: &str, role: &str) -> SessionSpec {
        SessionSpec {
            kind: kind.to_string(),
            name: name.to_string(),
            role: role.to_string(),
            flows: Vec::new(),
        }
    }

    /// 目的：这条会话有没有流程在推进（界面据此决定能不能自由发言）。
    pub fn is_proxy_spec(&self) -> bool {
        !self.driven() && self.role == PROXY_ROLE
    }

    /// 目的：这条声明要不要由流程推进（有流程 = 由流程推；没有 = 直接轮循环）。
    pub fn driven(&self) -> bool {
        !self.flows.is_empty()
    }

    /// 目的：要跑的流程 id，按声明顺序。
    pub fn flow_ids(&self) -> Vec<String> {
        self.flows.iter().map(|f| f.id.clone()).collect()
    }
}

/// 目的：把临时拼装的流程声明包成落盘声明（类型 id 固定 `custom`，根身份取流程里出现的第一个角色）。
/// 参数：`flows` = 已通过 `Profiles::check_inline` 校验的流程声明。
/// 返回：可以直接落进 `SessionMeta` 的声明。
pub fn inline_spec(flows: Vec<FlowDecl>) -> SessionSpec {
    let role = flows
        .iter()
        .find_map(|f| f.core.first().cloned())
        .or_else(|| flows.iter().find_map(|f| f.members.keys().next().cloned()))
        .unwrap_or_default();
    SessionSpec {
        kind: CUSTOM.to_string(),
        name: CUSTOM_NAME.to_string(),
        role,
        flows,
    }
}

/// 目的：一个会话类型：身份角色，以及按顺序串起来的流程（空 = 没有流程，直接轮循环）。
#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    /// 目的：会话类型 id（命令与落盘用，如 single / collab / proxy；提示词册按它配对）。
    pub id: String,
    /// 目的：显示名（界面渲染用，如 单机 / 会议 / 代理）。
    #[serde(default)]
    pub name: String,
    /// 目的：给界面的一句说明（选类型时显示这句差别；空 = 不额外解释）。
    #[serde(default)]
    pub hint: String,
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

    /// 目的：把一个类型解析成落盘声明（root 角色取类型里的第一个角色）；没有这个类型返回 `None`。
    pub fn spec_of(&self, kind: &str) -> Option<SessionSpec> {
        self.profiles
            .iter()
            .find(|p| p.id == kind)
            .map(|p| SessionSpec {
                kind: p.id.clone(),
                name: p.name.clone(),
                role: p.roles.first().cloned().unwrap_or_default(),
                flows: p.flows.clone(),
            })
    }

    /// 目的：显示名（界面要按预设名字渲染；找不到这个类型就退回 id）。
    pub fn label_of(&self, kind: &str) -> String {
        self.profiles
            .iter()
            .find(|p| p.id == kind)
            .map(|p| {
                if p.name.trim().is_empty() {
                    p.id.clone()
                } else {
                    p.name.clone()
                }
            })
            .unwrap_or_else(|| kind.to_string())
    }

    /// 目的：这个类型是不是表里的一个预设（临时拼装另走校验，不算预设）。
    pub fn known(&self, kind: &str) -> bool {
        self.profiles.iter().any(|p| p.id == kind)
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
            if p.id.trim().is_empty() {
                return Err("会话类型的 id 是空的（systools/profiles.yaml）".to_string());
            }
            if p.id.chars().any(|c| c.is_whitespace()) {
                return Err(format!(
                    "会话类型 id 不能含空白：{}（systools/profiles.yaml）",
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
        Ok(())
    }

    /// 目的：装配期校验角色与席位：类型与流程引用的角色都得在角色表里，且核心席 / 成员席不能放反。
    /// 参数：`seats` = 角色表的全部角色与坐席（组合根从工具能力问来）。
    /// 错误：逐个点名「会话类型 / 流程 → 角色」。
    pub fn check_roles(&self, seats: &[(String, Seat)]) -> Result<(), String> {
        for p in &self.profiles {
            check_profile(p, seats)?;
        }
        Ok(())
    }

    /// 目的：临时拼装的校验：流程都注册过且不重复、每个流程有人出演、角色都在册且席位没坐反。
    /// 参数：`flows` = 调用方内联的流程声明；`seats` = 角色表的全部角色与坐席。
    /// 返回：`Ok(())` 表示这场临时拼装可以拿去装配。
    /// 错误：文案点名「临时拼装」与出问题的那一项。
    pub fn check_inline(&self, flows: &[FlowDecl], seats: &[(String, Seat)]) -> Result<(), String> {
        if flows.is_empty() {
            return Err("临时拼装至少要有一个流程（systools/profiles.yaml）".to_string());
        }
        let mut seen: Vec<&str> = Vec::new();
        let mut roles: Vec<String> = Vec::new();
        for f in flows {
            if !REGISTERED.contains(&f.id.as_str()) {
                return Err(format!(
                    "临时拼装的流程没注册：{}（已注册：{}）",
                    f.id,
                    REGISTERED.join(" / ")
                ));
            }
            if seen.contains(&f.id.as_str()) {
                return Err(format!("临时拼装里同一个流程出现了两次：{}", f.id));
            }
            seen.push(&f.id);
            if f.core.is_empty() && f.members.is_empty() {
                return Err(format!("临时拼装的流程 {} 没有声明任何身份", f.id));
            }
            for (role, min) in &f.members {
                if *min == 0 {
                    return Err(format!("临时拼装的流程 {} 给 {} 要 0 个成员", f.id, role));
                }
            }
            for role in f.core.iter().chain(f.members.keys()) {
                if !roles.contains(role) {
                    roles.push(role.clone());
                }
            }
        }
        let p = Profile {
            id: CUSTOM.to_string(),
            name: CUSTOM_NAME.to_string(),
            hint: String::new(),
            roles,
            flows: flows.to_vec(),
        };
        check_profile(&p, seats)
    }
}

/// 目的：一个类型的席位与角色自洽：角色都在册、核心席不坐成员角色、成员席不坐核心角色。
/// 参数：`p` = 要检查的类型（表里的或临时拼装包出来的）；`seats` = 角色表的全部角色与坐席。
/// 错误：逐个点名「类型 / 流程 → 角色」。
fn check_profile(p: &Profile, seats: &[(String, Seat)]) -> Result<(), String> {
    let seat_of = |role: &str| {
        seats
            .iter()
            .find(|(name, _)| name == role)
            .map(|(_, seat)| *seat)
    };
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
    Ok(())
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
                    id: SINGLE.to_string(),
                    name: "单机".to_string(),
                    hint: "一个 agent 直接干活。".to_string(),
                    roles: vec!["solo".to_string()],
                    flows: Vec::new(),
                },
                Profile {
                    id: MEETING.to_string(),
                    name: "会议".to_string(),
                    hint: "先讨论再执行。".to_string(),
                    roles: vec![
                        "discussant".to_string(),
                        "executor".to_string(),
                        "planner".to_string(),
                        "core_proxy".to_string(),
                    ],
                    flows: vec![flow("discuss"), flow("execute")],
                },
                Profile {
                    id: PROXY.to_string(),
                    name: "代理".to_string(),
                    hint: "决定权交给核心。".to_string(),
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
            .find(|x| x.id == MEETING)
            .expect("会议类型");
        assert_eq!(collab.flows[0].id, "discuss");
        assert_eq!(collab.flows[0].members["discussant"], 2);
        assert_eq!(collab.flows[1].id, "execute");
        assert_eq!(p.ids(), vec![SINGLE, MEETING, PROXY]);
        assert_eq!(p.label_of(MEETING), "会议");
        assert_eq!(p.spec_of(MEETING).expect("会议类型").name, "会议");
        assert_eq!(
            p.spec_of(MEETING).expect("会议类型").flow_ids(),
            vec!["discuss", "execute"]
        );
        assert!(p.spec_of(SINGLE).expect("单机").flows.is_empty());
        assert!(p.spec_of("临时").is_none(), "没声明的类型解析不出声明");
    }

    #[test]
    fn a_type_id_must_be_unique_and_not_empty() {
        let mut p = base();
        p.profiles[0].id = "  ".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("会话类型的 id 是空的"), "{}", err);

        let mut p = base();
        p.profiles[0].id = "单 机".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("不能含空白"), "{}", err);

        let mut p = base();
        p.profiles[2].id = SINGLE.to_string();
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
    fn an_inline_assembly_is_checked_against_the_same_whitelist() {
        let p = base();
        // 合法：现拼一段讨论。
        p.check_inline(&[flow("discuss")], &seats())
            .expect("表里声明过的流程可以直接拼");

        // 空表：临时拼装至少要有一段流程。
        let err = p.check_inline(&[], &seats()).unwrap_err();
        assert!(err.contains("至少要有一个流程"), "{}", err);

        // 没注册的流程。
        let err = p.check_inline(&[flow("review")], &seats()).unwrap_err();
        assert!(err.contains("没注册：review"), "{}", err);

        // 同一段流程拼两次。
        let err = p
            .check_inline(&[flow("discuss"), flow("discuss")], &seats())
            .unwrap_err();
        assert!(err.contains("同一个流程出现了两次"), "{}", err);

        // 席位坐反：成员席角色放进核心席，仍然要拦。
        let mut bad = flow("discuss");
        bad.core = vec!["discussant".to_string()];
        let err = p.check_inline(&[bad], &seats()).unwrap_err();
        assert!(err.contains("成员席角色 discussant 放在核心席"), "{}", err);

        // 折出来的声明：类型记作 custom，流程原样带上。
        let spec = inline_spec(vec![flow("discuss"), flow("execute")]);
        assert_eq!(spec.kind, CUSTOM);
        assert_eq!(spec.name, CUSTOM_NAME);
        assert_eq!(spec.role, "planner", "角色取第一段流程的核心席");
        assert_eq!(spec.flow_ids(), vec!["discuss", "execute"]);
        assert!(spec.driven());
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
