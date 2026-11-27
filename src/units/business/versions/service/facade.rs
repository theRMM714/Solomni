//! 目的：`Versions` 用例面的唯一 trait 实现——逐条转发到同级子模块。
//! 管：入站版本化用例到实现的映射。
//! 不管：具体用例逻辑（commit / sync / history / resolve / lifecycle 各子模块）、端口适配（detail）。
//! 联动：Rust 不允许 trait impl 分块，故用例实现都在子模块里做成 pub(super) 方法，本文件只做转发。

use super::*;
use crate::units::business::versions::api::{
    AdoptReport, ArchiveReport, CommitDetail, CommitFileView, CommitReport, CommitRequest,
    DeleteReport, DiffReport, DiffRequest, IgnoreReport, LostReport, PullReport, PushReport,
    PushRequest, ReadQuery, ReadVerdict, ResetReport, ResetRequest, ResolveReport, ResolveRequest,
    RestoreLostReport, StatusReport, SyncReport, VersionView, Versions, WorkGcReport, WorkLog,
};

impl Versions for VersionService {
    fn work_commit(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<CommitReport, String> {
        VersionService::work_commit(self, work, agent, session, req)
    }
    fn work_push(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &PushRequest,
    ) -> Result<PushReport, String> {
        VersionService::work_push(self, work, agent, session, req)
    }
    fn work_commit_auto(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        req: &CommitRequest,
    ) -> Result<Option<CommitReport>, String> {
        VersionService::work_commit_auto(self, work, agent, session, req)
    }
    fn work_commit_user(
        &self,
        work: &str,
        path: &str,
        bytes: &[u8],
        time: i64,
        line: u64,
    ) -> Result<u64, String> {
        VersionService::work_commit_user(self, work, path, bytes, time, line)
    }
    fn work_pull(&self, work: &str, agent: &str, paths: &[String]) -> Result<PullReport, String> {
        VersionService::work_pull(self, work, agent, paths)
    }
    fn work_adopt_sandbox(
        &self,
        work: &str,
        agent: &str,
        session: &str,
        time: i64,
        line: u64,
    ) -> Result<AdoptReport, String> {
        VersionService::work_adopt_sandbox(self, work, agent, session, time, line)
    }
    fn judge_reads(
        &self,
        work: &str,
        agent: &str,
        queries: &[ReadQuery],
    ) -> Result<Vec<ReadVerdict>, String> {
        VersionService::judge_reads(self, work, agent, queries)
    }
    fn work_gate_info(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<Vec<crate::units::business::versions::api::PathGateInfo>, String> {
        VersionService::work_gate_info(self, work, agent, paths)
    }
    fn work_sync(
        &self,
        work: &str,
        paths: &[String],
        time: i64,
        anchor: crate::units::business::versions::api::CommitAnchor,
    ) -> Result<SyncReport, String> {
        VersionService::work_sync(self, work, paths, time, anchor)
    }
    fn work_import_root(&self, work: &str, time: i64) -> Result<SyncReport, String> {
        VersionService::work_import_root(self, work, time)
    }
    fn work_sync_tracked(
        &self,
        work: &str,
        time: i64,
        anchor: crate::units::business::versions::api::CommitAnchor,
    ) -> Result<SyncReport, String> {
        VersionService::work_sync_tracked(self, work, time, anchor)
    }
    fn work_status(
        &self,
        work: &str,
        agent: &str,
        paths: &[String],
    ) -> Result<StatusReport, String> {
        VersionService::work_status(self, work, agent, paths)
    }
    fn work_log(&self, work: &str, limit: usize, before: Option<u64>) -> Result<WorkLog, String> {
        VersionService::work_log(self, work, limit, before)
    }
    fn work_commit_view(&self, work: &str, commit: u64) -> Result<CommitDetail, String> {
        VersionService::work_commit_view(self, work, commit)
    }
    fn work_commit_file(
        &self,
        work: &str,
        commit: u64,
        path: &str,
    ) -> Result<CommitFileView, String> {
        VersionService::work_commit_file(self, work, commit, path)
    }
    fn work_versions(&self, work: &str, agent: &str, path: &str) -> Result<VersionView, String> {
        VersionService::work_versions(self, work, agent, path)
    }
    fn work_diff(&self, work: &str, agent: &str, req: &DiffRequest) -> Result<DiffReport, String> {
        VersionService::work_diff(self, work, agent, req)
    }
    fn work_reset(
        &self,
        work: &str,
        agent: &str,
        req: &ResetRequest,
    ) -> Result<ResetReport, String> {
        VersionService::work_reset(self, work, agent, req)
    }
    fn work_resolve(
        &self,
        work: &str,
        agent: &str,
        req: &ResolveRequest,
    ) -> Result<ResolveReport, String> {
        VersionService::work_resolve(self, work, agent, req)
    }
    fn work_lost(&self, work: &str, agent: &str, path: Option<&str>) -> Result<LostReport, String> {
        VersionService::work_lost(self, work, agent, path)
    }
    fn work_restore_lost(
        &self,
        work: &str,
        agent: &str,
        path: &str,
        expect_current: &str,
    ) -> Result<RestoreLostReport, String> {
        VersionService::work_restore_lost(self, work, agent, path, expect_current)
    }
    fn work_ignore(
        &self,
        work: &str,
        op: &str,
        patterns: &[String],
    ) -> Result<IgnoreReport, String> {
        VersionService::work_ignore(self, work, op, patterns)
    }
    fn work_recover(&self, work: &str) -> Result<(), String> {
        VersionService::work_recover(self, work)
    }
    fn work_gc(&self, work: &str, apply: bool) -> Result<WorkGcReport, String> {
        VersionService::work_gc(self, work, apply)
    }
    fn work_restore(&self, work: &str, commit: u64) -> Result<(), String> {
        VersionService::work_restore(self, work, commit)
    }
    fn work_restore_force(&self, work: &str, commit: u64) -> Result<(), String> {
        VersionService::work_restore_force(self, work, commit)
    }
    fn work_head(&self, work: &str) -> Result<Option<u64>, String> {
        VersionService::work_head(self, work)
    }
    #[cfg(test)]
    fn work_rewind_to(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
    ) -> Result<Option<u64>, String> {
        VersionService::work_rewind_to(self, work, keep_by_agent)
    }
    fn work_archive(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
        agents: &[String],
        mark: u64,
    ) -> Result<ArchiveReport, String> {
        VersionService::work_archive(self, work, keep_by_agent, agents, mark)
    }
    fn work_delete(
        &self,
        work: &str,
        keep_by_agent: &BTreeMap<String, u64>,
        agents: &[String],
        keep_marks: &[u64],
    ) -> Result<DeleteReport, String> {
        VersionService::work_delete(self, work, keep_by_agent, agents, keep_marks)
    }
    fn work_archives(&self, work: &str) -> Result<Vec<u64>, String> {
        VersionService::work_archives(self, work)
    }
    fn work_restore_archive(&self, work: &str, mark: u64) -> Result<Option<u64>, String> {
        VersionService::work_restore_archive(self, work, mark)
    }
    #[cfg(test)]
    fn work_restore_point(&self, work: &str, commit: Option<u64>) -> Result<(), String> {
        VersionService::work_restore_point(self, work, commit)
    }
    #[cfg(test)]
    fn work_discard_after(&self, work: &str, keep: Option<u64>) -> Result<(), String> {
        VersionService::work_discard_after(self, work, keep)
    }
}
