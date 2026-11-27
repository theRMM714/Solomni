//! 目的：流程段与角色的形状——段（loop）与角色（role）的数据类型、按 id 取值、表自身的装配期规则。
//! 管：`Role` / `Cast` / `Loop` / `Loops` 的形状与 `validate`（角色唯一、角色位在册、流程段与静止段的形状、抑制的能力组认识）。
//! 不管：哪些段串进哪个会话类型（在 `profile.rs`）；段与工具总表的比对（在 `conversation::session` 的 `loop_check`）；yaml 解析（在 `detail/yaml_decls.rs`）。
//! 联动：`systools/loops.yaml` 是唯一声明处；组合根装载进 `SessionDecls`，装配期调 `validate`。

use serde::Deserialize;

use super::artifact::Kind;
use super::tools::{EXEC, FS_READ, FS_WRITE, SHARED_WRITE};

/// 目的：一个角色：提示词段名、是不是核心角色（核心角色坐在主会话里，成员角色由 roster 里的人承担）。
#[derive(Debug, Clone, Deserialize)]
pub struct Role {
    /// 目的：角色 id（`systools/loops.yaml` 的角色位与 `SessionSpec::role` 引用它）。
    pub id: String,
    /// 目的：提示词段名（`roles/<id>` 或复用别的段，如 solo 用 `roles/executor`）。
    pub prompt: String,
    /// 目的：是不是核心角色（核心占主会话，不占名单）。
    #[serde(default)]
    pub core: bool,
}

/// 目的：一段里的一个角色位：谁、至少几个、这一段发它哪些机制动作。
#[derive(Debug, Clone, Deserialize)]
pub struct Cast {
    /// 目的：角色 id（必须在 `roles` 里）。
    pub role: String,
    /// 目的：这一段要几个这样的人（0 = 不占名单；成员角色按它算最少人数）。
    #[serde(default)]
    pub min: usize,
    /// 目的：这一段发给它的机制动作 id（系统工具总表里的名字）。
    #[serde(default)]
    pub tools: Vec<String>,
}

/// 目的：一段流程（或一个静止段）：谁参与、抑制什么、要用户点头什么、吃什么产物吐什么产物。
#[derive(Debug, Clone, Deserialize)]
pub struct Loop {
    /// 目的：段 id（会话类型表的 flows / loop 引用它）。
    pub id: String,
    /// 目的：流程实现名（`discuss` / `execute`）；None = 静止段（没有流程的会话坐它）。
    #[serde(default)]
    pub builder: Option<String>,
    /// 目的：角色位（顺序即名单顺序）。
    #[serde(default)]
    pub cast: Vec<Cast>,
    /// 目的：这一段抑制的能力组（发放与抑制共用 `policy::ids_of` 的映射）。
    #[serde(default)]
    pub deny: Vec<String>,
    /// 目的：这一段要用户点头才执行的动作 id（细到工具，避免把只读查询也变成弹窗）。
    #[serde(default)]
    pub ask: Vec<String>,
    /// 目的：这一段收得下的产物种类（第一段必须收用户需求）。
    #[serde(default)]
    pub accepts: Vec<Kind>,
    /// 目的：这一段交出的产物种类（None = 不交，后面不能再接段）。
    #[serde(default)]
    pub produces: Option<Kind>,
}

/// 目的：流程段与角色的总表（`systools/loops.yaml` 的顶层形状）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Loops {
    /// 目的：全部角色。
    #[serde(default)]
    pub roles: Vec<Role>,
    /// 目的：全部段。
    #[serde(default)]
    pub loops: Vec<Loop>,
}

impl Loops {
    /// 目的：按 id 取段。
    pub fn of(&self, id: &str) -> Option<&Loop> {
        self.loops.iter().find(|l| l.id == id)
    }

    /// 目的：按 id 取角色。
    pub fn role(&self, id: &str) -> Option<&Role> {
        self.roles.iter().find(|r| r.id == id)
    }

    /// 目的：取某段里某个角色的角色位。
    pub fn cast_of(&self, loop_id: &str, role_id: &str) -> Option<&Cast> {
        self.of(loop_id)?.cast.iter().find(|c| c.role == role_id)
    }

    /// 目的：一段的成员角色 id（第一个非核心角色位）：派节点时子会话按它取身份；静止段没有成员角色。
    pub fn member_role(&self, loop_id: &str) -> Option<&str> {
        self.of(loop_id)?
            .cast
            .iter()
            .find(|c| self.role(&c.role).is_some_and(|r| !r.core))
            .map(|c| c.role.as_str())
    }

    /// 目的：这一段开跑至少要几个成员（所有非核心角色位的 min 之和）。
    pub fn min_members(&self, loop_id: &str) -> usize {
        self.of(loop_id)
            .map(|l| {
                l.cast
                    .iter()
                    .filter(|c| self.role(&c.role).is_some_and(|r| !r.core))
                    .map(|c| c.min)
                    .sum()
            })
            .unwrap_or(0)
    }

    /// 目的：注册过的流程段 id（有 builder 的才算流程段），按声明顺序。
    pub fn flow_ids(&self) -> Vec<&str> {
        self.loops
            .iter()
            .filter(|l| l.builder.is_some())
            .map(|l| l.id.as_str())
            .collect()
    }

    /// 目的：装配期校验：角色与段的 id 唯一、角色位在册、流程段有核心与成员、静止段只有一个角色位、抑制的能力组认识。
    /// 返回：`Ok(())` 表示这张表可以拿去装配。
    /// 错误：文案点名出问题的那一项与文件位置。
    pub fn validate(&self) -> Result<(), String> {
        let mut seen: Vec<&str> = Vec::new();
        for r in &self.roles {
            if r.id.trim().is_empty() {
                return Err("角色的 id 是空的（systools/loops.yaml）".to_string());
            }
            if seen.contains(&r.id.as_str()) {
                return Err(format!("角色 id 重复：{}（systools/loops.yaml）", r.id));
            }
            seen.push(&r.id);
            if r.prompt.trim().is_empty() {
                return Err(format!("角色 {} 没声明提示词（systools/loops.yaml）", r.id));
            }
        }
        let mut lids: Vec<&str> = Vec::new();
        for l in &self.loops {
            if l.id.trim().is_empty() {
                return Err("流程段的 id 是空的（systools/loops.yaml）".to_string());
            }
            if lids.contains(&l.id.as_str()) {
                return Err(format!("流程段 id 重复：{}（systools/loops.yaml）", l.id));
            }
            lids.push(&l.id);
            if l.cast.is_empty() {
                return Err(format!(
                    "流程段 {} 一个角色位都没声明（systools/loops.yaml）",
                    l.id
                ));
            }
            let mut has_core = false;
            let mut has_member = false;
            for c in &l.cast {
                match self.role(&c.role) {
                    None => {
                        return Err(format!(
                            "流程段 {} 引用了不存在的角色：{}（systools/loops.yaml）",
                            l.id, c.role
                        ));
                    }
                    Some(r) if r.core => has_core = true,
                    Some(_) => has_member = true,
                }
            }
            if l.builder.is_some() && !(has_core && has_member) {
                return Err(format!(
                    "流程段 {} 要同时有核心角色与成员角色（systools/loops.yaml）",
                    l.id
                ));
            }
            if l.builder.is_none() && l.cast.len() != 1 {
                return Err(format!(
                    "静止段 {} 只能有一个角色位（它是没有流程的会话的静止身份；systools/loops.yaml）",
                    l.id
                ));
            }
            for ability in &l.deny {
                if !matches!(ability.as_str(), FS_READ | FS_WRITE | EXEC | SHARED_WRITE) {
                    return Err(format!(
                        "流程段 {} 抑制了一个不认识的能力组：{}（systools/loops.yaml）",
                        l.id, ability
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles() -> Vec<Role> {
        vec![
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
        ]
    }

    #[test]
    fn a_segment_with_a_builder_needs_a_core_and_a_member() {
        let bad = Loops {
            roles: roles(),
            loops: vec![Loop {
                id: "solo_flow".to_string(),
                builder: Some("x".to_string()),
                cast: vec![Cast {
                    role: "core".to_string(),
                    min: 0,
                    tools: Vec::new(),
                }],
                deny: Vec::new(),
                ask: Vec::new(),
                accepts: Vec::new(),
                produces: None,
            }],
        };
        let err = bad.validate().unwrap_err();
        assert!(err.contains("要同时有核心角色与成员角色"), "{}", err);
    }

    #[test]
    fn a_standing_segment_holds_exactly_one_role() {
        let bad = Loops {
            roles: roles(),
            loops: vec![Loop {
                id: "stand".to_string(),
                builder: None,
                cast: vec![
                    Cast {
                        role: "core".to_string(),
                        min: 0,
                        tools: Vec::new(),
                    },
                    Cast {
                        role: "member".to_string(),
                        min: 1,
                        tools: Vec::new(),
                    },
                ],
                deny: Vec::new(),
                ask: Vec::new(),
                accepts: Vec::new(),
                produces: None,
            }],
        };
        let err = bad.validate().unwrap_err();
        assert!(err.contains("只能有一个角色位"), "{}", err);
    }

    #[test]
    fn an_unknown_ability_group_is_refused() {
        let bad = Loops {
            roles: roles(),
            loops: vec![Loop {
                id: "stand".to_string(),
                builder: None,
                cast: vec![Cast {
                    role: "core".to_string(),
                    min: 0,
                    tools: Vec::new(),
                }],
                deny: vec!["fs-everything".to_string()],
                ask: Vec::new(),
                accepts: Vec::new(),
                produces: None,
            }],
        };
        let err = bad.validate().unwrap_err();
        assert!(err.contains("不认识的能力组"), "{}", err);
    }
}
