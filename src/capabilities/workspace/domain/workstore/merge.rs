//! 目的：提交与拉取的三方比较——改动、冲突与"没有改动"的原因都在这里判。
//! 管：`plan_commit`（提交侧）、`pull_verdict`（拉取侧）、`Conflict` / `Unchanged` / `CommitPlan`。
//! 不管：冲突解决与备份（`resolve.rs`）、状态展示（`status.rs`）。
//! 联动：`workspace/service` 调它算提交计划，`workspace::api` 重导出这些契约类型（docs/workspace/README.md）。

use std::collections::BTreeMap;

use super::*;

/// 目的：冲突的四种形态（回执要能点名"哪种冲突"，不是笼统一句"冲突"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// 两边都改了同一文件，结果不同。
    BothModified,
    /// 两边都新增了同一路径，内容不同。
    BothAdded,
    /// 上游删了、本地改了。
    ModifyVsDelete,
    /// 本地删了、上游改了。
    DeleteVsModify,
    /// 本地要删一个自己没拉取过的路径——没有基线，无法确认删的是哪一版。
    DeleteWithoutBase,
}

impl Conflict {
    pub fn reason(&self) -> &'static str {
        match self {
            Conflict::BothModified => "两边都改了",
            Conflict::BothAdded => "两边都新增了",
            Conflict::ModifyVsDelete => "上游删了、本地改了",
            Conflict::DeleteVsModify => "本地删了、上游改了",
            Conflict::DeleteWithoutBase => "没拉取过，无法确认删除的是哪一版",
        }
    }
}

/// 目的：冲突要能点名是**哪个路径**的哪种冲突（回执据此让 agent 逐条处理）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictAt {
    pub path: String,
    pub kind: Conflict,
}

/// 目的：一条拉取的判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullVerdict {
    /// 把主副本这一版写进沙箱，并更新基线。
    Update,
    /// 沙箱里已经是主副本这一版：只更新基线。
    AlreadySame,
    /// 沙箱里有一个未被跟踪、或与基线无关的本地文件：不覆盖。
    Occupied,
    /// 本地已改、上游也改了：不覆盖，报冲突。
    Conflict,
    /// 主副本没有这个路径。
    Missing,
    /// 什么都没变（上游没动）。
    NoChange,
}

/// 目的：一条路径没有可提交改动的原因（`work_commit` 的空提交回执按它给下一步）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unchanged {
    /// 本地与基线一致（上游也还是基线那一版）。
    SameAsBase,
    /// 本地已经与主副本一致。
    SameAsHead,
    /// 本地未改，但上游已前进：先拉取再提交。
    UpstreamMoved,
    /// 上游已删除，本地还留着基线那一版。
    UpstreamDeleted,
    /// 显式删除的路径，上游已经删了。
    AlreadyDeleted,
}

impl Unchanged {
    pub fn as_str(&self) -> &'static str {
        match self {
            Unchanged::SameAsBase => "本地与基线一致",
            Unchanged::SameAsHead => "已与主副本一致",
            Unchanged::UpstreamMoved => "本地未改，上游已更新",
            Unchanged::UpstreamDeleted => "上游已删除，本地未改",
            Unchanged::AlreadyDeleted => "上游已删除，无需再删",
        }
    }
}

/// 目的：提交计划：要写的变化 + 没有改动的原因（两者合起来覆盖 `locals` 里的全部路径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitPlan {
    pub changes: Vec<Change>,
    pub unchanged: Vec<(String, Unchanged)>,
}

/// 目的：文件级三方比较（提交侧）：`locals` = 路径 → 要写的**内容指纹**（`None` = 显式删除）。
///    Ok(changes) 为空 = 没有可提交的改动，`unchanged` 逐条给出原因；Err = 冲突清单（**整个提交都拒绝**）。
pub fn plan_commit(
    index: &Index,
    tree: &Tree,
    locals: &BTreeMap<String, Option<String>>,
) -> Result<CommitPlan, Vec<ConflictAt>> {
    let mut changes: Vec<Change> = Vec::new();
    let mut unchanged: Vec<(String, Unchanged)> = Vec::new();
    let mut conflicts: Vec<(String, Conflict)> = Vec::new();
    for (path, local) in locals {
        let base = index.get(path).map(String::as_str);
        let head = tree.get(path).map(String::as_str);
        match local {
            None => {
                // 显式删除：必须有基线，且上游没在拉取之后动过。
                match (base, head) {
                    (None, _) => conflicts.push((path.clone(), Conflict::DeleteWithoutBase)),
                    (Some(_), None) => unchanged.push((path.clone(), Unchanged::AlreadyDeleted)),
                    (Some(b), Some(h)) if b == h => changes.push(Change {
                        path: path.clone(),
                        kind: ChangeKind::Delete,
                        hash: String::new(),
                    }),
                    (Some(_), Some(_)) => conflicts.push((path.clone(), Conflict::DeleteVsModify)),
                }
            }
            Some(local_hash) => {
                let local_hash = local_hash.as_str();
                match (base, head) {
                    (None, None) => changes.push(Change {
                        path: path.clone(),
                        kind: ChangeKind::Add,
                        hash: local_hash.to_string(),
                    }),
                    (None, Some(h)) => {
                        if local_hash == h {
                            unchanged.push((path.clone(), Unchanged::SameAsHead));
                        } else {
                            conflicts.push((path.clone(), Conflict::BothAdded));
                        }
                    }
                    (Some(b), None) => {
                        if local_hash == b {
                            unchanged.push((path.clone(), Unchanged::UpstreamDeleted));
                        } else {
                            conflicts.push((path.clone(), Conflict::ModifyVsDelete));
                        }
                    }
                    (Some(b), Some(h)) => {
                        if local_hash == b {
                            if b == h {
                                unchanged.push((path.clone(), Unchanged::SameAsBase));
                            } else {
                                unchanged.push((path.clone(), Unchanged::UpstreamMoved));
                            }
                        } else if local_hash == h {
                            unchanged.push((path.clone(), Unchanged::SameAsHead));
                        } else if b == h {
                            changes.push(Change {
                                path: path.clone(),
                                kind: ChangeKind::Modify,
                                hash: local_hash.to_string(),
                            });
                        } else {
                            conflicts.push((path.clone(), Conflict::BothModified));
                        }
                    }
                }
            }
        }
    }
    if conflicts.is_empty() {
        Ok(CommitPlan { changes, unchanged })
    } else {
        Err(conflicts
            .into_iter()
            .map(|(path, kind)| ConflictAt { path, kind })
            .collect())
    }
}

/// 目的：文件级三方比较（拉取侧）：给哈希（本地文件的内容指纹），返回该路径该怎么处理。
pub fn pull_verdict(base: Option<&str>, head: Option<&str>, local: Option<&str>) -> PullVerdict {
    let Some(head) = head else {
        return PullVerdict::Missing;
    };
    match base {
        None => match local {
            None => PullVerdict::Update,
            Some(l) if l == head => PullVerdict::AlreadySame,
            Some(_) => PullVerdict::Occupied,
        },
        Some(b) => match local {
            // 本地删了自己的副本：上游没动就保持删除，上游动了就没法自动合。
            None => {
                if b == head {
                    PullVerdict::NoChange
                } else {
                    PullVerdict::Conflict
                }
            }
            Some(l) => {
                if l == b {
                    if b == head {
                        PullVerdict::NoChange
                    } else {
                        PullVerdict::Update
                    }
                } else if l == head {
                    PullVerdict::AlreadySame
                } else if b == head {
                    // 上游没动、本地改了：不动本地
                    PullVerdict::NoChange
                } else {
                    PullVerdict::Conflict
                }
            }
        },
    }
}
