//! 目的：写入门禁的实现——按路径向版本库取快照，再对会话账本里的观察事实判定。
//! 管：`SessionWriteGate`（一次工具调用用的只读门禁：判定、审计、批量快照）。
//! 不管：口径与端口的形状（在 `workbench`）；账本的事实来源（`tools` 的 `Observations`）；版本查询（`versions`）。
//! 联动：引擎把它包成 `Option<&dyn WriteGate>` 交给内置工具；拒绝要留审计，只记版本不记内容。

use crate::units::business::tools::api::{Coverage, OBSERVATION_FORMAT, ObservationFact};
use crate::units::business::versions::api::Versions;
use crate::units::business::workbench::api::{GateFault, GateItem, Need, VersionRef, WriteGate};

/// 目的：一次工具调用用的只读门禁——按路径向工作区取快照，再对账本里的观察事实。
/// 约束：只在调用期间借用；每次 check 都重新取该路径的当前状态，不缓存、不跨调用复用。
pub struct SessionWriteGate<'a> {
    /// 目的：取 tracked / 当前 / 基线版本的版本库用例面。
    pub versions: &'a (dyn Versions + Send + Sync),
    /// 目的：工作名（定位工作线）。
    pub work: &'a str,
    /// 目的：agent 名（定位它的工作线）。
    pub agent: &'a str,
    /// 目的：当前上下文账本里的观察事实（调用方已按批前快照裁剪）。
    pub facts: &'a [ObservationFact],
    /// 目的：这些事实属于哪棵工作树（空 = 主副本）。
    pub worktree: &'a str,
    /// 目的：拒绝时记审计日志（只记路径、需求、观察/当前版本，不记文件内容）。
    pub log: Option<&'a dyn crate::kernel::ports::Log>,
}

impl SessionWriteGate<'_> {
    /// 目的：按快照判定一条写入；不做 I/O，也不记日志。
    fn judge(
        &self,
        path: &str,
        need: Need,
        current: Option<&VersionRef>,
        info: &crate::units::business::versions::api::PathGateInfo,
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
    fn shown_version(
        &self,
        current: Option<&VersionRef>,
        info: &crate::units::business::versions::api::PathGateInfo,
    ) -> Option<String> {
        current
            .or(info.current.as_ref())
            .or(info.base.as_ref())
            .map(|f| f.hash.clone())
    }

    /// 目的：拒绝写入时留一条可追溯的审计记录；只记路径、需求与版本，不记文件内容。
    fn audit(&self, path: &str, need: Need, fault: &GateFault, current: Option<&str>) {
        let Some(log) = self.log else {
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
    fn snapshot(
        &self,
        paths: &[String],
    ) -> Result<Vec<crate::units::business::versions::api::PathGateInfo>, GateFault> {
        self.versions
            .work_gate_info(self.work, self.agent, paths)
            .map_err(GateFault::Unavailable)
    }
}

impl WriteGate for SessionWriteGate<'_> {
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
