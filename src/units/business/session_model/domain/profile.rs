//! 目的：会话类型表——一个会话类型就是一串要跑的流程（或一个静止机制）；装配期的规则校验在这里。
//! 管：`Profiles` / `Profile` / `SessionSpec` 的形状与校验（流程已注册、静止机制在册、人数够、agent 能建哪些）；
//!   会话创建时把选中的类型**解析成** `SessionSpec` 落进 meta，装配只看那份声明。
//! 不管：yaml 解析（在 `detail/yaml_profiles.rs`）；机制与席位的声明（在 `domain/mechanism.rs`）；会话装配（在 `turn` 的重建里）。
//! 联动：组合根装载并在装配期调用 `validate`；创建期按 `min_agents` 校验名单。

use serde::{Deserialize, Serialize};

use crate::units::business::session_model::domain::mechanism::{flow_ids, mechanism_of, seat};

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
/// 目的：核心代理会话坐的静止机制（代理式会话没有流程，坐它唯一的席位 core_proxy）。
/// 约束：产品自带类型表里的代理项声明它；代码按声明里的 `auto` 判定，不认这个名字。
#[cfg(test)]
pub const PROXY_MECHANISM: &str = "proxy";
/// 目的：一条会话落盘的**解析后声明**：哪一类会话、没有流程时坐哪个席位、按顺序跑哪些流程。
/// 约束：装配只看这份声明（不按类型名分岔）；`flows` 空 = 没有调度者，直接轮循环。
///   有流程的会话没有静止席位（`seat` 为空）——身份由流程按回合给席位。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    /// 目的：会话类型 id（命令与落盘用；表里的预设 id，如 single / collab / proxy）。
    pub kind: String,
    /// 目的：会话类型的显示名（界面渲染用，如 单机 / 会议 / 代理）。
    #[serde(default)]
    pub name: String,
    /// 目的：没有流程时的静止**席位** id（由类型声明的静止机制解析出来，如 solo / core_proxy）；
    ///   有流程时为空串（身份由流程按回合给席位）。
    #[serde(default)]
    pub seat: String,
    /// 目的：按顺序串起来的流程 id；空表示没有人调度，直接轮循环。
    #[serde(default)]
    pub flows: Vec<String>,
    /// 目的：整块交给核心代理（没有名单、没有本次需求，核心在该会话里工作并自己决定做什么）。
    /// 约束：由会话类型声明，不按类型 id 或机制名判定；`true` 的类型不得再声明流程。
    #[serde(default)]
    pub auto: bool,
}

impl SessionSpec {
    /// 目的：造一条没有流程的声明（单 agent 会话；代理式再用 `automatic` 打开 `auto`）。
    pub fn plain(kind: &str, name: &str, seat: &str) -> SessionSpec {
        SessionSpec {
            kind: kind.to_string(),
            name: name.to_string(),
            seat: seat.to_string(),
            flows: Vec::new(),
            auto: false,
        }
    }

    /// 目的：造一条**整块交给核心**的声明（代理式会话：没有流程、没有静止需求）。
    pub fn automatic(kind: &str, name: &str, seat: &str) -> SessionSpec {
        SessionSpec {
            auto: true,
            ..SessionSpec::plain(kind, name, seat)
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

/// 目的：一个会话类型：一串要跑的流程（空 = 没有流程，直接轮循环）与它的静止机制。
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
    /// 目的：没有流程时的静止机制 id；有流程时不用声明（席位由机制按回合发放）。
    #[serde(default)]
    pub mechanism: String,
    /// 目的：按顺序串起来的流程 id；空表示没有人调度，直接轮循环。
    #[serde(default)]
    pub flows: Vec<String>,
    /// 目的：整块交给核心代理的类型（没有名单、没有需求；核心在该会话里自己决定做什么）。
    #[serde(default)]
    pub auto: bool,
    /// 目的：这个类型能不能由 **agent** 建（`create_session` 的 kind 可选它）。
    /// 约束：默认 false——没标 = 只有人经呈现层能建；人的入口不受它限制。
    #[serde(default)]
    pub agent_visible: bool,
}

/// 目的：会话类型总表（`systools/profiles.yaml` 的顶层形状）。
#[derive(Debug, Clone, Deserialize)]
pub struct Profiles {
    /// 目的：全部会话类型。
    pub profiles: Vec<Profile>,
}

impl Profiles {
    /// 目的：把一个类型解析成落盘声明；没有这个类型返回 `None`。
    /// 约束：有流程的会话 `seat` 取空串（静止席位对流程会话没有意义）；没有流程时取类型声明的静止机制唯一的那一席。
    pub fn spec_of(&self, kind: &str) -> Option<SessionSpec> {
        self.profiles
            .iter()
            .find(|p| p.id == kind)
            .map(|p| SessionSpec {
                kind: p.id.clone(),
                name: p.name.clone(),
                seat: if p.flows.is_empty() {
                    mechanism_of(&p.mechanism)
                        .and_then(|m| m.seats.first().copied())
                        .unwrap_or_default()
                        .to_string()
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

    /// 目的：这个类型是不是表里的一个预设。
    pub fn known(&self, kind: &str) -> bool {
        self.profiles.iter().any(|p| p.id == kind)
    }

    /// 目的：这一串流程开跑至少要有几个 agent（各段流程要的人数取最大值——每段都要在自己的会话里跑得起来）。
    pub fn min_agents(&self, flows: &[String]) -> usize {
        flows
            .iter()
            .filter_map(|id| mechanism_of(id))
            .map(|m| {
                m.seats
                    .iter()
                    .filter_map(|id| seat(id))
                    .filter(|s| !s.core)
                    .map(|s| s.min)
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
    }

    /// 目的：装配期校验：类型 id 唯一且无空白、流程都注册过且不重复、没有流程的类型要有静止机制。
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
            if p.flows.is_empty() {
                if p.mechanism.trim().is_empty() {
                    return Err(format!(
                        "会话类型 {} 没有声明静止机制（没有流程的会话要坐一席静止机制；systools/profiles.yaml）",
                        p.id
                    ));
                }
                let Some(m) = mechanism_of(&p.mechanism) else {
                    return Err(format!(
                        "会话类型 {} 的静止机制没声明：{}（systools/profiles.yaml）",
                        p.id, p.mechanism
                    ));
                };
                if m.flow {
                    return Err(format!(
                        "会话类型 {} 的静止机制不能是流程机制：{}（systools/profiles.yaml）",
                        p.id, p.mechanism
                    ));
                }
            } else if !p.mechanism.trim().is_empty() {
                return Err(format!(
                    "会话类型 {} 有流程，不该再声明静止机制：{}（systools/profiles.yaml）",
                    p.id, p.mechanism
                ));
            }
            let mut flows: Vec<&str> = Vec::new();
            for id in &p.flows {
                if mechanism_of(id).is_none() {
                    return Err(format!(
                        "会话类型 {} 的流程没注册：{}（已注册：{}；systools/profiles.yaml）",
                        p.id,
                        id,
                        flow_ids().join(" / ")
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
                    mechanism: "standard".to_string(),
                    flows: Vec::new(),
                    auto: false,
                    agent_visible: true,
                },
                Profile {
                    id: MEETING.to_string(),
                    name: "会议".to_string(),
                    hint: "先讨论再执行。".to_string(),
                    mechanism: String::new(),
                    flows: vec!["discuss".to_string(), "execute".to_string()],
                    auto: false,
                    agent_visible: true,
                },
                Profile {
                    id: PROXY.to_string(),
                    name: "代理".to_string(),
                    hint: "决定权交给核心。".to_string(),
                    mechanism: PROXY_MECHANISM.to_string(),
                    flows: Vec::new(),
                    auto: true,
                    agent_visible: false,
                },
            ],
        }
    }

    #[test]
    fn the_shape_it_documents_validates() {
        let p = base();
        assert_eq!(p.validate(), Ok(()), "基准表必须合法");
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
        assert_eq!(spec.seat, "", "有流程的会话没有静止席位");
        assert_eq!(p.spec_of(SINGLE).expect("单机").seat, "solo");
        assert_eq!(p.spec_of(PROXY).expect("代理").seat, "core_proxy");
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
    fn a_type_without_flows_needs_a_standing_mechanism() {
        // 没有流程又不声明静止机制：不知道坐哪一席。
        let mut p = base();
        p.profiles[0].mechanism.clear();
        let err = p.validate().unwrap_err();
        assert!(err.contains("没有声明静止机制"), "{}", err);

        // 声明了流程机制：流程机制要串进会话才有席位，不能当静止机制。
        let mut p = base();
        p.profiles[0].mechanism = "execute".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("不能是流程机制：execute"), "{}", err);

        // 声明了没登记的机制 id。
        let mut p = base();
        p.profiles[0].mechanism = "ghost".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("静止机制没声明：ghost"), "{}", err);

        // 有流程又声明静止机制：两套身份来源，装配不知道该听谁。
        let mut p = base();
        p.profiles[1].mechanism = "standard".to_string();
        let err = p.validate().unwrap_err();
        assert!(err.contains("有流程，不该再声明静止机制"), "{}", err);
    }

    #[test]
    fn only_the_types_marked_visible_are_open_to_agents() {
        let p = base();
        let ids: Vec<&str> = p
            .profiles
            .iter()
            .filter(|x| x.agent_visible)
            .map(|x| x.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![SINGLE, MEETING],
            "没标 agent_visible 的类型不给 agent 建"
        );
        assert!(
            p.known(PROXY),
            "不可见不等于不存在：人的入口按 known 判，仍能建它"
        );
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
