//! 目的：guest 只读诊断的纯逻辑——把 helper 输出的 JSON 洗成宿主侧结构体，并把失败原因收在一处。
//! 管：报告结构（键与 helper 输出一一对应）、parse_report 的合法性判定、七类失败文案。
//! 不管：guest 通道怎么执行（service.rs）、policy 怎么拼（detail/qemu.rs）、脚本正文（detail/guest_helper.rs）。
//! 联动：api.rs 重导出；service.rs 的 guest_diagnose 用它解析报告并给出原因。

use serde::{Deserialize, Serialize};

/// 目的：一次 guest 只读诊断的完整报告。lease 由宿主补（guest 侧不知道自己叫哪个租约）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuestDiagnosis {
    pub lease: String,
    pub uptime_secs: f64,
    pub loadavg: [f64; 3],
    pub process_total: usize,
    /// 目的：报告被裁剪过（进程数超过 80、挂载过多或字段被截短）；完整时为 false。
    pub truncated: bool,
    pub processes: Vec<GuestProcess>,
    pub memory: GuestMemory,
    pub cpu: GuestCpu,
    pub mounts: Vec<GuestMountInfo>,
    pub guest: GuestMeta,
}

/// 目的：guest 进程表的一行（按采样窗口内的 CPU 占用降序）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuestProcess {
    pub pid: u32,
    pub ppid: u32,
    pub state: String,
    pub cpu_percent: f64,
    pub rss_kb: u64,
    pub etime_secs: u64,
    pub comm: String,
    pub args: String,
}

/// 目的：/proc/meminfo 的摘要与已用比例。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuestMemory {
    pub total_kb: u64,
    pub available_kb: u64,
    pub free_kb: u64,
    pub buffers_kb: u64,
    pub cached_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
    pub used_percent: f64,
}

/// 目的：/proc/stat 首行的 CPU 累计计数与空闲比例（自开机以来的累计口径）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuestCpu {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
    pub idle_percent: f64,
}

/// 目的：一处挂载的容量摘要（只列有容量的挂载）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuestMountInfo {
    pub source: String,
    pub target: String,
    pub fstype: String,
    pub total_kb: u64,
    pub free_kb: u64,
}

/// 目的：guest 自身的内核与核数事实。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuestMeta {
    pub kernel: String,
    pub nproc: u32,
}

/// 目的：本次诊断给的有界超时（秒）：guest 里采两轮 /proc 快照 + 汇总，正常远低于它。
pub const DIAGNOSE_TIMEOUT_SECS: u64 = 15;

/// 目的：租约没有活 guest——与 service::run_guest 的同一句事实（调用方一眼看出不是别的问题）。
pub fn no_live_guest(lease: &str) -> String {
    format!("这次会话没有活着的 guest（租约 {}）", lease)
}

/// 目的：Windows / macOS guest 本期没有 helper 诊断模式，如实拒绝。
pub fn not_linux(os: &str) -> String {
    format!("只有 Linux guest 支持诊断（当前 guest 是 {}）", os)
}

/// 目的：QGA 通道不通（连不上 / 已退出 / 读超时）——与 "guest 里没有进程" 不是一回事。
pub fn channel_down(detail: &str) -> String {
    format!("QGA 通道不通：{}", detail)
}

/// 目的：helper 不在、装不上，或是不支持诊断的早期 helper（要重启会话才换新）。
pub fn helper_unavailable(detail: &str) -> String {
    format!(
        "guest 诊断助手不在或不支持（旧 helper 的话重启会话后重试）：{}",
        detail
    )
}

/// 目的：guest 里没有 python3——helper 本身是 python3 脚本，因此诊断起不来。
pub fn python_missing(detail: &str) -> String {
    format!("guest 缺 python3：诊断助手起不来（{}）", detail)
}

/// 目的：本次诊断超时（helper 的机器标记或宿主外层预算到点）。
pub fn timed_out() -> String {
    format!(
        "guest 诊断超时（{} 秒）：本次诊断已终止",
        DIAGNOSE_TIMEOUT_SECS
    )
}

/// 目的：诊断执行本身失败（helper 在、python3 在，但不是超时也不是通道不通）。
pub fn exec_failed(detail: &str) -> String {
    format!("guest 诊断执行失败：{}", detail)
}

/// 目的：输出不可解析（被通道截断 / 不是 JSON / 缺字段）——绝不退化成一份空诊断。
pub fn unparseable(detail: &str) -> String {
    format!("guest 诊断输出不可解析：{}", detail)
}

/// 目的：helper 输出的 JSON（键与脚本 collect_diagnosis 完全一致；租约不在里面，由宿主补）。
#[derive(Debug, Deserialize)]
struct RawReport {
    uptime_secs: f64,
    loadavg: [f64; 3],
    process_total: usize,
    truncated: bool,
    processes: Vec<RawProcess>,
    memory: RawMemory,
    cpu: RawCpu,
    mounts: Vec<RawMount>,
    guest: RawMeta,
}

#[derive(Debug, Deserialize)]
struct RawProcess {
    pid: u32,
    ppid: u32,
    state: String,
    cpu_percent: f64,
    rss_kb: u64,
    etime_secs: u64,
    comm: String,
    args: String,
}

#[derive(Debug, Deserialize)]
struct RawMemory {
    total_kb: u64,
    available_kb: u64,
    free_kb: u64,
    buffers_kb: u64,
    cached_kb: u64,
    swap_total_kb: u64,
    swap_free_kb: u64,
    used_percent: f64,
}

#[derive(Debug, Deserialize)]
struct RawCpu {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
    idle_percent: f64,
}

#[derive(Debug, Deserialize)]
struct RawMount {
    source: String,
    target: String,
    fstype: String,
    total_kb: u64,
    free_kb: u64,
}

#[derive(Debug, Deserialize)]
struct RawMeta {
    kernel: String,
    nproc: u32,
}

/// 目的：把 helper 打出来的那一个 JSON 对象解析成报告；非法 / 缺字段 / 被截断一律 Err。
/// 约束：绝不返回空诊断——"读不到"必须和"没有进程"分开报。
pub fn parse_report(raw: &str, lease: &str) -> Result<GuestDiagnosis, String> {
    let body = raw.trim();
    // 正常情况下 stdout 只有一个 JSON 对象；允许前面有杂字节（QGA / shell 的噪声），但不吞掉尾部多余内容。
    let source = if body.starts_with('{') {
        body
    } else {
        match raw.find('{') {
            Some(i) => raw[i..].trim_end(),
            None => {
                return Err(unparseable(&format!(
                    "输出里没有 JSON 对象（共 {} 字符）",
                    raw.chars().count()
                )))
            }
        }
    };
    let raw_report: RawReport = serde_json::from_str(source).map_err(|e| {
        let cut = if raw.contains("[截断]") {
            "；输出被通道截断"
        } else {
            ""
        };
        unparseable(&format!("{}（共 {} 字符{}）", e, raw.chars().count(), cut))
    })?;
    if raw_report.process_total < raw_report.processes.len() {
        return Err(unparseable(&format!(
            "进程总数 {} 小于列出的 {} 条",
            raw_report.process_total,
            raw_report.processes.len()
        )));
    }
    Ok(GuestDiagnosis {
        lease: lease.to_string(),
        uptime_secs: raw_report.uptime_secs,
        loadavg: raw_report.loadavg,
        process_total: raw_report.process_total,
        truncated: raw_report.truncated,
        processes: raw_report
            .processes
            .into_iter()
            .map(|p| GuestProcess {
                pid: p.pid,
                ppid: p.ppid,
                state: p.state,
                cpu_percent: p.cpu_percent,
                rss_kb: p.rss_kb,
                etime_secs: p.etime_secs,
                comm: p.comm,
                args: p.args,
            })
            .collect(),
        memory: GuestMemory {
            total_kb: raw_report.memory.total_kb,
            available_kb: raw_report.memory.available_kb,
            free_kb: raw_report.memory.free_kb,
            buffers_kb: raw_report.memory.buffers_kb,
            cached_kb: raw_report.memory.cached_kb,
            swap_total_kb: raw_report.memory.swap_total_kb,
            swap_free_kb: raw_report.memory.swap_free_kb,
            used_percent: raw_report.memory.used_percent,
        },
        cpu: GuestCpu {
            user: raw_report.cpu.user,
            nice: raw_report.cpu.nice,
            system: raw_report.cpu.system,
            idle: raw_report.cpu.idle,
            iowait: raw_report.cpu.iowait,
            irq: raw_report.cpu.irq,
            softirq: raw_report.cpu.softirq,
            steal: raw_report.cpu.steal,
            idle_percent: raw_report.cpu.idle_percent,
        },
        mounts: raw_report
            .mounts
            .into_iter()
            .map(|m| GuestMountInfo {
                source: m.source,
                target: m.target,
                fstype: m.fstype,
                total_kb: m.total_kb,
                free_kb: m.free_kb,
            })
            .collect(),
        guest: GuestMeta {
            kernel: raw_report.guest.kernel,
            nproc: raw_report.guest.nproc,
        },
    })
}
