//! 目的：`Workspace` 用例面的唯一 trait 实现——基础用例写在这里，其余逐条转发到同级子模块。
//! 管：入站用例到实现的映射（bind / prepare / discard / work_has / files / usage / roots 直接实现）。
//! 不管：具体用例逻辑（commit / sync / history / resolve / lifecycle 各子模块）、端口适配（detail）。
//! 联动：Rust 不允许 trait impl 分块，故用例实现都在子模块里做成 pub(super) 方法，本文件只做转发。

use super::*;
use crate::capabilities::workspace::api::{
    AdoptReport, ArchiveReport, CommitDetail, CommitFileView, CommitReport, CommitRequest,
    DeleteReport, GcReport, LostReport, PullReport, ReadQuery, ResolveReport, ResolveRequest,
    RestoreLostReport, StatusReport, SyncReport, VersionView, WorkFiles, WorkLog, WorkRoots,
    WorkUsage, Workspace,
};

impl Workspace for WorkspaceService {
    fn bind(&self, session: &str, root: Option<&Path>) -> Result<(), String> {
        self.dirs.bind(session, root)
    }

    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String> {
        self.dirs.prepare(session, agents)
    }

    fn discard(&self, session: &str, agents: &[String]) -> Result<(), String> {
        self.dirs.discard(session, agents)
    }

    fn work_has(&self, session: &str, name: &str) -> bool {
        self.dirs.work_has(session, name)
    }

    fn files(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String> {
        self.dirs.list(session, agents)
    }

    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String> {
        self.dirs.usage(session, agents)
    }

    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String> {
        self.dirs.roots(session, agents)
    }

    fn work_commit(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<CommitReport, String> {
        WorkspaceService::work_commit(self, work, agent, session, req)
    }
    fn work_commit_auto(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<Option<CommitReport>, CommitRefusal> {
        WorkspaceService::work_commit_auto(self, work, agent, session, req)
    }
    fn work_commit_user(
        &self,
        work: &str,
        path: &str,
        bytes: &[u8],
        time: i64,
        line: u64,
    ) -> Result<u64, String> {
        WorkspaceService::work_commit_user(self, work, path, bytes, time, line)
    }
    fn work_pull(&self, work: &str, agent: &str, paths: &[String]) -> Result<PullReport, String> {
        WorkspaceService::work_pull(self, work, agent, paths)
    }
    fn work_adopt_sandbox(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        time: i64,
        line: u64,
    ) -> Result<AdoptReport, String> {
        WorkspaceService::work_adopt_sandbox(self, work, agent, session, time, line)
    }
    fn work_stale_reads(&self, work: &str, queries: &[ReadQuery]) -> Result<Vec<bool>, String> {
        WorkspaceService::work_stale_reads(self, work, queries)
    }
    fn work_sync(&self, work: &str, paths: &[String], time: i64) -> Result<SyncReport, String> {
        WorkspaceService::work_sync(self, work, paths, time)
    }
    fn work_import_root(&self, work: &str, time: i64) -> Result<SyncReport, String> {
        WorkspaceService::work_import_root(self, work, time)
    }
    fn work_sync_tracked(&self, work: &str, time: i64) -> Result<SyncReport, String> {
        WorkspaceService::work_sync_tracked(self, work, time)
    }
    fn work_status(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<StatusReport, String> {
        WorkspaceService::work_status(self, work, agent, paths)
    }
    fn work_log(&self, work: &str, limit: usize, before: Option<u64>) -> Result<WorkLog, String> {
        WorkspaceService::work_log(self, work, limit, before)
    }
    fn work_commit_view(&self, work: &str, commit: u64) -> Result<CommitDetail, String> {
        WorkspaceService::work_commit_view(self, work, commit)
    }
    fn work_commit_file(
        &self,
        work: &str,
        commit: u64,
        path: &str,
    ) -> Result<CommitFileView, String> {
        WorkspaceService::work_commit_file(self, work, commit, path)
    }
    fn work_versions(&self, work: &str, agent: &str, path: &str) -> Result<VersionView, String> {
        WorkspaceService::work_versions(self, work, agent, path)
    }
    fn work_resolve(
        &self,
        work: &str,
        agent: &str,
        req: &ResolveRequest,
    ) -> Result<ResolveReport, String> {
        WorkspaceService::work_resolve(self, work, agent, req)
    }
    fn work_lost(&self, work: &str, agent: &str, path: Option<&str>) -> Result<LostReport, String> {
        WorkspaceService::work_lost(self, work, agent, path)
    }
    fn work_restore_lost(
        &self,
        work: &str,
        agent: &str,
        path: &str,
        expect_current: &str,
    ) -> Result<RestoreLostReport, String> {
        WorkspaceService::work_restore_lost(self, work, agent, path, expect_current)
    }
    fn work_recover(&self, work: &str) -> Result<(), String> {
        WorkspaceService::work_recover(self, work)
    }
    fn work_gc(&self, work: &str, apply: bool) -> Result<GcReport, String> {
        WorkspaceService::work_gc(self, work, apply)
    }
    fn work_restore(&self, work: &str, commit: u64) -> Result<(), String> {
        WorkspaceService::work_restore(self, work, commit)
    }
    fn work_restore_force(&self, work: &str, commit: u64) -> Result<(), String> {
        WorkspaceService::work_restore_force(self, work, commit)
    }
    fn work_head(&self, work: &str) -> Result<Option<u64>, String> {
        WorkspaceService::work_head(self, work)
    }
    #[cfg(test)]
    fn work_rewind_to(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
    ) -> Result<Option<u64>, String> {
        WorkspaceService::work_rewind_to(self, work, keep_by_agent)
    }
    fn work_archive(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
        agents: &[String],
        mark: u64,
    ) -> Result<ArchiveReport, String> {
        WorkspaceService::work_archive(self, work, keep_by_agent, agents, mark)
    }
    fn work_delete(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
        agents: &[String],
        keep_marks: &[u64],
    ) -> Result<DeleteReport, String> {
        WorkspaceService::work_delete(self, work, keep_by_agent, agents, keep_marks)
    }
    fn work_archives(&self, work: &str) -> Result<Vec<u64>, String> {
        WorkspaceService::work_archives(self, work)
    }
    fn work_restore_archive(&self, work: &str, mark: u64) -> Result<Option<u64>, String> {
        WorkspaceService::work_restore_archive(self, work, mark)
    }
    #[cfg(test)]
    fn work_restore_point(&self, work: &str, commit: Option<u64>) -> Result<(), String> {
        WorkspaceService::work_restore_point(self, work, commit)
    }
    #[cfg(test)]
    fn work_discard_after(&self, work: &str, keep: Option<u64>) -> Result<(), String> {
        WorkspaceService::work_discard_after(self, work, keep)
    }
}
