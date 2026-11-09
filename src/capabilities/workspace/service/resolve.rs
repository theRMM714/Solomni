//! 目的：冲突解决的用例与它专用的计划结构——按 agent 选边生成新提交。
//! 管：work_resolve、ResolvePlan、解决被拒的回执渲染。
//! 不管：三方比较规则在 domain::workstore（local_take_verdict / needs_backup），落盘在 commit.rs。
//! 联动：service.rs 的锁与物化、commit.rs 的 commit_locked 与 base 记录。

use super::*;
use crate::capabilities::workspace::api::{
    ResolveAction, ResolveEntry, ResolveReport, ResolveRequest, ResolveTake, ResolveWarning,
    ResolveWarningKind,
};

/// 把 take=local 的拒绝清单渲染成模型侧回执：整次不做，逐条点名。
fn render_resolve_denied(paths: &[String]) -> String {
    let mut out = String::from(
        "解决被拒：以下路径的本地内容与基线一致（你没有改过它），take=local 不会包含主副本的新改动：",
    );
    for path in paths.iter().take(50) {
        out.push_str(&format!("\n- {}", path));
    }
    if paths.len() > 50 {
        out.push_str(&format!("\n…（共 {} 条）", paths.len()));
    }
    out.push_str(
        "\n先用 work_versions 看基线 / 主副本 / 沙箱三方：要接受主副本用 take=upstream；要保留本地，请先把主副本内容合进沙箱，再带 head=<主副本提交 id> 调 take=local。\n本次没有改动文件或基线。",
    );
    out
}

/// 一条待执行的 resolve 计划：先把三方看全、把要备份的备份完，再统一改文件/基线。
struct ResolvePlan {
    path: String,
    head_hash: Option<String>,
    base_hash: Option<String>,
    local_hash: Option<String>,
    /// 覆盖/删除前要写入的备份相对路径（版本库内）；不需要备份 = None。
    backup: Option<String>,
}

impl WorkspaceService {
    pub(super) fn work_resolve(
        &self,
        work: &str,
        agent: &str,
        req: &ResolveRequest,
    ) -> Result<ResolveReport, String> {
        if req.paths.is_empty() {
            return Err("work_resolve 要给出要解决的路径（文件或文件夹）".to_string());
        }
        validate_paths(&req.paths)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head_id, tree) = self.head_tree(&store)?;
        let mut index: Index = self.store.read_index(&store, agent)?;
        let sandbox_files: BTreeSet<String> = self.store.list(&sandbox)?.into_iter().collect();
        let available: BTreeSet<String> = index
            .keys()
            .chain(tree.keys())
            .chain(sandbox_files.iter())
            .cloned()
            .collect();
        let (want, missing) = expand_paths(&req.paths, &available);
        if want.is_empty() {
            return Err(format!(
                "这些路径在基线、主副本与沙箱里都没有：{}",
                missing.join("、")
            ));
        }
        if matches!(req.take, ResolveTake::Local) {
            if let Some(current) = head_id {
                match req.expect_head {
                    Some(expected) if expected == current => {}
                    Some(expected) => {
                        return Err(format!(
                            "主副本已从提交 {} 变为 {}：你查看后主副本又前进了，不能把旧合并结果登记到新版本。先重新 work_versions 看三方、把新版合进沙箱，再带 head={} 调 work_resolve。",
                            expected, current, current
                        ))
                    }
                    None => {
                        return Err(format!(
                            "take=local 要带 head=<主副本提交 id>（先 work_status 或 work_versions 取，当前是 {}）；主副本在你查看之后可能已变化，resolve 不能替你猜。",
                            current
                        ))
                    }
                }
            }
        }
        let mut plans: Vec<ResolvePlan> = Vec::new();
        let mut denied: Vec<String> = Vec::new();
        let mut warnings: Vec<ResolveWarning> = Vec::new();
        for rel in want {
            let head_hash = tree.get(&rel).cloned();
            let base_hash = index.get(&rel).cloned();
            let local = self.store.scan_under(&sandbox, &rel, None)?;
            let local_hash = local.as_ref().map(|v| v.hash.clone());
            let backup = if matches!(req.take, ResolveTake::Upstream)
                && needs_backup(head_hash.as_deref(), local_hash.as_deref())
            {
                Some(lost_rel(agent, &rel)?)
            } else {
                None
            };
            if matches!(req.take, ResolveTake::Local) {
                match local_take_verdict(
                    base_hash.as_deref(),
                    head_hash.as_deref(),
                    local_hash.as_deref(),
                ) {
                    LocalTakeVerdict::DenyUnchanged => denied.push(rel.clone()),
                    LocalTakeVerdict::WarnLocalMissing => warnings.push(ResolveWarning {
                        path: rel.clone(),
                        kind: ResolveWarningKind::LocalMissing,
                    }),
                    LocalTakeVerdict::WarnNoBase => warnings.push(ResolveWarning {
                        path: rel.clone(),
                        kind: ResolveWarningKind::NoBase,
                    }),
                    LocalTakeVerdict::Ok => {}
                }
            }
            plans.push(ResolvePlan {
                path: rel,
                head_hash,
                base_hash,
                local_hash,
                backup,
            });
        }
        if !denied.is_empty() {
            return Err(render_resolve_denied(&denied));
        }
        // 先把所有要覆盖/删除的本地版本备份完，再做任何改动；备份失败就整次拒绝。
        let backup_root = lost_root(&store, agent)?;
        for plan in &plans {
            if plan.backup.is_some() {
                let hash = plan
                    .local_hash
                    .as_deref()
                    .ok_or_else(|| format!("内部错误：{} 需要备份但没有本地指纹", plan.path))?;
                self.store.copy_under_to_under(
                    &sandbox,
                    &plan.path,
                    &backup_root,
                    &plan.path,
                    hash,
                )?;
            }
        }
        let mut entries: Vec<ResolveEntry> = Vec::new();
        let mut changed = false;
        let mut done = 0usize;
        let apply = (|| -> Result<(), (String, String)> {
            for plan in plans {
                let ResolvePlan {
                    path: rel,
                    head_hash,
                    base_hash,
                    local_hash,
                    backup,
                } = plan;
                let action = match req.take {
                    ResolveTake::Upstream => match head_hash.as_deref() {
                        Some(hash) => {
                            let same = local_hash.as_deref() == Some(hash);
                            if !same {
                                self.store
                                    .copy_object_to_under(&store, hash, &sandbox, &rel)
                                    .map_err(|e| (rel.clone(), e))?;
                            }
                            let base_changed = base_hash.as_deref() != Some(hash);
                            if base_changed {
                                index.insert(rel.clone(), hash.to_string());
                                changed = true;
                            }
                            if !same {
                                ResolveAction::WroteUpstream
                            } else if base_changed {
                                ResolveAction::Marked
                            } else {
                                ResolveAction::NoChange
                            }
                        }
                        None => {
                            let had_local = local_hash.is_some();
                            if had_local {
                                self.store
                                    .remove_under(&sandbox, &rel)
                                    .map_err(|e| (rel.clone(), e))?;
                            }
                            let had_base = index.remove(&rel).is_some();
                            changed |= had_local || had_base;
                            if had_local {
                                ResolveAction::RemovedLocal
                            } else if had_base {
                                ResolveAction::Cleared
                            } else {
                                ResolveAction::NoChange
                            }
                        }
                    },
                    ResolveTake::Local => match head_hash.as_deref() {
                        Some(hash) => {
                            let base_changed = base_hash.as_deref() != Some(hash);
                            if base_changed {
                                index.insert(rel.clone(), hash.to_string());
                                changed = true;
                                ResolveAction::Marked
                            } else {
                                ResolveAction::NoChange
                            }
                        }
                        None => {
                            let had_base = index.remove(&rel).is_some();
                            changed |= had_base;
                            if had_base {
                                ResolveAction::Cleared
                            } else {
                                ResolveAction::NoChange
                            }
                        }
                    },
                };
                entries.push(ResolveEntry {
                    path: rel,
                    action,
                    backup,
                });
                done += 1;
            }
            Ok(())
        })();
        if let Err((rel, e)) = apply {
            return Err(format!(
                "解决中断：已处理 {} 条；失败于 {}：{}；已覆盖的文件可用 work_pull 对齐，已备份的本地版本在版本库 lost/ 下（可用 work_lost 找回）。",
                done, rel, e
            ));
        }
        if changed {
            if let Err(e) = self.store.write_index(&store, agent, &index) {
                return Err(format!(
                    "解决已完成，但基线写入失败：{}；重试 work_pull 或 work_resolve 会补上基线。",
                    e
                ));
            }
        }
        Ok(ResolveReport {
            take: req.take,
            head: head_id,
            entries,
            missing,
            warnings,
        })
    }
}
