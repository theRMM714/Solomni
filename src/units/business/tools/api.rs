//! 目的：工具单元的入站面——其它单元与呈现层只准用这里（不许碰 `domain` / `ports`）。
//! 管：`Tools`（拿总表与保留名）与 `ToolExec`（执行外部工具 / shell / 内置工具、释放围栏授权）两个 trait，并重导出领域共享类型；席位面由会话单元按总表合成。
//! 不管：实现与机制（在 `service/` / `detail/`）；端口定义（在 `ports.rs`）。
//! 联动：实现见 `service.rs`；`runtime` 等消费方只持 `Arc<dyn Tools>` / `Arc<dyn ToolExec>`。

pub use crate::units::business::tools::domain::budget::{
    ResultBudget, cap_result_with, continuation_for,
};
pub use crate::units::business::tools::domain::commands::{CommandBook, Phase, Runtime};
pub use crate::units::business::tools::domain::op::{
    Executor, Op, OpBook, complete as complete_op, parse as parse_op, usage as op_usage,
    usage_named,
};
pub use crate::units::business::tools::domain::patch::{EditFault, Fault};
pub use crate::units::business::tools::domain::schema::{
    ArgFault, ToolBook, ToolSchema, json_schema_lines,
};
pub use crate::units::business::tools::domain::shell::{
    ShellFacts, did_run, facts as shell_facts, parse as parse_shell, place_notes, shared_exec_note,
    truncated,
};
pub use crate::units::business::tools::domain::systool::{
    DELETE, EDIT, GREP, LIST, Observations, PATCH, READ, REPORT, SHELL, ToolNotes, ToolOutcome,
    WRITE, arg_fault_text, is_builtin, is_exec, is_freeform, names, patch_decl, refuse, tool_notes,
};
pub use crate::units::business::tools::domain::table::{SystemTools, action_audit};

pub use crate::units::business::workspace::api::GateFile;
use crate::units::business::workspace::api::{ReadFact, Workspace};

/// 目的：一个从 1 开始、闭区间的行范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LineRange {
    /// 目的：起点行号（1 起）。
    pub from: u32,
    /// 目的：终点行号（1 起、含此行）。
    pub to: u32,
}

impl LineRange {
    /// 目的：按端点造一个闭区间。
    pub fn new(from: u32, to: u32) -> LineRange {
        LineRange { from, to }
    }
}

/// 目的：这次观察覆盖了文件的哪些行。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Coverage {
    /// 目的：整份原文都看过（完整读、整份写、patch Add）。
    Full,
    /// 目的：只看过这些行（分段读、edit、patch Update）。
    Ranges(Vec<LineRange>),
}

impl Coverage {
    /// 目的：把一组行区间排序、去重、合并相邻与重叠区间。
    /// 约束：只保留 from <= to 的合法区间；相邻区间合并（1-2 + 3-4 = 1-4）不改变覆盖语义。
    pub fn normalized(&self) -> Coverage {
        match self {
            Coverage::Full => Coverage::Full,
            Coverage::Ranges(ranges) => Coverage::Ranges(normalize_ranges(ranges.iter().copied())),
        }
    }

    /// 目的：两次观察的覆盖并集；任一方为 Full 则结果 Full。
    pub fn union(&self, other: &Coverage) -> Coverage {
        match (self, other) {
            (Coverage::Full, _) | (_, Coverage::Full) => Coverage::Full,
            (Coverage::Ranges(a), Coverage::Ranges(b)) => {
                Coverage::Ranges(normalize_ranges(a.iter().chain(b).copied()))
            }
        }
    }

    /// 目的：行区间并集是否覆盖 1..=total；Full 直接为真。
    /// 约束：空文件（total == 0）只有 Full 才能覆盖；区间不能凭空覆盖不存在的行。
    pub fn covers_all(&self, total: u32) -> bool {
        match self {
            Coverage::Full => true,
            Coverage::Ranges(ranges) => {
                if total == 0 {
                    return false;
                }
                let sorted: Vec<LineRange> = normalize_ranges(ranges.iter().copied())
                    .into_iter()
                    .filter(|r| r.from <= total)
                    .map(|r| LineRange::new(r.from, r.to.min(total)))
                    .collect();
                let mut next = 1u32;
                for r in sorted {
                    if r.from > next {
                        return false;
                    }
                    if r.to >= next {
                        next = r.to.saturating_add(1);
                    }
                    if next > total {
                        return true;
                    }
                }
                next > total
            }
        }
    }
}

/// 目的：排序、去重并合并相邻/重叠的行区间；输入里的空区间（from > to）直接丢弃。
pub fn normalize_ranges(ranges: impl IntoIterator<Item = LineRange>) -> Vec<LineRange> {
    let mut sorted: Vec<LineRange> = ranges.into_iter().filter(|r| r.from <= r.to).collect();
    sorted.sort_by_key(|r| (r.from, r.to));
    let mut out: Vec<LineRange> = Vec::with_capacity(sorted.len());
    for r in sorted {
        match out.last_mut() {
            Some(last) if r.from <= last.to.saturating_add(1) => {
                last.to = last.to.max(r.to);
            }
            _ => out.push(r),
        }
    }
    out
}

/// 目的：观察来源，只用于诊断与显示，不参与授权判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ObservationSource {
    /// 目的：完整或分段 read。
    Read,
    /// 目的：整份 write。
    Write,
    /// 目的：单处或多处 edit。
    Edit,
    /// 目的：patch 的 Add 块。
    PatchAdd,
    /// 目的：patch 的 Update 块。
    PatchUpdate,
    /// 目的：work_diff 的差异。
    Diff,
}

/// 目的：观察事实的持久化格式版本；与代码不符的事实一律降级为“没有观察”（失败关闭）。
pub const OBSERVATION_FORMAT: u32 = 1;

/// 目的：一条观察事实——某个路径的某个整份版本，被看过哪些行。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObservationFact {
    /// 目的：持久化格式版本；旧转录/损坏数据缺省为 0，门禁不认。
    #[serde(default)]
    pub schema: u32,
    /// 目的：这条观察来自哪一次模型回复（= 该回复第一行的行 id）；压缩按它丢弃失效观察。
    pub reply: u64,
    /// 目的：工作树内的相对路径（斜杠分隔）。
    pub path: String,
    /// 目的：空 = 主副本；否则 = agent 沙箱名。
    pub worktree: String,
    /// 目的：整份内容的 SHA-256 十六进制串；必须与 ReadFact.blob / Change.hash 同算法。
    pub version: String,
    /// 目的：这条事实覆盖了该版本的哪些行。
    pub coverage: Coverage,
    /// 目的：该版本的逻辑行数；区间覆盖是否等于全文时用。
    pub total_lines: u32,
    /// 目的：这条事实由哪种操作产生（诊断与显示）。
    pub source: ObservationSource,
}

impl ObservationFact {
    /// 目的：把这条观察还原成 freshness 用的读取事实：整份读 = 一条，分段读 = 每段一条。
    /// 约束：from/to 只落在该版本确实有内容的行上（空文件、越界段不产生）；blob 与版本同一指纹。
    pub fn read_facts(&self) -> Vec<ReadFact> {
        if self.schema != OBSERVATION_FORMAT || self.source != ObservationSource::Read {
            return Vec::new();
        }
        let total = u64::from(self.total_lines);
        let mut out = Vec::new();
        let mut push = |from: u64, to: u64| {
            if total == 0 || from == 0 || from > to || from > total {
                return;
            }
            out.push(ReadFact {
                worktree: self.worktree.clone(),
                path: self.path.clone(),
                from,
                to: to.min(total),
                blob: self.version.clone(),
            });
        };
        match &self.coverage {
            Coverage::Full => push(1, total),
            Coverage::Ranges(ranges) => {
                for r in ranges {
                    push(u64::from(r.from), u64::from(r.to));
                }
            }
        }
        out
    }
}

/// 目的：写入门禁要求观察到的程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// 目的：观察过该版本即可（edit 单处、patch Update、work_commit）。
    Observed,
    /// 目的：必须完整观察过该版本（write 覆盖、patch Add 覆盖、replace_all）。
    Full,
}

/// 目的：写入门禁拒绝的原因；带稳定错误码，供模型与 UI 区分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFault {
    /// 目的：当前上下文里没有这个版本的任何观察。
    NotObserved,
    /// 目的：观察到的是旧版本；当前版本已经变了。
    Stale {
        /// 目的：账本里那次的版本哈希。
        observed: String,
        /// 目的：磁盘上当前的版本哈希。
        current: String,
    },
    /// 目的：观察过，但只覆盖了部分行，而这次要求全文。
    NotFull {
        /// 目的：该版本的逻辑行数。
        total: u32,
    },
    /// 目的：当前内容存在，但没有权限/信息判断。
    Unavailable(String),
}

impl GateFault {
    /// 目的：稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            GateFault::NotObserved => "FS_NOT_OBSERVED",
            GateFault::Stale { .. } => "FS_STALE_VERSION",
            GateFault::NotFull { .. } => "FS_NOT_FULLY_READ",
            GateFault::Unavailable(_) => "FS_GATE_UNAVAILABLE",
        }
    }

    /// 目的：给模型看的中文说明；保留现有测试依赖的关键短语。
    pub fn message(&self, path: &str) -> String {
        let body = match self {
            GateFault::NotObserved => format!("要先看过 {} 当前版本才能改；请 read 后再改。", path),
            GateFault::Stale { .. } => format!(
                "{} 在你读过之后又被改动过（内容与当时不同）：请重新 read 一遍再决定怎么写，不要凭记忆覆盖。",
                path
            ),
            GateFault::NotFull { .. } => format!(
                "这次要整份覆盖 {}，但你只读到一部分：整份覆盖会丢掉没读到的内容。请先完整 read 全文，或改用 edit 只改那一处。",
                path
            ),
            GateFault::Unavailable(why) => format!(
                "暂时无法确认 {} 的当前版本（{}）；请稍后重试，不要凭记忆覆盖。",
                path, why
            ),
        };
        format!("[{}] {}", self.code(), body)
    }
}

/// 目的：一条待判定的写入；`current` 是即将被替换/覆盖的磁盘版本。
#[derive(Clone, Copy)]
pub struct GateItem<'a> {
    /// 目的：工作树内的相对路径。
    pub path: &'a str,
    /// 目的：这次写入要求观察到什么程度。
    pub need: Need,
    /// 目的：即将被覆盖的磁盘版本；None = 文件不存在（或调用方没取到）。
    pub current: Option<&'a GateFile>,
}

/// 目的：写入门禁端口；None = 不做门禁（内部写、直调测试）。
/// 约束：current = 即将被替换/覆盖的磁盘版本；None = 文件不存在。
pub trait WriteGate: Send + Sync {
    /// 目的：判断账本是否足以支撑这次写入；`current` 是即将被覆盖的磁盘版本。
    fn check(&self, path: &str, need: Need, current: Option<&GateFile>) -> Result<(), GateFault>;

    /// 目的：一次判定多条写入（同一批补丁只取一次工作区快照）；返回第一条失败的序号与原因。
    /// 约束：默认逐条调用 `check`；实现方可以合并快照，但不能改变单条的判定语义。
    fn check_many(&self, items: &[GateItem<'_>]) -> Result<(), (usize, GateFault)> {
        for (i, item) in items.iter().enumerate() {
            if let Err(f) = self.check(item.path, item.need, item.current) {
                return Err((i, f));
            }
        }
        Ok(())
    }
}

/// 目的：一次工具调用用的只读门禁——按路径向工作区取快照，再对账本里的观察事实。
/// 约束：只在调用期间借用；每次 check 都重新取该路径的当前状态，不缓存、不跨调用复用。
pub struct SessionWriteGate<'a> {
    /// 目的：取 tracked / 当前 / 基线版本的工作区。
    pub workspace: &'a (dyn Workspace + Send + Sync),
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
        current: Option<&GateFile>,
        info: &crate::units::business::workspace::api::PathGateInfo,
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
        current: Option<&GateFile>,
        info: &crate::units::business::workspace::api::PathGateInfo,
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
    ) -> Result<Vec<crate::units::business::workspace::api::PathGateInfo>, GateFault> {
        self.workspace
            .work_gate_info(self.work, self.agent, paths)
            .map_err(GateFault::Unavailable)
    }
}

impl WriteGate for SessionWriteGate<'_> {
    fn check(&self, path: &str, need: Need, current: Option<&GateFile>) -> Result<(), GateFault> {
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

/// 目的：系统工具总表与名字表的**入站面**：别的单元只拿声明书，**看不见表里的保留名与自检实现**。
/// 约束：表本体（`SystemTools`）是纯数据、没有端口；组合根装载一次，`runtime` 只持 `Arc<dyn Tools>`
///   并与协作会话共享（见 `service.rs`）。**本单元不认识席位**：面由会话单元的机制声明按总表合成。
pub trait Tools: Send + Sync {
    /// 内置工具总表（工具**是什么**的声明书）：调用方要持有它时拿走一份。
    fn book(&self) -> ToolBook;

    /// agent 保留名（`systools/names.yaml` 的 `agents`）：`session/<工作>/` 下由布局固定占用的
    /// 目录名，agent 实例名不得占用。名单在表里，不在代码里。
    fn reserved_agents(&self) -> Vec<String>;

    /// 模块保留名（同表 `modules` 块）：模块 id 不得占用。产品命令名与别名由 Op 目录在装配期并入。
    fn reserved_modules(&self) -> Vec<String>;

    /// 总表自身的自洽断言（空 = 一切正常）：装配期自检与测试门禁读它。
    fn problems(&self) -> Vec<String>;
}

/// 目的：一次内置工具调用的结果：结论与更新后的观察账本。
/// 约束：账本属于会话，**按值进出**——调用方交一份，拿回更新后的那份；不把可变借用递给别的单元（R16）。
#[derive(Debug, Clone)]
pub struct Recorded {
    /// 目的：这次调用的结论（回给模型的那一段文本）。
    pub outcome: ToolOutcome,
    /// 目的：更新后的观察账本（调用方原样接管）。
    pub observations: Observations,
}

/// 目的：工具单元的**执行面**（`service.rs` 实现）：别的单元要执行工具、要释放围栏授权，走这里。
/// 约束：出站端口 `SysIo` 由它持有（R12）；`ProcessRunner` / `FenceHost` 是 kernel 共享的机制端口。
///   为什么 `SysIo` 的读写不在这里：它只被本单元自己的 domain（内置工具实现）用，
///   别人要的是"跑一个工具"，不是"按路径读写文件"。
pub trait ToolExec: Send + Sync {
    /// 执行一次**外部工具**（模块声明的那种）：围栏安装、守门进程、超时杀树、截断都在端口后面。
    /// 参数：`module` = 所属模块 id（按它取进程租约）；`env` = 该模块隐私字段的注入项（只经环境变量进进程，不进命令行）；
    ///   `path` = 本机档运行包的生效 PATH（`None` = 用围栏白名单里的 PATH）；
    ///   `parallel` = 该工具是否声明只读可并发（true 取共享租约，false 取独占租约）；
    ///   `ask` = 这一趟的**提问端口**（围栏的必要落点授不上时经它问用户）；`None` = 没有可回答的前端。
    /// 约束：同一模块的独占调用跨 agent 串行；不同模块与内置工具不受影响。
    #[allow(clippy::too_many_arguments)]
    fn run_module(
        &self,
        module: &str,
        fence: &crate::kernel::api::fence::FencePlan,
        command: &str,
        args_json: &str,
        env: &[(String, String)],
        path: Option<&std::ffi::OsStr>,
        parallel: bool,
        ask: Option<&dyn crate::kernel::ports::AskUser>,
        budget: ResultBudget,
    ) -> ToolOutcome;

    /// 执行一次**一次性 shell**：平台 shell、按次超时、取消与 head+tail 截断都在 kernel 的 `ProcessRunner` 后面；
    /// 跑完（若这次用的是按次临时 profile）由本单元释放围栏授权。
    fn run_shell(
        &self,
        run: &crate::kernel::ports::ShellRun<'_>,
        budget: ResultBudget,
    ) -> ToolOutcome;

    /// 执行一次**内置工具**：放行、寻址、参数校验在本单元的 domain，机制在 `SysIo` 后面。
    /// 参数：`observations` = 会话的观察账本（按值传入）；`gate` = 写入门禁（`None` = 直调兜底，只给旧测试用）；
    ///   `budget` = 本批回执预算，read 在**执行前**按它选行，整批不会超剩余上下文。
    /// 返回：结论与更新后的账本（见 `Recorded`）。
    #[allow(clippy::too_many_arguments)]
    fn run_builtin(
        &self,
        sb: &crate::units::business::workspace::api::Sandbox,
        builtin_tools: &ToolBook,
        observations: Observations,
        gate: Option<&dyn WriteGate>,
        budget: ResultBudget,
        name: &str,
        args_json: &str,
    ) -> Recorded;

    /// 一次调用的收尾：撤它自己的 ACE、删按次容器 profile 与临时目录（调用方只提出请求，不碰任何 ACL）。
    fn release_call(&self, spec: &crate::kernel::api::fence::FencePlan) -> Result<(), String>;

    /// 按会话租约撤销：该会话的授权与 profile 由核心在改配 / 删除时统一回收。
    fn release_by_lease(&self, lease: &str) -> Result<(), String>;
}
