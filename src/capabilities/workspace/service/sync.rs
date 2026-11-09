//! 目的：工作区的拉取与同步用例——把已提交内容取回沙箱、接管沙箱、把外部目录导入共享区。
//! 管：work_pull、work_adopt_sandbox、work_sync、work_import_root、work_sync_tracked 与过时读判断。
//! 不管：提交怎么落盘（commit.rs）、历史与备份怎么查（history.rs / lifecycle.rs）。
//! 联动：service.rs 的底座与 domain::workstore 的 pull_verdict / status_of。

use super::*;
use crate::capabilities::workspace::api::{
    AdoptReport, Change, ChangeKind, CommitRequest, HeldChange, PullReport, ReadQuery, SyncReport,
};

impl WorkspaceService {
    pub(super) fn work_pull(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<PullReport, String> {
        if paths.is_empty() {
            return Err("work_pull 要给出要拉取的路径（文件或文件夹）".to_string());
        }
        validate_paths(paths)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let external = self.dirs.is_external(work);
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head, tree) = self.head_tree(&store)?;
        let Some(head) = head else {
            return Err("共享区还没有任何提交：没有可拉取的内容".to_string());
        };
        let available: BTreeSet<String> = tree.keys().cloned().collect();
        let (want, missing) = expand_paths(paths, &available);
        let mut index: Index = self.store.read_index(&store, agent)?;
        let mut report = PullReport {
            commit: head,
            missing,
            ..PullReport::default()
        };
        let mut index_dirty = false;
        for rel in want {
            // 逐条收敛：中途失败时，已成功的路径保留在 index 里并落盘，
            // 避免“沙箱已是新版、基线还没推进”在下次上游提交后变成伪冲突。
            let step = (|| -> Result<(), String> {
                let head_hash = tree.get(&rel).cloned().unwrap_or_default();
                if external {
                    // 磁盘是外部工作区的真相：与 head 不一致时绝不拿旧对象覆盖沙箱。
                    if self.disk_hash(&shared, &rel)?.as_deref() != Some(head_hash.as_str()) {
                        report.external.push(rel.clone());
                        return Ok(());
                    }
                }
                let local = self.store.scan_under(&sandbox, &rel, None)?;
                let local_hash = local.as_ref().map(|v| v.hash.clone());
                let base = index.get(&rel).map(String::as_str);
                match pull_verdict(base, Some(&head_hash), local_hash.as_deref()) {
                    PullVerdict::Update => {
                        self.store
                            .copy_object_to_under(&store, &head_hash, &sandbox, &rel)?;
                        index.insert(rel.clone(), head_hash);
                        report.updated.push(rel.clone());
                        index_dirty = true;
                    }
                    PullVerdict::AlreadySame => {
                        index.insert(rel.clone(), head_hash);
                        report.already.push(rel.clone());
                        index_dirty = true;
                    }
                    PullVerdict::NoChange => report.no_change.push(rel.clone()),
                    PullVerdict::Occupied => report.occupied.push(rel.clone()),
                    PullVerdict::Conflict => {
                        // pull 侧能落到 Conflict 的形态：本地删了/上游改了，或两边都改了。
                        let kind = if local_hash.is_none() {
                            Conflict::DeleteVsModify
                        } else {
                            Conflict::BothModified
                        };
                        report.conflicts.push(ConflictAt {
                            path: rel.clone(),
                            kind,
                        });
                    }
                    PullVerdict::Missing => report.missing.push(rel.clone()),
                }
                Ok(())
            })();
            if let Err(e) = step {
                let written = report.updated.len() + report.already.len();
                let mut msg = format!(
                    "拉取中断：已写入/登记 {} 个路径；失败于 {}：{}",
                    written, rel, e
                );
                if index_dirty {
                    match self.store.write_index(&store, agent, &index) {
                        Ok(()) => msg.push_str("；已完成部分的基线已记录，重试同一路径即可"),
                        Err(se) => msg.push_str(&format!(
                            "；已完成部分的基线写入也失败：{}。重试 work_pull 会收敛",
                            se
                        )),
                    }
                } else {
                    msg.push_str("；还没有写入任何文件，重试同一路径即可");
                }
                return Err(msg);
            }
        }
        if let Err(e) = self.store.write_index(&store, agent, &index) {
            return Err(format!(
                "拉取已写入沙箱，但基线写入失败：{}；重试 work_pull 会收敛（已是最新的文件只更新基线）",
                e
            ));
        }
        // 逐路径基线 = 这次拉到的提交号；之后自动提交据此判快进。
        let mut bases = self.agent_bases(&store, agent)?;
        for rel in report.updated.iter().chain(report.already.iter()) {
            bases.insert(rel.clone(), head);
        }
        self.write_agent_bases(&store, agent, head, &bases)?;
        Ok(report)
    }

    pub(super) fn work_adopt_sandbox(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        time: i64,
        line: u64,
    ) -> Result<AdoptReport, String> {
        // 外部工作区的真相是磁盘（work_sync 采纳）；沙箱副本不是它的权威版本。
        if self.dirs.is_external(work) {
            return Ok(AdoptReport::default());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (_, tree) = self.head_tree(&store)?;
        let index: Index = self.store.read_index(&store, agent)?;
        // 候选 = 受管路径 + 沙箱里的全部文件：新增 / 修改 / 删除都在这一遍里认出来。
        let mut all: BTreeSet<String> = index.keys().cloned().collect();
        all.extend(tree.keys().cloned());
        all.extend(self.store.list(&sandbox)?);
        let mut paths: Vec<String> = Vec::new();
        let mut deletes: Vec<String> = Vec::new();
        let mut held: Vec<HeldChange> = Vec::new();
        for rel in all {
            let local = self.store.scan_under(&sandbox, &rel, None)?;
            let local_hash = local.as_ref().map(|v| v.hash.as_str());
            let state = status_of(
                index.get(&rel).map(String::as_str),
                tree.get(&rel).map(String::as_str),
                local_hash,
            );
            match state {
                StatusState::LocalOnly | StatusState::LocalModified => {
                    // 超大文件会让整次提交被拒：单列出来，让剩下的照常进共享区。
                    let size = self.store.stat_under(&sandbox, &rel)?.unwrap_or(0);
                    if size > self.max_asset_bytes {
                        held.push(HeldChange {
                            path: rel,
                            why: format!("超过单文件上限（{} 字节）", self.max_asset_bytes),
                        });
                        continue;
                    }
                    paths.push(rel);
                }
                StatusState::LocalDeleted => deletes.push(rel),
                StatusState::BothPresent => held.push(HeldChange {
                    path: rel,
                    why: "共享区已有这个路径、你的沙箱还没有基线：先 work_pull 再改".to_string(),
                }),
                StatusState::Conflict => held.push(HeldChange {
                    path: rel,
                    why: "共享区先改过、你的沙箱也改了：先 work_pull（冲突用 work_resolve）"
                        .to_string(),
                }),
                _ => {}
            }
        }
        if paths.is_empty() && deletes.is_empty() {
            return Ok(AdoptReport {
                commit: None,
                changed: Vec::new(),
                held,
            });
        }
        let req = CommitRequest {
            paths,
            deletes,
            message: "沙箱改动（shell / 模块工具）".to_string(),
            time,
            line,
        };
        let anchor = CommitAnchor {
            agent: agent.to_string(),
            line,
        };
        let report = self.commit_locked(
            work,
            &shared,
            &store,
            &sandbox,
            agent,
            session,
            &req,
            Source::External,
            Some(anchor),
        )?;
        self.record_committed_bases(&store, agent, &report)?;
        Ok(AdoptReport {
            commit: Some(report.id),
            changed: report.changes.iter().map(|c| c.path.clone()).collect(),
            held,
        })
    }

    pub(super) fn work_stale_reads(
        &self,
        work: &str,
        queries: &[ReadQuery],
    ) -> Result<Vec<bool>, String> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _ = shared;
        let _lock = self.store.lock(&store)?;
        let (_head, tree) = self.head_tree(&store)?;
        let mut out = Vec::with_capacity(queries.len());
        for q in queries {
            // 路径在主副本里没了 = 读过的东西已经不在了。
            let Some(now) = tree.get(&q.path) else {
                out.push(true);
                continue;
            };
            // 指纹相同 = 读的就是现在的版本（文件头插一行不会走到这里，插入会改指纹，
            // 但下面的行区间比较会把"读过的行没变"判成不过期）。
            if *now == q.blob {
                out.push(false);
                continue;
            }
            let old = self.store.read_object(&store, &q.blob).ok();
            let new = self.store.read_object(&store, now).ok();
            match (old, new) {
                (Some(old), Some(new)) => match (String::from_utf8(old), String::from_utf8(new)) {
                    (Ok(old), Ok(new)) => {
                        out.push(lines_changed(&old, &new, q.from, q.to));
                    }
                    // 非 UTF-8（二进制 / 编码坏掉）：没法按行比，按过期处理。
                    _ => out.push(true),
                },
                // 旧对象缺失（被 GC 或从未入库）：宁可供模型重读，不供奉可能错的旧内容。
                _ => out.push(true),
            }
        }
        Ok(out)
    }

    pub(super) fn work_sync(
        &self,
        work: &str,
        paths: &[String],
        time: i64,
    ) -> Result<SyncReport, String> {
        if paths.is_empty() {
            return Err("work_sync 要给出要同步的路径（文件或文件夹）".to_string());
        }
        validate_paths(paths)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let (head_id, tree) = self.head_tree(&store)?;
        // 文件集合 = 工作区磁盘全部文件 + 当前 head；这样“磁盘新增”和“外部删除”都能被展开。
        let disk_files: BTreeSet<String> = self.store.list(&shared)?.into_iter().collect();
        let available: BTreeSet<String> = tree
            .keys()
            .cloned()
            .chain(disk_files.iter().cloned())
            .collect();
        let (want, missing) = expand_paths(paths, &available);
        let mut changes: Vec<Change> = Vec::new();
        let mut unchanged: Vec<String> = Vec::new();
        for rel in want {
            let disk = self.disk_hash(&shared, &rel)?;
            let head = tree.get(&rel).map(String::as_str);
            match (disk.as_deref(), head) {
                (None, None) => {}
                (None, Some(_)) => {
                    changes.push(Change {
                        path: rel,
                        kind: ChangeKind::Delete,
                        hash: String::new(),
                    });
                }
                (Some(d), None) => {
                    changes.push(Change {
                        path: rel,
                        kind: ChangeKind::Add,
                        hash: d.to_string(),
                    });
                }
                (Some(d), Some(h)) if d == h => unchanged.push(rel),
                (Some(d), Some(_)) => {
                    changes.push(Change {
                        path: rel,
                        kind: ChangeKind::Modify,
                        hash: d.to_string(),
                    });
                }
            }
        }
        if changes.is_empty() {
            return Ok(SyncReport {
                commit: None,
                changes,
                unchanged,
                missing,
            });
        }
        for ch in &changes {
            if matches!(ch.kind, ChangeKind::Add | ChangeKind::Modify) {
                self.store.ingest_under(
                    &store,
                    &shared,
                    &ch.path,
                    &ch.hash,
                    self.max_asset_bytes,
                )?;
            }
        }
        let id = self.reserve_id(&store)?;
        let depth = self.next_depth(&store, head_id)?;
        let commit = Commit {
            id,
            parent: head_id,
            base: head_id.unwrap_or(0),
            author: "external".to_string(),
            session: String::new(),
            time,
            message: format!("外部工作区同步：{} 条", changes.len()),
            changes: changes.clone(),
            tree: deltas_tree(&tree, &changes, depth),
            depth,
            source: Source::External,
            anchor: None,
        };
        let pending = Pending::Commit {
            commit: Box::new(commit.clone()),
            index: None,
        };
        self.store.write_pending(&store, &pending)?;
        let result = self.apply_commit_pending(work, &shared, &store, &commit, None);
        self.settle(work, &shared, &store, result)?;
        Ok(SyncReport {
            commit: Some(id),
            changes,
            unchanged,
            missing,
        })
    }

    pub(super) fn work_import_root(&self, work: &str, time: i64) -> Result<SyncReport, String> {
        if !self.dirs.is_external(work) {
            return Ok(SyncReport::default());
        }
        let (shared, store) = self.work_roots(work)?;
        if self.store.head(&store)?.is_some() {
            return Ok(SyncReport::default());
        }
        let mut paths: Vec<String> = Vec::new();
        let mut total: u64 = 0;
        for rel in self.store.list(&shared)? {
            if import_ignored(&rel) {
                continue;
            }
            let size = self.store.stat_under(&shared, &rel)?.unwrap_or(0);
            if size > self.max_asset_bytes {
                continue;
            }
            total = total.saturating_add(size);
            paths.push(rel);
            if paths.len() >= MAX_IMPORT_FILES || total >= MAX_IMPORT_BYTES {
                break;
            }
        }
        if paths.is_empty() {
            return Ok(SyncReport::default());
        }
        self.work_sync(work, &paths, time)
    }

    pub(super) fn work_sync_tracked(&self, work: &str, time: i64) -> Result<SyncReport, String> {
        let (_, store) = self.work_roots(work)?;
        let (_, tree) = self.head_tree(&store)?;
        let paths: Vec<String> = tree.keys().cloned().collect();
        if paths.is_empty() {
            return Ok(SyncReport::default());
        }
        self.work_sync(work, &paths, time)
    }
}
