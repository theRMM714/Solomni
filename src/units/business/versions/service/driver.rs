//! 目的：版本化工作台在回合边界的驱动——写入门禁、自动存档、沙箱采纳与读取过期判定。
//! 管：`VersionedDriver`（`BenchDriver` 的版本化实现）与一次工具调用用的只读门禁 `SessionWriteGate`。
//! 不管：谁用它（`workspace` 的 versioned 策略）；工具面与调度（`agentloop`）；版本库内部（`service` 各用例）。
//! 联动：`agentloop` 每回合从策略取驱动，只在 gate / before / after_write / after_batch / stale_reads 五处碰它。

use crate::kernel::api::types::ToolOutcome;
use crate::kernel::ports::Log;
use crate::units::business::versions::api as dw;
use crate::units::business::versions::api::{
    AdoptReport, CommitRequest, DiffRequest, DiffSign, PathGateInfo, ReadQuery, ReadVerdict,
    Versions,
};
use crate::units::business::workbench::api::{
    BatchDone, Bench, BenchDriver, CallRequest, Coverage, GateFault, GateItem, GateRequest, Need,
    OBSERVATION_FORMAT, ObservationFact, VersionRef, WriteDone, WriteGate,
};
use std::path::Path;
use std::sync::Arc;

/// 目的：一次工具调用用的只读门禁——按路径向版本库取快照，再对账本里的观察事实。
/// 约束：每次 check 都重新取该路径的当前状态，不缓存、不跨调用复用。
pub struct SessionWriteGate {
    versions: Arc<dyn Versions + Send + Sync>,
    work: String,
    agent: String,
    facts: Vec<ObservationFact>,
    worktree: String,
    log: Option<Arc<dyn Log>>,
}

impl SessionWriteGate {
    /// 目的：按一次调用的现场造门禁（版本库句柄、工作线、观察快照、审计口）。
    /// 参数：`versions` = 版本库用例面；`work` / `agent` = 工作线与 agent 名；`worktree` = 这些事实属于哪棵树（空 = 主副本）；`facts` = 批前观察快照；`log` = 拒绝时的审计口。
    pub fn new(
        versions: &Arc<dyn Versions + Send + Sync>,
        work: &str,
        agent: &str,
        worktree: &str,
        facts: &[ObservationFact],
        log: Option<Arc<dyn Log>>,
    ) -> Self {
        Self {
            versions: Arc::clone(versions),
            work: work.to_string(),
            agent: agent.to_string(),
            facts: facts.to_vec(),
            worktree: worktree.to_string(),
            log,
        }
    }

    /// 目的：按快照判定一条写入；不做 I/O，也不记日志。
    fn judge(
        &self,
        path: &str,
        need: Need,
        current: Option<&VersionRef>,
        info: &PathGateInfo,
    ) -> Result<(), GateFault> {
        if !info.tracked {
            return Ok(());
        }
        let target = current.or(info.current.as_ref()).or(info.base.as_ref());
        let Some(target) = target else {
            return Err(GateFault::NotObserved);
        };
        let mine: Vec<&ObservationFact> = self
            .facts
            .iter()
            .filter(|f| {
                f.schema == OBSERVATION_FORMAT && f.path == path && f.worktree == self.worktree
            })
            .collect();
        let same: Vec<&&ObservationFact> =
            mine.iter().filter(|f| f.version == target.hash).collect();
        if same.is_empty() {
            return match mine.first() {
                Some(old) => Err(GateFault::Stale {
                    observed: old.version.clone(),
                    current: target.hash.clone(),
                }),
                None => Err(GateFault::NotObserved),
            };
        }
        // 同一版本的多次分段读按并集授权：读满 1..=total 就等价于一次完整读。
        // 不同版本的事实不合并（版本相同才走到这里）；total_lines 取最小值，宁可少授权。
        let total = same.iter().map(|f| f.total_lines).min().unwrap_or(0);
        let coverage = same.iter().fold(Coverage::Ranges(Vec::new()), |acc, f| {
            acc.union(&f.coverage)
        });
        match need {
            Need::Observed => Ok(()),
            Need::Full if coverage.covers_all(total) => Ok(()),
            Need::Full => Err(GateFault::NotFull { total }),
        }
    }

    /// 目的：快照里这条路径当前显示给审计的版本（优先调用方给的，其次磁盘当前，最后基线）。
    fn shown_version(&self, current: Option<&VersionRef>, info: &PathGateInfo) -> Option<String> {
        current
            .or(info.current.as_ref())
            .or(info.base.as_ref())
            .map(|f| f.hash.clone())
    }

    /// 目的：拒绝写入时留一条可追溯的审计记录；只记路径、需求与版本，不记文件内容。
    fn audit(&self, path: &str, need: Need, fault: &GateFault, current: Option<&str>) {
        let Some(log) = self.log.as_ref() else {
            return;
        };
        let (observed, current) = match fault {
            GateFault::Stale { observed, current } => {
                (Some(observed.as_str()), Some(current.as_str()))
            }
            _ => (None, current),
        };
        let msg = format!(
            "拒绝写入 {}（需要 {:?}）：{}；观察版本 {}；当前版本 {}",
            path,
            need,
            fault.code(),
            observed.unwrap_or("无"),
            current.unwrap_or("无")
        );
        log.warn("tools::write_gate", &msg);
    }

    /// 目的：一次取齐多条路径的快照，避免同一批补丁逐路径重复读版本树。
    fn snapshot(&self, paths: &[String]) -> Result<Vec<PathGateInfo>, GateFault> {
        self.versions
            .work_gate_info(&self.work, &self.agent, paths)
            .map_err(GateFault::Unavailable)
    }
}

impl WriteGate for SessionWriteGate {
    fn check(&self, path: &str, need: Need, current: Option<&VersionRef>) -> Result<(), GateFault> {
        let infos = self.snapshot(&[path.to_string()])?;
        let Some(info) = infos.first() else {
            let fault = GateFault::Unavailable(format!("拿不到 {} 的追踪状态", path));
            self.audit(path, need, &fault, None);
            return Err(fault);
        };
        let shown = self.shown_version(current, info);
        match self.judge(path, need, current, info) {
            Ok(()) => Ok(()),
            Err(fault) => {
                self.audit(path, need, &fault, shown.as_deref());
                Err(fault)
            }
        }
    }

    fn check_many(&self, items: &[GateItem<'_>]) -> Result<(), (usize, GateFault)> {
        if items.is_empty() {
            return Ok(());
        }
        let paths: Vec<String> = items.iter().map(|i| i.path.to_string()).collect();
        let infos = match self.snapshot(&paths) {
            Ok(v) => v,
            Err(fault) => {
                self.audit(items[0].path, items[0].need, &fault, None);
                return Err((0, fault));
            }
        };
        for (i, item) in items.iter().enumerate() {
            let Some(info) = infos.get(i) else {
                let fault = GateFault::Unavailable(format!("拿不到 {} 的追踪状态", item.path));
                self.audit(item.path, item.need, &fault, None);
                return Err((i, fault));
            };
            let shown = self.shown_version(item.current, info);
            if let Err(fault) = self.judge(item.path, item.need, item.current, info) {
                self.audit(item.path, item.need, &fault, shown.as_deref());
                return Err((i, fault));
            }
        }
        Ok(())
    }
}

/// 目的：版本化工作台的回合钩子——持有版本库用例面；引擎只经 `BenchDriver` 的五个方法碰它。
pub struct VersionedDriver {
    versions: Arc<dyn Versions + Send + Sync>,
}

/// 目的：按版本库用例面造一个驱动（`workspace` 的 versioned 策略在 `plan` 里调它）。
pub fn versioned_driver(versions: Arc<dyn Versions + Send + Sync>) -> Arc<dyn BenchDriver> {
    Arc::new(VersionedDriver { versions })
}

impl VersionedDriver {
    /// 目的：把一次调用真正写过的绝对路径翻成工作树内的相对路径（工作副本之外的一律不管）。
    fn rel(&self, bench: &Bench, abs: &Path) -> Option<String> {
        abs.strip_prefix(&bench.private)
            .ok()
            .map(crate::kernel::api::path::slash)
    }

    /// 目的：沙箱采纳的回执（给模型看）；没有改动也没有没采纳的路径时为空串。
    /// 约束：逐路径标出"当前版本看过没有"，并附**有界** diff 摘要——回执只是提醒，不代替 read。
    fn adopt_note(&self, report: &AdoptReport, bench: &Bench, facts: &[ObservationFact]) -> String {
        const MAX_PATHS: usize = 8;
        const MAX_LINES: usize = 40;
        const MAX_BYTES: usize = 8 * 1024;
        let mut out = String::new();
        if let Some(id) = report.commit {
            out.push_str(&format!(
                "\n\n[沙箱改动已存档 {}] 这些文件不是 write / edit 改的（shell 或模块工具），已记进你的存档线。",
                id
            ));
        }
        let infos = self
            .versions
            .work_gate_info(&bench.work_name, &bench.agent, &report.changed)
            .unwrap_or_default();
        let mut unread: Vec<String> = Vec::new();
        if !report.changed.is_empty() {
            out.push_str("\n改动路径（当前版本）：");
            for path in report.changed.iter().take(MAX_PATHS) {
                let version = infos
                    .iter()
                    .find(|i| &i.path == path)
                    .and_then(|i| i.current.as_ref().or(i.base.as_ref()))
                    .map(|f| f.hash.as_str());
                let seen = version.is_some_and(|v| {
                    facts
                        .iter()
                        .any(|f| f.path == *path && f.worktree == bench.agent && f.version == v)
                });
                out.push_str(&format!(
                    "\n  - {}：{}",
                    path,
                    if seen { "已读" } else { "未读" }
                ));
                if !seen {
                    unread.push(path.clone());
                }
            }
            if report.changed.len() > MAX_PATHS {
                out.push_str(&format!(
                    "\n  ……还有 {} 个路径未列出（先 read 再改）",
                    report.changed.len() - MAX_PATHS
                ));
            }
            if !unread.is_empty() {
                out.push_str(
                    "\n这些路径的当前内容尚未被你看过；直接 edit / write 会被门禁拒绝，先 read。",
                );
            }
        }
        for h in &report.held {
            out.push_str(&format!("\n[未采纳] {}：{}", h.path, h.why));
        }
        // 有界 diff：最多前 8 个路径、40 行、8 KiB；超出只报增删行数与"已截断"。
        let paths: Vec<String> = report.changed.iter().take(MAX_PATHS).cloned().collect();
        if paths.is_empty() {
            return out;
        }
        let Ok(diff) = self.versions.work_diff(
            &bench.work_name,
            &bench.agent,
            &DiffRequest {
                from: None,
                to: None,
                paths,
            },
        ) else {
            return out;
        };
        let mut shown = 0usize;
        let mut added = 0usize;
        let mut removed = 0usize;
        let mut body = String::new();
        for e in &diff.entries {
            let mut lines: Vec<String> = Vec::new();
            if e.binary {
                lines.push("（二进制文件，不展开行差异）".to_string());
            } else if e.too_large {
                lines.push("（超过单文件上限，不展开行差异；用 read 看当前内容）".to_string());
            } else {
                for h in &e.hunks {
                    for line in &h.lines {
                        match line.sign {
                            DiffSign::Add => added += 1,
                            DiffSign::Remove => removed += 1,
                            DiffSign::Context => {}
                        }
                        if shown < MAX_LINES && body.len() < MAX_BYTES {
                            let sign = match line.sign {
                                DiffSign::Add => '+',
                                DiffSign::Remove => '-',
                                DiffSign::Context => ' ',
                            };
                            lines.push(format!("{}{}", sign, line.text));
                            shown += 1;
                        }
                    }
                }
            }
            if lines.is_empty() {
                continue;
            }
            body.push_str(&format!("\n--- a/{}\n+++ b/{}", e.path, e.path));
            for line in lines {
                body.push('\n');
                body.push_str(&line);
            }
        }
        if added + removed == 0 {
            return out;
        }
        out.push_str("\n[沙箱改动摘要，已截断]");
        out.push_str(&body);
        let total = added + removed;
        if total > shown || shown >= MAX_LINES || body.len() >= MAX_BYTES {
            out.push_str(&format!(
                "\n……还有约 {} 行未显示（+{} / -{}）",
                total.saturating_sub(shown),
                added,
                removed
            ));
        }
        out
    }
}

impl BenchDriver for VersionedDriver {
    fn gate(&self, req: &GateRequest<'_>) -> Option<Arc<dyn WriteGate>> {
        Some(Arc::new(SessionWriteGate::new(
            &self.versions,
            req.work,
            req.agent,
            req.worktree,
            req.facts,
            req.log.clone(),
        )))
    }

    fn before(&self, req: &CallRequest<'_>) -> Option<ToolOutcome> {
        if req.name != dw::COMMIT {
            return None;
        }
        let args: serde_json::Value = serde_json::from_str(req.args_json).ok()?;
        let strings = |key: &str| -> Vec<String> {
            args.get(key)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut paths = strings("paths");
        let deletes = strings("deletes");
        if paths.is_empty() {
            let report = self
                .versions
                .work_status(&req.bench.work_name, &req.bench.agent, &[])
                .ok()?;
            paths = report.entries.into_iter().map(|e| e.path).collect();
        }
        paths.retain(|p| !deletes.contains(p));
        if paths.is_empty() {
            return None;
        }
        let infos =
            match self
                .versions
                .work_gate_info(&req.bench.work_name, &req.bench.agent, &paths)
            {
                Ok(v) => v,
                Err(e) => return Some(ToolOutcome::new(false, e)),
            };
        let gate = SessionWriteGate::new(
            &self.versions,
            &req.bench.work_name,
            &req.bench.agent,
            &req.bench.agent,
            req.facts,
            None,
        );
        for info in &infos {
            if !info.tracked {
                continue;
            }
            let changed = match (&info.current, &info.base) {
                (None, None) => false,
                (Some(c), Some(b)) => c.hash != b.hash,
                _ => true,
            };
            if !changed {
                continue;
            }
            if let Err(f) = gate.check(&info.path, Need::Observed, info.current.as_ref()) {
                return Some(ToolOutcome::new(
                    false,
                    format!(
                        "work_commit 不能提交未读的已追踪路径 {}：{}",
                        info.path,
                        f.message(&info.path)
                    ),
                ));
            }
        }
        None
    }

    fn after_write(&self, req: &WriteDone<'_>) -> Option<String> {
        let mut paths: Vec<String> = req
            .written
            .iter()
            .filter_map(|p| self.rel(req.bench, p))
            .collect();
        let mut deletes: Vec<String> = req
            .removed
            .iter()
            .filter_map(|p| self.rel(req.bench, p))
            .collect();
        paths.sort();
        paths.dedup();
        deletes.sort();
        deletes.dedup();
        if paths.is_empty() && deletes.is_empty() {
            return None;
        }
        let message = if paths.is_empty() {
            "工具删除"
        } else {
            "工具写入"
        };
        let commit = CommitRequest {
            paths,
            deletes,
            message: message.to_string(),
            time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            // 锚 = 这次工具调用所属的那一行：回档才能把这次写入和它的转录证据一起退掉。
            line: req.line,
        };
        match self.versions.work_commit_auto(
            &req.bench.work_name,
            &req.bench.agent,
            &req.bench.session,
            &commit,
        ) {
            Ok(_) => None,
            Err(e) => Some(format!(
                "\n\n[存档未完成] 这次改动没有记进存档：{}。文件已在你的沙箱里，但回档/删除时可能对不上",
                e
            )),
        }
    }

    fn after_batch(&self, req: &BatchDone<'_>) -> Option<String> {
        let bench = req.bench;
        if bench.work_name.is_empty() {
            return None;
        }
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let note = match self.versions.work_adopt_sandbox(
            &bench.work_name,
            &bench.agent,
            &bench.session,
            time,
            req.line,
        ) {
            Ok(report) => self.adopt_note(&report, bench, req.facts),
            Err(e) => format!("\n\n[沙箱改动未存档] 这些文件没有记进存档：{}", e),
        };
        Some(note)
    }

    fn stale_reads(&self, work: &str, agent: &str, queries: &[ReadQuery]) -> Option<Vec<bool>> {
        Some(
            self.versions
                .judge_reads(work, agent, queries)
                .ok()?
                .into_iter()
                .map(|v: ReadVerdict| v.is_stale())
                .collect(),
        )
    }
}
