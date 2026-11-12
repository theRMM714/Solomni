//! 目的：工作区的生命周期用例——丢失文件、回收、还原、归档与删除。
//! 管：work_lost / restore_lost / recover / gc / restore(_force) / head / rewind_to / archive / delete / archives / restore_archive / restore_point / discard_after。
//! 不管：常规提交（commit.rs）、拉取同步（sync.rs）。
//! 联动：service.rs 的锁与树物化、commit.rs 的归档标记与锚点。

use super::*;
use crate::capabilities::workspace::api::{
    ArchiveReport, DeleteReport, GcReport, LostEntry, LostReport, RestoreLostReport,
};

impl WorkspaceService {
    pub(super) fn work_lost(
        &self,
        work: &str,
        agent: &str,
        path: Option<&str>,
    ) -> Result<LostReport, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (_shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &_shared, &store)?;
        let lost = lost_root(&store, agent)?;
        let filter = match path {
            Some(p) => Some(clean_rel(p)?),
            None => None,
        };
        let mut rels = self.store.list(&lost)?;
        rels.retain(|r| !is_lost_temp(r));
        if let Some(f) = &filter {
            rels.retain(|r| r == f || r.starts_with(&format!("{}/", f)));
        }
        let total = rels.len();
        let truncated = total > MAX_LOST;
        rels.truncate(MAX_LOST);
        let mut entries = Vec::new();
        for rel in rels {
            let Some(v) = self.store.scan_under(&lost, &rel, None)? else {
                continue;
            };
            let current = self.store.scan_under(&sandbox, &rel, None)?;
            entries.push(LostEntry {
                path: rel,
                hash: v.hash,
                size: v.size,
                current_hash: current.map(|c| c.hash),
            });
        }
        Ok(LostReport {
            agent: agent.to_string(),
            total,
            entries,
            truncated,
        })
    }

    pub(super) fn work_restore_lost(
        &self,
        work: &str,
        agent: &str,
        path: &str,
        expect_current: &str,
    ) -> Result<RestoreLostReport, String> {
        let rel = clean_rel(path)?;
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (_shared, store, sandbox) = self.agent_roots(work, agent)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &_shared, &store)?;
        let lost = lost_root(&store, agent)?;
        let backups = self.store.list(&lost)?;
        if !backups.iter().any(|p| p == &rel) {
            return Err(format!(
                "没有这个被覆盖的本地版本：{}（先用 work_lost 看可恢复清单）",
                rel
            ));
        }
        let Some(backup) = self.store.scan_under(&lost, &rel, None)? else {
            return Err(format!("备份文件不存在：{}", rel));
        };
        let current = self.store.scan_under(&sandbox, &rel, None)?;
        let current_hash = current.as_ref().map(|v| v.hash.clone());
        let matches = match current_hash.as_deref() {
            None => expect_current.is_empty(),
            Some(hash) => hash == expect_current,
        };
        if !matches {
            return Err(format!(
                "当前沙箱内容与 expect_current 不符：当前={}；先 work_lost 重新看当前 hash，确认后再恢复。",
                current_hash.as_deref().unwrap_or("不存在")
            ));
        }
        let size = self
            .store
            .copy_under_to_under(&lost, &rel, &sandbox, &rel, &backup.hash)?;
        Ok(RestoreLostReport {
            path: rel,
            hash: backup.hash,
            size,
            previous: current_hash,
        })
    }

    pub(super) fn work_recover(&self, work: &str) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)
    }

    pub(super) fn work_gc(&self, work: &str, apply: bool) -> Result<GcReport, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;

        // 先让旧布局的 head 升级成 refs/main，再只从引用出发算活集合：
        // 引用指向的提交 + 祖先链；不在这条集合里的提交与对象都可回收。
        let _ = self.store.head(&store)?;
        let refs = self.store.list_refs(&store)?;
        let mut stack: Vec<u64> = Vec::new();
        for r in refs.values() {
            stack.extend(r.target);
            stack.extend(r.heads.values().flatten().copied());
        }
        let mut live: BTreeSet<u64> = BTreeSet::new();
        let mut reachable: BTreeSet<String> = BTreeSet::new();
        while let Some(id) = stack.pop() {
            if !live.insert(id) {
                continue;
            }
            let Some(commit) = self.store.read_commit(&store, id)? else {
                continue;
            };
            for ch in &commit.changes {
                if !ch.hash.is_empty() {
                    reachable.insert(ch.hash.clone());
                }
            }
            if let Some(tree) = &commit.tree {
                reachable.extend(tree.values().cloned());
            }
            if let Some(parent) = commit.parent {
                stack.push(parent);
            }
        }
        // 只被基线引用的对象也必须保留。
        for (_, index) in self.store.list_indexes(&store)? {
            reachable.extend(index.values().cloned());
        }

        let mut report = GcReport {
            applied: apply,
            ..GcReport::default()
        };
        for object in self.store.list_objects(&store)? {
            if !is_content_hash(&object.hash) {
                report.ignored_files += 1;
                if apply {
                    self.store
                        .remove_object(&store, &object.hash)
                        .map_err(|e| format!("清临时文件 {} 失败：{}", object.hash, e))?;
                    report.removed_ignored += 1;
                }
                continue;
            }
            report.objects += 1;
            report.bytes += object.size;
            if reachable.contains(&object.hash) {
                report.reachable_objects += 1;
                report.reachable_bytes += object.size;
            } else {
                report.unreachable_objects += 1;
                report.unreachable_bytes += object.size;
                if apply {
                    self.store
                        .remove_object(&store, &object.hash)
                        .map_err(|e| format!("回收对象 {} 失败：{}", object.hash, e))?;
                    report.removed_objects += 1;
                    report.removed_bytes += object.size;
                }
            }
        }
        if apply {
            let ids = self.store.list_commits(&store)?;
            for id in ids {
                if !live.contains(&id) {
                    self.store.remove_commit(&store, id)?;
                    report.removed_commits += 1;
                }
            }
        }
        Ok(report)
    }

    pub(super) fn work_restore(&self, work: &str, commit: u64) -> Result<(), String> {
        self.restore_impl(work, commit, false)
    }

    pub(super) fn work_restore_force(&self, work: &str, commit: u64) -> Result<(), String> {
        self.restore_impl(work, commit, true)
    }

    pub(super) fn work_head(&self, work: &str) -> Result<Option<u64>, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        self.store.head(&store)
    }

    #[cfg(test)]
    pub(super) fn work_rewind_to(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
    ) -> Result<Option<u64>, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        // 只沿**当前 head 的祖先链**选点：用户回滚离开过的提交仍在库里，但不在当前线上，
        // 不能被后续转录回档误选回来。
        let head = self.store.head(&store)?;
        let target = self.archive_point(&store, head, keep_by_agent)?;
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, target)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "回档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本；强制覆盖需要用户危险确认。",
                conflicts.join("、")
            ));
        }
        self.pin_head(&store)?;
        let pending = Pending::Restore { target };
        self.store.write_pending(&store, &pending)?;
        let result = WorkspaceService::materialize(self.store.as_ref(), &shared, &store, target);
        self.settle(work, &shared, &store, result)?;
        Ok(target)
    }

    pub(super) fn work_archive(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
        agents: &[String],
        mark: u64,
    ) -> Result<ArchiveReport, String> {
        if mark == 0 {
            return Err("档案号不能是 0".to_string());
        }
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let name = format!("archive/{}", mark);
        if self.store.read_ref(&store, &name)?.is_some() {
            return Err(format!("档案 {} 已存在", mark));
        }
        let head = self.store.head(&store)?;
        let target = self.archive_point(&store, head, keep_by_agent)?;
        let tree = match target {
            Some(id) => Self::tree_of(self.store.as_ref(), &store, id)?,
            None => Tree::new(),
        };
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, target)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "留档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本。",
                conflicts.join("、")
            ));
        }
        let mut uniq: BTreeSet<String> = BTreeSet::new();
        for agent in agents {
            if !agent.is_empty() {
                uniq.insert(agent.clone());
            }
        }
        // 引用先落：它记着留档前的主线；此时文件还没动，失败可以整次撤销。
        let mut heads: BTreeMap<String, Option<u64>> = BTreeMap::new();
        for agent in &uniq {
            heads.insert(agent.clone(), head);
        }
        self.store.write_ref(
            &store,
            &name,
            &Ref {
                kind: RefKind::Archive,
                target: head,
                heads: heads.clone(),
            },
        )?;
        let mut backed_up: BTreeMap<String, usize> = BTreeMap::new();
        let result = (|| -> Result<(), String> {
            for agent in &uniq {
                let (_, _, sandbox) = self.agent_roots(work, agent)?;
                let prefix = format!("archive-{}", mark);
                let n = self.lay_down_tree(&store, &sandbox, &tree, agent, Some(&prefix))?;
                self.reset_agent_point(&store, agent, target, &tree)?;
                backed_up.insert(agent.clone(), n);
            }
            let pending = Pending::Restore { target };
            self.store.write_pending(&store, &pending)?;
            let result =
                WorkspaceService::materialize(self.store.as_ref(), &shared, &store, target);
            self.settle(work, &shared, &store, result)
        })();
        if result.is_err() {
            let _ = self.store.remove_ref(&store, &name);
        }
        result?;
        Ok(ArchiveReport {
            mark,
            shared: head,
            point: target,
            backed_up,
        })
    }

    pub(super) fn work_delete(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
        agents: &[String],
        keep_marks: &[u64],
    ) -> Result<DeleteReport, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let head = self.store.head(&store)?;
        let target = self.archive_point(&store, head, keep_by_agent)?;
        let tree = match target {
            Some(id) => Self::tree_of(self.store.as_ref(), &store, id)?,
            None => Tree::new(),
        };
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, target)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "删除会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本；强制覆盖需要用户危险确认。",
                conflicts.join("、")
            ));
        }
        let mut uniq: BTreeSet<String> = BTreeSet::new();
        for agent in agents {
            if !agent.is_empty() {
                uniq.insert(agent.clone());
            }
        }
        // 先落恢复日志：中途崩了，下一次操作会按同一份名单把沙箱与主副本一起补完。
        let pending = Pending::Discard {
            keep: target,
            agents: uniq.iter().cloned().collect(),
        };
        self.store.write_pending(&store, &pending)?;
        let mut backed_up: BTreeMap<String, usize> = BTreeMap::new();
        let result = (|| -> Result<(), String> {
            let prefix = Self::delete_prefix(target);
            for agent in &uniq {
                let (_, _, sandbox) = self.agent_roots(work, agent)?;
                let n = self.lay_down_tree(&store, &sandbox, &tree, agent, Some(&prefix))?;
                self.reset_agent_point(&store, agent, target, &tree)?;
                backed_up.insert(agent.clone(), n);
            }
            WorkspaceService::materialize(self.store.as_ref(), &shared, &store, target)?;
            self.discard_commits_locked(&store, target)
        })();
        self.settle(work, &shared, &store, result)?;
        // 提交已丢：不在保留转录里的留档引用一并作废（它们的标记已随转录被截掉）。
        let pruned = self.prune_archives_locked(&store, keep_marks)?;
        Ok(DeleteReport {
            point: target,
            backed_up,
            pruned,
        })
    }

    pub(super) fn work_archives(&self, work: &str) -> Result<Vec<u64>, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (_shared_root, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        let mut out = Vec::new();
        for (name, r) in self.store.list_refs(&store)? {
            if r.kind != RefKind::Archive {
                continue;
            }
            if let Some(id) = name
                .strip_prefix("archive/")
                .and_then(|s| s.parse::<u64>().ok())
            {
                out.push(id);
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    pub(super) fn work_restore_archive(
        &self,
        work: &str,
        mark: u64,
    ) -> Result<Option<u64>, String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        let name = format!("archive/{}", mark);
        let Some(reference) = self.store.read_ref(&store, &name)? else {
            return Err(format!("没有档案 {}（可能已恢复）", mark));
        };
        let target = reference.target;
        if let Some(id) = target
            && self.store.read_commit(&store, id)?.is_none()
        {
            return Err(format!("档案 {} 指向的提交 {} 不在了", mark, id));
        }
        let tree = match target {
            Some(id) => Self::tree_of(self.store.as_ref(), &store, id)?,
            None => Tree::new(),
        };
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, target)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "恢复会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本。",
                conflicts.join("、")
            ));
        }
        // 先把当前线钉住，再动文件：剪叉不靠"删提交"保证可恢复。
        self.pin_head(&store)?;
        for agent in reference.heads.keys() {
            let (_, _, sandbox) = self.agent_roots(work, agent)?;
            let prefix = format!("restore-{}", mark);
            self.lay_down_tree(&store, &sandbox, &tree, agent, Some(&prefix))?;
            self.reset_agent_point(&store, agent, target, &tree)?;
        }
        let pending = Pending::Restore { target };
        self.store.write_pending(&store, &pending)?;
        let result = WorkspaceService::materialize(self.store.as_ref(), &shared, &store, target);
        self.settle(work, &shared, &store, result)?;
        self.store.remove_ref(&store, &name)?;
        Ok(target)
    }

    #[cfg(test)]
    pub(super) fn work_restore_point(&self, work: &str, commit: Option<u64>) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        if let Some(id) = commit
            && self.store.read_commit(&store, id)?.is_none()
        {
            return Err(format!("没有提交点 {}", id));
        }
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, commit)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "回档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本；强制覆盖需要用户危险确认。",
                conflicts.join("、")
            ));
        }
        self.pin_head(&store)?;
        let pending = Pending::Restore { target: commit };
        self.store.write_pending(&store, &pending)?;
        let result = WorkspaceService::materialize(self.store.as_ref(), &shared, &store, commit);
        self.settle(work, &shared, &store, result)
    }

    #[cfg(test)]
    pub(super) fn work_discard_after(&self, work: &str, keep: Option<u64>) -> Result<(), String> {
        let gate = self.gate(work);
        let _g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let _shared = self.dirs.lock_shared(work)?;
        let (shared, store) = self.work_roots(work)?;
        let _lock = self.store.lock(&store)?;
        self.recover_locked(work, &shared, &store)?;
        if let Some(id) = keep
            && self.store.read_commit(&store, id)?.is_none()
        {
            return Err(format!("没有提交点 {}", id));
        }
        let conflicts = self.external_materialize_conflicts(work, &shared, &store, keep)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "回档会覆盖工作区里被外部改动的内容：{}——先 work_sync 采纳外部版本；强制覆盖需要用户危险确认。",
                conflicts.join("、")
            ));
        }
        // 先物化到 keep、写 head，再删非祖先提交；崩溃时 head 不会指向被删提交。
        let pending = Pending::Discard {
            keep,
            agents: Vec::new(),
        };
        self.store.write_pending(&store, &pending)?;
        let result = (|| {
            WorkspaceService::materialize(self.store.as_ref(), &shared, &store, keep)?;
            self.discard_commits_locked(&store, keep)
        })();
        self.settle(work, &shared, &store, result)
    }
}
