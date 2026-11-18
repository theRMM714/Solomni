//! 目的：四档 profile 的完整预设——配置界面照抄的那份事实。
//! 管：`ProfileId` / `ProfileSpec` / `Decisions` 与档位标签、摘要、各 effect 的默认处置。
//! 不管：用户覆盖与子会话收窄（`layer.rs` / `set.rs`）；共享区范围（`scope.rs`）。
//! 联动：`PermissionSet` 的 `profile` 字段与设置视图（docs/permission/README.md）。

use serde::{Deserialize, Serialize};

use super::*;

/// 目的：四档用户可切换的会话权限预设（名称是呈现层文案，id 是契约）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProfileId {
    /// 目的：仅可查看。
    View,
    /// 目的：工作区内修改（默认）。
    #[default]
    Workspace,
    /// 目的：外部可达（逐次确认）。
    External,
    /// 目的：全可达（自动放行）。
    AllReach,
}

impl ProfileId {
    /// 目的：档位宽度排序（收窄校验用；高不能换低）。
    pub fn rank(self) -> u8 {
        match self {
            ProfileId::View => 0,
            ProfileId::Workspace => 1,
            ProfileId::External => 2,
            ProfileId::AllReach => 3,
        }
    }

    /// 目的：稳定 id（配置、状态栏与审计）。
    pub fn as_str(self) -> &'static str {
        match self {
            ProfileId::View => "view",
            ProfileId::Workspace => "workspace",
            ProfileId::External => "external",
            ProfileId::AllReach => "all_reach",
        }
    }

    /// 目的：用户标签。
    pub fn label(self) -> &'static str {
        match self {
            ProfileId::View => "仅可查看",
            ProfileId::Workspace => "工作区内修改",
            ProfileId::External => "外部可达（逐次确认）",
            ProfileId::AllReach => "全可达（自动放行）",
        }
    }

    /// 目的：一句话说明这个档位能做什么。
    pub fn summary(self) -> &'static str {
        match self {
            ProfileId::View => "读共享区与模块目录；写、提交与执行类工具都关闭。",
            ProfileId::Workspace => "可写自己的沙箱、提交共享区；执行类工具只能碰私有沙箱。",
            ProfileId::External => "在上一档之上可读主机只读根、可出网；越出沙箱的执行每次确认。",
            ProfileId::AllReach => {
                "reach 有多宽就授予多宽；仍受全局网络 / 主机根 / 黑名单与围栏上限。"
            }
        }
    }

    /// 目的：从配置串解析档位（只认四个 id）。
    pub fn parse(raw: &str) -> Result<ProfileId, String> {
        match raw.trim() {
            "view" => Ok(ProfileId::View),
            "workspace" => Ok(ProfileId::Workspace),
            "external" => Ok(ProfileId::External),
            "all_reach" => Ok(ProfileId::AllReach),
            other => Err(format!(
                "未知权限档位：{}（只认 view / workspace / external / all_reach）",
                other
            )),
        }
    }

    /// 目的：四档按从窄到宽排列（配置界面与校验共用）。
    pub fn all() -> [ProfileId; 4] {
        [
            ProfileId::View,
            ProfileId::Workspace,
            ProfileId::External,
            ProfileId::AllReach,
        ]
    }

    /// 目的：这个档位的 reach 与 decision 预设。
    pub fn spec(self) -> ProfileSpec {
        let (reach, decisions) = match self {
            ProfileId::View => (
                ReachSet::from([Reach::Private, Reach::SharedRead, Reach::ModuleRead]),
                Decisions {
                    read: Decision::Allow,
                    write: Decision::Deny,
                    exec_sandbox: Decision::Deny,
                    exec_external: Decision::Deny,
                },
            ),
            ProfileId::Workspace => (
                ReachSet::from([
                    Reach::Private,
                    Reach::SharedRead,
                    Reach::SharedWrite,
                    Reach::ModuleRead,
                    Reach::ModuleWrite,
                ]),
                Decisions {
                    read: Decision::Allow,
                    write: Decision::Allow,
                    exec_sandbox: Decision::Allow,
                    exec_external: Decision::Deny,
                },
            ),
            ProfileId::External => (
                ReachSet::from([
                    Reach::Private,
                    Reach::SharedRead,
                    Reach::SharedWrite,
                    Reach::ModuleRead,
                    Reach::ModuleWrite,
                    Reach::HostRead,
                    Reach::Network,
                ]),
                Decisions {
                    read: Decision::Allow,
                    write: Decision::Allow,
                    exec_sandbox: Decision::Allow,
                    exec_external: Decision::Ask,
                },
            ),
            ProfileId::AllReach => (
                ReachSet::from([
                    Reach::Private,
                    Reach::SharedRead,
                    Reach::SharedWrite,
                    Reach::ModuleRead,
                    Reach::ModuleWrite,
                    Reach::HostRead,
                    Reach::Network,
                ]),
                Decisions {
                    read: Decision::Allow,
                    write: Decision::Allow,
                    exec_sandbox: Decision::Allow,
                    exec_external: Decision::Allow,
                },
            ),
        };
        ProfileSpec {
            id: self,
            label: self.label(),
            summary: self.summary(),
            reach,
            decisions,
        }
    }
}

/// 目的：四档 profile 的完整预设（配置界面直接照抄，不在前端另写一份）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileSpec {
    /// 目的：档位 id。
    pub id: ProfileId,
    /// 目的：用户标签。
    pub label: &'static str,
    /// 目的：一句话说明。
    pub summary: &'static str,
    /// 目的：这一档授予的 reach。
    pub reach: ReachSet,
    /// 目的：这一档的默认 decision。
    pub decisions: Decisions,
}

/// 目的：按 effect / scope 分流的有效处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Decisions {
    /// 目的：读类。
    pub read: Decision,
    /// 目的：写类。
    pub write: Decision,
    /// 目的：执行类、sandbox scope。
    pub exec_sandbox: Decision,
    /// 目的：执行类、external scope。
    pub exec_external: Decision,
}

impl Decisions {
    /// 目的：逐字段取更严的一边。
    pub fn stricter(self, other: Decisions) -> Decisions {
        Decisions {
            read: self.read.stricter(other.read),
            write: self.write.stricter(other.write),
            exec_sandbox: self.exec_sandbox.stricter(other.exec_sandbox),
            exec_external: self.exec_external.stricter(other.exec_external),
        }
    }

    /// 目的：应用一层显式覆盖；narrowing 为真时不允许放宽。
    pub(super) fn with_edit(
        self,
        edit: &DecisionEdit,
        narrowing: bool,
    ) -> Result<Decisions, String> {
        let mut out = self;
        let apply =
            |label: &str, value: Option<Decision>, slot: &mut Decision| -> Result<(), String> {
                if let Some(v) = value {
                    if narrowing && v.rank() > slot.rank() {
                        return Err(format!("子会话的 {} 决定只能更严", label));
                    }
                    *slot = v;
                }
                Ok(())
            };
        apply("read", edit.read, &mut out.read)?;
        apply("write", edit.write, &mut out.write)?;
        apply("exec_sandbox", edit.exec_sandbox, &mut out.exec_sandbox)?;
        apply("exec_external", edit.exec_external, &mut out.exec_external)?;
        Ok(out)
    }
}
