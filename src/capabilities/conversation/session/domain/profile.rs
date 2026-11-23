//! 目的：会话类型表——一个会话类型就是一组身份与一串要跑的流程；装配期的规则校验在这里。
//! 管：`Profiles` / `Profile` / `SessionSpec` 的形状与校验（流程已注册、角色在册、席位对、人数够）；
//!   会话创建时把选中的类型**解析成** `SessionSpec` 落进 meta，装配只看那份声明。
//! 不管：yaml 解析（在 `detail/yaml_profiles.rs`）；流程实现与它要的席位（在 `domain/flow.rs` 的 `FLOWS`）；会话装配（在 `turn` 的重建里）。
//! 联动：组合根装载并在装配期调用 `validate` / `check_roles`；创建期按 `min_agents` 校验名单。

use serde::{Deserialize, Serialize};

use crate::capabilities::conversation::session::domain::flow::cast_of;
use crate::capabilities::tools::api::Seat;

/// 目的：产品自带的单机类型 id（一个 agent、没有流程）。
/// 约束：只有产品自带的会话类型表与测试知道它——代码按声明分支，不按 id 分支。
#[cfg(test)]
pub const SINGLE: &str = "single";
/// 目的：产品自带的会议类型 id（多个 agent，讨论后执行）。
/// 约束：只有产品自带的会话类型表与测试知道它——代码按声明分支，不按 id 分支。
#[cfg(test)]
pub const MEETING: &str = "collab";
/// 目的：产品自带的代理类型 id（核心代理在该会话里工作，可以建子会话）。
/// 约束：只有产品自带的会话类型表与测试知道它——代码按声明分支，不按 id 分支。
#[cfg(test)]
pub const PROXY: &str = "proxy";
/// 目的：核心代理坐的角色名（角色表里的名字；代理式会话没有流程、坐这个席）。
/// 约束：产品自带类型表里的代理项坐它；代码按声明里的 `auto` 判定，不认这个名字。
#[cfg(test)]
pub const PROXY_ROLE: &str = "core_proxy";
/// 目的：临时拼装用的会话类型 id（没落进类型表，声明随会话一起落盘）。
pub const CUSTOM: &str = "custom";
/// 目的：临时拼装的显示名（界面与转录里按这个名字显示，不用 id）。
pub const CUSTOM_NAME: &str = "临时组合";

/// 目的：一条会话落盘的**解析后声明**：哪一类会话、这条会话自己坐哪个角色、按顺序跑哪些流程。
/// 约束：装配只看这份声明（不按类型名分岔）；`flows` 空 = 没有调度者，直接轮循环。
///   有流程的会话没有静止身份（`role` 为空）——身份由流程在回合里现给。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    /// 目的：会话类型 id（命令与落盘用；表里的预设 id，如 single / collab / proxy）。
    pub kind: String,
    /// 目的：会话类型的显示名（界面渲染用，如 单机 / 会议 / 代理）。
    #[serde(default)]
    pub name: String,
    /// 目的：没有流程时的静止身份；有流程时为空串（流程在回合里另换身份）。
    #[serde(default)]
    pub role: String,
    /// 目的：按顺序串起来的流程 id；空表示没有人调度，直接轮循环。
    #[serde(default)]
    pub flows: Vec<String>,
    /// 目的：整块交给核心代理（没有名单、没有本次需求，核心在该会话里工作并自己决定做什么）。
    /// 约束：由会话类型声明，不按类型 id 或角色名判定；`true` 的类型不得再声明流程。
    #[serde(default)]
    pub auto: bool,
}

impl SessionSpec {
    /// 目的：造一条没有流程的声明（单 agent 会话；代理式再用 `automatic` 打开 `auto`）。
    pub fn plain(kind: &str, name: &str, role: &str) -> SessionSpec {
        SessionSpec {
            kind: kind.to_string(),
            name: name.to_string(),
            role: role.to_string(),
            flows: Vec::new(),
            auto: false,
        }
    }

    /// 目的：造一条**整块交给核心**的声明（代理式会话：没有流程、没有静止需求）。
    pub fn automatic(kind: &str, name: &str, role: &str) -> SessionSpec {
        SessionSpec {
            auto: true,
            ..SessionSpec::plain(kind, name, role)
        }
    }

    /// 目的：这条会话是不是代理式（整块交给核心：没有流程、没有名单）。
    pub fn is_proxy_spec(&self) -> bool {
        !self.driven() && self.auto
    }

    /// 目的：这条声明要不要由流程推进（有流程 = 由流程推；没有 = 直接轮循环）。
    pub fn driven(&self) -> bool {
        !self.flows.is_empty()
    }

    /// 目的：要跑的流程 id，按声明顺序。
    pub fn flow_ids(&self) -> Vec<String> {
        self.flows.clone()
    }
}

/// 目的：把临时拼装的流程 id 包成落盘声明（类型 id 固定 `custom`，没有静止身份——身份由流程给）。
/// 参数：`flows` = 已通过 `Profiles::check_inline` 校验的流程 id。
/// 返回：可以直接落进 `SessionMeta` 的声明。
pub fn inline_spec(flows: Vec<String>) -> SessionSpec {
    SessionSpec {
        kind: CUSTOM.to_string(),
        name: CUSTOM_NAME.to_string(),
        role: String::new(),
        flows,
        auto: false,
    }
}

/// 目的：一个会话类型：一串要跑的流程（空 = 没有流程，直接轮循环）与它的静止身份。
#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    /// 目的：会话类型 id（命令与落盘用，如 single / collab / proxy）。
    pub id: String,
    /// 目的：显示名（界面渲染用，如 单机 / 会议 / 代理）。
    #[serde(default)]
    pub name: String,
    /// 目的：给界面的一句说明（选类型时显示这句差别；空 = 不额外解释）。
    #[serde(default)]
    pub hint: String,
    /// 目的：没有流程时的静止身份；有流程时不用声明（身份由流程按回发放，见 `flow::FLOWS`）。
    #[serde(default)]
    pub roles: Vec<String>,
    /// 目的：按顺序串起来的流程 id；空表示没有人调度，直接轮循环。
    #[serde(default)]
    pub flows: Vec<String>,
    /// 目的：整块交给核心代理的类型（没有名单、没有需求；核心在该会话里自己决定做什么）。
    #[serde(default)]
    pub auto: bool,
}

/// 目的：会话类型总表（`systools/profiles.yaml` 的顶层形状）。
#[derive(Debug, Clone, Deserialize)]
pub struct Profiles {
    /// 目的：全部会话类型。
    pub profiles: Vec<Profile>,
}

impl Profiles {
    /// 目的：把一个类型解析成落盘声明；没有这个类型返回 `None`。
    /// 约束：有流程的会话 `role` 取空串（静止身份对流程会话没有意义）；没有流程时取类型声明的第一个角色。
    pub fn spec_of(&self, kind: &str) -> Option<SessionSpec> {
        self.profiles
            .iter()
            .find(|p| p.id == kind)
            .map(|p| SessionSpec {
                kind: p.id.clone(),
                name: p.name.clone(),
                role: if p.flows.is_empty() {
                    p.roles.first().cloned().unwrap_or_default()
                } else {
                    String::new()
                },
                flows: p.flows.clone(),
                auto: p.auto,
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

    /// 目的：这一串流程开跑至少要有几个 agent（各段流程要的人数取最大值——每段都要在自己的会话里跑得起来）。
    pub fn min_agents(&self, flows: &[String]) -> usize {
        flows
            .iter()
            .filter_map(|id| cast_of(id))
            .map(|c| c.members.iter().map(|(_, min)| *min).sum::<usize>())
            .max()
            .unwrap_or(0)
    }

    /// 目的：装配期校验：类型 id 唯一且无空白、流程都注册过且不重复、没有流程的类型要有静止身份。
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
            if p.auto && !p.flows.is_empty() {
                return Err(format!(
                    "会话类型 {} 整块交给核心，不能再声明流程（systools/profiles.yaml）",
                    p.id
                ));
            }
            if p.flows.is_empty() && p.roles.is_empty() {
                return Err(format!(
                    "会话类型 {} 没有声明身份角色（没有流程的会话要一个静止身份；systools/profiles.yaml）",
                    p.id
                ));
            }
            let mut flows: Vec<&str> = Vec::new();
            for id in &p.flows {
                if cast_of(id).is_none() {
                    return Err(format!(
                        "会话类型 {} 的流程没注册：{}（已注册：{}；systools/profiles.yaml）",
                        p.id,
                        id,
                        crate::capabilities::conversation::session::domain::flow::registered()
                            .join(" / ")
                    ));
                }
                if flows.contains(&id.as_str()) {
                    return Err(format!(
                        "会话类型 {} 里同一个流程出现了两次：{}（systools/profiles.yaml）",
                        p.id, id
                    ));
                }
                flows.push(id);
            }
        }
        Ok(())
    }

    /// 目的：装配期校验角色与席位：类型声明的静止身份与每段流程要的身份都得在角色表里，且核心席 / 成员席不能放反。
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
            for id in &p.flows {
                let Some(cast) = cast_of(id) else {
                    continue;
                };
                check_cast(&p.id, cast, &seat_of)?;
            }
        }
        Ok(())
    }

    /// 目的：临时拼装的校验：流程都注册过且不重复、要的身份都在册且席位没坐反。
    /// 参数：`flows` = 调用方内联的流程 id；`seats` = 角色表的全部角色与坐席。
    /// 返回：`Ok(())` 表示这场临时拼装可以拿去装配。
    /// 错误：文案点名「临时拼装」与出问题的那一项。
    pub fn check_inline(&self, flows: &[String], seats: &[(String, Seat)]) -> Result<(), String> {
        if flows.is_empty() {
            return Err("临时拼装至少要有一个流程".to_string());
        }
        let seat_of = |role: &str| {
            seats
                .iter()
                .find(|(name, _)| name == role)
                .map(|(_, seat)| *seat)
        };
        let mut seen: Vec<&str> = Vec::new();
        for id in flows {
            let Some(cast) = cast_of(id) else {
                return Err(format!(
                    "临时拼装的流程没注册：{}（已注册：{}）",
                    id,
                    crate::capabilities::conversation::session::domain::flow::registered()
                        .join(" / ")
                ));
            };
            if seen.contains(&id.as_str()) {
                return Err(format!("临时拼装里同一个流程出现了两次：{}", id));
            }
            seen.push(id);
            check_cast(CUSTOM, cast, &seat_of)?;
        }
        Ok(())
    }
}

/// 目的：一段流程要的身份与角色表对得上：核心席不坐成员角色、成员席不坐核心角色、名字都在册。
/// 参数：`kind` = 报错里点的会话类型（临时拼装是 `custom`）；`cast` = 这段流程要的席位；`seat_of` = 角色表查询。
/// 错误：逐个点名「类型 → 流程 → 角色」。
fn check_cast(
    kind: &str,
    cast: &crate::capabilities::conversation::session::domain::flow::Cast,
    seat_of: &dyn Fn(&str) -> Option<Seat>,
) -> Result<(), String> {
    for role in cast.core {
        match seat_of(role) {
            None => {
                return Err(format!(
                    "会话类型 {} 的流程 {} 引用的角色不在角色表里：{}（systools/roles.yaml）",
                    kind, cast.id, role
                ));
            }
            Some(Seat::Member) => {
                return Err(format!(
                    "会话类型 {} 的流程 {} 把成员席角色 {} 放在核心席（systools/roles.yaml）",
                    kind, cast.id, role
                ));
            }
            Some(Seat::Core) => {}
        }
    }
    for (role, _) in cast.members {
        match seat_of(role) {
            None => {
                return Err(format!(
                    "会话类型 {} 的流程 {} 引用的角色不在角色表里：{}（systools/roles.yaml）",
                    kind, cast.id, role
                ));
            }
            Some(Seat::Core) => {
                return Err(format!(
                    "会话类型 {} 的流程 {} 把核心席角色 {} 放在成员席（systools/roles.yaml）",
                    kind, cast.id, role
                ));
            }
            Some(Seat::Member) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Profiles {
        Profiles {
            profiles: vec![
                Profile {
                    id: SINGLE.to_string(),
                    name: "单机".to_string(),
                    hint: "一个 agent 直接干活。".to_string(),
                    roles: vec!["solo".to_string()],
                    flows: Vec::new(),
                    auto: false,
                },
                Profile {
                    id: MEETING.to_string(),
                    name: "会议".to_string(),
                    hint: "先讨论再执行。".to_string(),
                    roles: Vec::new(),
                    flows: vec!["discuss".to_string(), "execute".to_string()],
                    auto: false,
                },
                Profile {
                    id: PROXY.to_string(),
                    name: "代理".to_string(),
                    hint: "决定权交给核心。".to_string(),
                    roles: vec!["core_proxy".to_string()],
                    flows: Vec::new(),
                    auto: true,
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
            ("orchestrator", Seat::Core),
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
        assert_eq!(collab.flows, vec!["discuss", "execute"]);
        let ids: Vec<String> = p.profiles.iter().map(|x| x.id.clone()).collect();
        assert_eq!(ids, vec![SINGLE, MEETING, PROXY]);
        assert_eq!(p.label_of(MEETING), "会议");
        let spec = p.spec_of(MEETING).expect("会议类型");
        assert_eq!(spec.name, "会议");
        assert_eq!(spec.flow_ids(), vec!["discuss", "execute"]);
        assert_eq!(spec.role, "", "有流程的会话没有静止身份");
        assert_eq!(p.spec_of(SINGLE).expect("单机").role, "solo");
        assert!(p.spec_of("临时").is_none(), "没声明的类型解析不出声明");
        assert_eq!(p.min_agents(&spec.flow_ids()), 2, "讨论至少要两人");
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
        p.profiles[1].flows = vec!["review".to_string()];
        let err = p.validate().unwrap_err();
        assert!(err.contains("流程没注册：review"), "{}", err);
        assert!(
            err.contains("已注册：discuss / execute"),
            "报错要给出已注册清单：{}",
            err
        );

        let mut p = base();
        p.profiles[1].flows.push("discuss".to_string());
        let err = p.validate().unwrap_err();
        assert!(err.contains("同一个流程出现了两次"), "{}", err);
    }

    #[test]
    fn a_type_without_flows_needs_a_standing_role() {
        let mut p = base();
        p.profiles[0].roles.clear();
        let err = p.validate().unwrap_err();
        assert!(err.contains("没有声明身份角色"), "{}", err);
        // 有流程的类型不声明角色也合法：席位由流程自己报。
        let mut p = base();
        p.profiles[1].roles.clear();
        assert_eq!(p.validate(), Ok(()));
    }

    #[test]
    fn seats_must_match_the_role_table() {
        // 成员席角色被角色表标成核心席：装配错误。
        let mut wrong = seats();
        wrong[1] = ("discussant".to_string(), Seat::Core);
        let err = base().check_roles(&wrong).unwrap_err();
        assert!(
            err.contains("把核心席角色 discussant 放在成员席"),
            "{}",
            err
        );

        // 核心席角色被标成成员席：同样是装配错误。
        let mut wrong = seats();
        wrong[3] = ("core_proxy".to_string(), Seat::Member);
        let err = base().check_roles(&wrong).unwrap_err();
        assert!(
            err.contains("把成员席角色 core_proxy 放在核心席"),
            "{}",
            err
        );

        // 角色表里没有这个名字：装配错误。
        let mut missing = seats();
        missing.retain(|(name, _)| name != "orchestrator");
        let err = base().check_roles(&missing).unwrap_err();
        assert!(err.contains("不在角色表里：orchestrator"), "{}", err);
    }

    #[test]
    fn an_inline_assembly_is_checked_against_the_same_whitelist() {
        let p = base();
        // 合法：现拼一段讨论。
        p.check_inline(&["discuss".to_string()], &seats())
            .expect("注册过的流程可以直接拼");

        // 空表：临时拼装至少要有一段流程。
        let err = p.check_inline(&[], &seats()).unwrap_err();
        assert!(err.contains("至少要有一个流程"), "{}", err);

        // 没注册的流程。
        let err = p
            .check_inline(&["review".to_string()], &seats())
            .unwrap_err();
        assert!(err.contains("没注册：review"), "{}", err);

        // 同一段流程拼两次。
        let err = p
            .check_inline(&["discuss".to_string(), "discuss".to_string()], &seats())
            .unwrap_err();
        assert!(err.contains("同一个流程出现了两次"), "{}", err);

        // 席位坐反：讨论席被标成核心席，仍然要拦。
        let mut wrong = seats();
        wrong[1] = ("discussant".to_string(), Seat::Core);
        let err = p
            .check_inline(&["discuss".to_string()], &wrong)
            .unwrap_err();
        assert!(
            err.contains("把核心席角色 discussant 放在成员席"),
            "{}",
            err
        );

        // 折出来的声明：类型记作 custom，没有静止身份，流程原样带上。
        let spec = inline_spec(vec!["discuss".to_string(), "execute".to_string()]);
        assert_eq!(spec.kind, CUSTOM);
        assert_eq!(spec.name, CUSTOM_NAME);
        assert_eq!(spec.role, "");
        assert_eq!(spec.flow_ids(), vec!["discuss", "execute"]);
        assert!(spec.driven());
        assert_eq!(p.min_agents(&spec.flow_ids()), 2);
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
