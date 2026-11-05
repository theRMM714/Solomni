//! 目的：vm 能力的用例与端口持有者。
//! 管：持 VmEngine / ShareBackend（R12），按租约管理 guest 生命周期
//!   （guest 与共享凭据同生共死；每次执行前后走共享的同步钩子）。
//! 不管：纯逻辑（domain/）、运行包库与选包（packages）；适配器（detail/）由组合根构造。
//! 联动：端口在 ports.rs；被 api::Vm 的调用方消费。

use crate::capabilities::vm::api::Vm;
use crate::capabilities::vm::domain::guest::{
    self as guest, GuestMount, GuestOs, GuestRun, GuestSpec, OutputLimit,
};
use crate::capabilities::vm::ports::{GuestVm, ShareBackend, ShareGuard, VmEngine};
use crate::kernel::api::ToolOutcome;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// 目的：一个租约的活体——guest、共享后端与它的凭据、它要同步的挂载清单。
struct Lease {
    vm: Box<dyn GuestVm>,
    /// 这**一台** guest 实际用的共享后端：同步走它（不同租约可以是不同的档）。
    share: Arc<dyn ShareBackend + Send + Sync>,
    guard: Box<dyn ShareGuard>,
    mounts: Vec<GuestMount>,
    /// 启动时实际装载的运行包（id@version）：新子会话据此判断要不要重建。
    packages: Vec<String>,
    /// 这份 guest 是什么 OS：命令路径按它译成 guest 拼法。
    os: GuestOs,
    /// 模块入口的本地副本（(模块, 入口) → (大小, mtime, guest 落点)）：没变就不重复复制。
    execs: BTreeMap<(String, String), (u64, Option<SystemTime>, String)>,
}

/// 目的：虚拟机档能力——持 VM 引擎与**已实现的全部共享后端**，按会话租约管理 guest。
pub struct VmService {
    engine: Arc<dyn VmEngine + Send + Sync>,
    /// 已实现的共享后端（按优先序持有）：起 guest 时按 GuestSpec.shares 挑第一个落得上的。
    shares: Vec<Arc<dyn ShareBackend + Send + Sync>>,
    /// 租约 → 该租约的活体。每个活体自己一把锁：表锁只用来取/插，执行在**该租约自己的锁**下跑。
    guests: Mutex<BTreeMap<String, Arc<Mutex<Lease>>>>,
}

impl VmService {
    /// 目的：组合根专用——把已实现的共享后端都交进来（选哪一个不在这里定）。
    pub fn new(
        engine: Arc<dyn VmEngine + Send + Sync>,
        shares: Vec<Arc<dyn ShareBackend + Send + Sync>>,
    ) -> VmService {
        VmService {
            engine,
            shares,
            guests: Mutex::new(BTreeMap::new()),
        }
    }

    /// 目的：这次启动的共享后端候选：GuestSpec.shares 有就按它（按 id 认），没有就按本 service 持有的用户档。
    /// 约束：候选只收本 service 真持有的那些——传进来的 id 认不出就跳过（不 new 别人的适配器）。
    /// 约束：空候选只返回**非 emergency** 档；copy 只有在 GuestSpec.shares 显式写出时才可能被选中。
    fn candidates(&self, want: &[&'static str]) -> Vec<Arc<dyn ShareBackend + Send + Sync>> {
        if want.is_empty() {
            return self
                .shares
                .iter()
                .filter(|s| !s.is_emergency())
                .cloned()
                .collect();
        }
        want.iter()
            .filter_map(|id| self.shares.iter().find(|s| s.id() == *id).cloned())
            .collect()
    }
}

impl VmService {
    /// 目的：一次 guest 执行（可带 per-agent 围栏）——共享同步与命令通道在同一把租约锁下。
    fn run_guest(
        &self,
        lease: &str,
        run: &GuestRun,
        fence: Option<&crate::kernel::api::FenceSpec>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> ToolOutcome {
        let entry = match self.guests.lock() {
            Ok(guests) => guests.get(lease).cloned(),
            Err(_) => return fail("guest 表锁坏了"),
        };
        let Some(entry) = entry else {
            return fail(&format!("这次会话没有活着的 guest（租约 {}）", lease));
        };
        // 全局表锁只用来取 entry；执行在该租约自己的锁下跑——跨租约因此真并行（同一租约仍串行）。
        let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
        let held = &mut *held;
        let translated =
            crate::capabilities::vm::domain::guest::translate_run(run, held.os, &held.mounts);
        if let Err(why) = held.share.sync_in(held.vm.channel(), &held.mounts) {
            return fail(&format!("共享同步（进去）失败：{}", why));
        }
        let out = match fence {
            Some(fence) => {
                let mut access = crate::capabilities::vm::domain::guest::GuestAccess::from_fence(
                    fence,
                    held.os,
                    &held.mounts,
                );
                // 只读挂载的包前缀（/opt/...）也要进 per-agent 允许集：命令经 /usr/local/bin
                // 的软链调用包内解释器时，Landlock 检查的是目标路径。
                for m in &held.mounts {
                    if m.read_only && !m.guest.is_empty() {
                        access.ro_tree.push(m.guest.clone());
                    }
                }
                access.ro_tree.sort();
                access.ro_tree.dedup();
                held.vm
                    .channel()
                    .exec_fenced_cancellable(&translated, &access, cancel)
            }
            None => held.vm.channel().exec(&translated),
        };
        if let Err(why) = held.share.sync_out(held.vm.channel(), &held.mounts) {
            return fail(&format!("共享同步（回来）失败：{}", why));
        }
        out
    }
}

impl Vm for VmService {
    fn guest_start(&self, spec: &GuestSpec) -> Result<(), String> {
        if !self.engine.supports(&spec.image) {
            return Err(format!("引擎 {} 承载不了这份 guest 镜像", self.engine.id()));
        }
        // 选档在**这一刻**做：候选里第一个落得到这台引擎 + 这份 guest 上的就用它。
        // 因此同一进程里不同租约可以用不同的档，补齐作业装完共享工具后下一次启动就用上（不必重启）。
        let candidates = self.candidates(&spec.shares);
        let mut tried: Vec<&'static str> = Vec::new();
        let mut chosen: Option<Arc<dyn ShareBackend + Send + Sync>> = None;
        for share in &candidates {
            tried.push(share.id());
            if share.supports(self.engine.id(), &spec.image) {
                chosen = Some(Arc::clone(share));
                break;
            }
        }
        let Some(share) = chosen else {
            return Err(format!(
                "这份 guest 落不到引擎 {} 的任何共享后端上（试过：{}）",
                self.engine.id(),
                if tried.is_empty() {
                    "没有候选".to_string()
                } else {
                    tried.join("、")
                }
            ));
        };
        let (plan, guard) =
            share.prepare(self.engine.id(), &spec.mounts, spec.share_path.as_deref())?;
        // 拉起放在表锁之外：boot 要等 agent 起来（有界但可能要几十秒），别把别的租约的执行挡在表锁后面。
        let vm = self.engine.boot(spec, &plan)?;
        let entry = Arc::new(Mutex::new(Lease {
            vm,
            share,
            guard,
            mounts: spec.mounts.clone(),
            packages: Vec::new(),
            os: spec.image.os,
            execs: BTreeMap::new(),
        }));
        let previous = self
            .guests
            .lock()
            .map_err(|_| "guest 表锁坏了".to_string())?
            .insert(spec.lease.clone(), entry);
        // 同一租约重启：等它的在途执行收尾，再销毁旧的（幂等）。
        if let Some(old) = previous {
            stop_lease(&old);
        }
        Ok(())
    }

    fn guest_run(&self, lease: &str, run: &GuestRun) -> ToolOutcome {
        self.run_guest(lease, run, None, None)
    }

    fn guest_run_fenced(
        &self,
        lease: &str,
        run: &GuestRun,
        fence: &crate::kernel::api::FenceSpec,
    ) -> ToolOutcome {
        self.run_guest(lease, run, Some(fence), None)
    }

    fn guest_run_fenced_cancellable(
        &self,
        lease: &str,
        run: &GuestRun,
        fence: &crate::kernel::api::FenceSpec,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> ToolOutcome {
        self.run_guest(lease, run, Some(fence), cancel)
    }

    /// 目的：把模块入口复制进 guest 本地副本（共享盘不承载执行位）；同一文件没变就复用上次的落点。
    /// 约束：只接受模块内的相对入口；失败如实拒绝，不回退到共享路径执行。
    fn stage_exec(
        &self,
        lease: &str,
        module: &str,
        host: &std::path::Path,
        entry: &str,
    ) -> Result<String, String> {
        let rel = entry.trim_start_matches("./").replace('\\', "/");
        if rel.is_empty()
            || rel
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
        {
            return Err(format!(
                "模块入口不合法（只能是模块内的相对路径）：{}",
                entry
            ));
        }
        let meta = std::fs::metadata(host)
            .map_err(|e| format!("读不到模块入口（{}）：{}", host.display(), e))?;
        let stamp = (meta.len(), meta.modified().ok());
        let Some(entry) = self.guests.lock().ok().and_then(|g| g.get(lease).cloned()) else {
            return Err(format!("这次会话没有活着的 guest（租约 {}）", lease));
        };
        let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
        let held = &mut *held;
        if let Some((len, mtime, dest)) = held.execs.get(&(module.to_string(), rel.clone())) {
            if *len == stamp.0 && *mtime == stamp.1 {
                return Ok(dest.clone());
            }
        }
        let dest = guest::staged_entry_path(module, &rel);
        let src = guest::guest_path(host, held.os, &held.mounts);
        let out = held.vm.channel().exec(&GuestRun {
            command: guest::stage_script(&src, &dest),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: 30,
            output_limit: OutputLimit::Head,
        });
        if !out.ok {
            return Err(format!("把模块入口落进 guest 失败：{}", out.output.trim()));
        }
        held.execs
            .insert((module.to_string(), rel), (stamp.0, stamp.1, dest.clone()));
        Ok(dest)
    }

    fn guest_packages(&self, lease: &str) -> Option<Vec<String>> {
        let entry = self.guests.lock().ok()?.get(lease).cloned()?;
        let held = entry.lock().unwrap_or_else(|e| e.into_inner());
        Some(held.packages.clone())
    }

    fn guest_set_packages(&self, lease: &str, packages: Vec<String>) {
        let Some(entry) = self.guests.lock().ok().and_then(|g| g.get(lease).cloned()) else {
            return;
        };
        let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
        held.packages = packages;
    }

    fn guest_wait_agent(&self, lease: &str, timeout: std::time::Duration) -> bool {
        let entry = match self.guests.lock() {
            Ok(guests) => guests.get(lease).cloned(),
            Err(_) => return false,
        };
        let Some(entry) = entry else {
            return false;
        };
        let deadline = std::time::Instant::now() + timeout;
        loop {
            // 每探一次短拿一次租约锁：探针不长时间挡住同一租约的其它调用。
            {
                let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
                if held.vm.channel().ping() {
                    return true;
                }
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    fn guest_stop(&self, lease: &str) {
        let entry = match self.guests.lock() {
            Ok(mut guests) => guests.remove(lease),
            Err(_) => None,
        };
        if let Some(entry) = entry {
            stop_lease(&entry);
        }
    }

    fn guest_leases(&self) -> Vec<String> {
        self.guests
            .lock()
            .map(|g| g.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// 目的：一次 guest **只读诊断**——系统动作，走 helper 诊断模式（与 per-agent 围栏同一条通道）。
    /// 约束：不跑共享同步（诊断不进出文件）；失败如实区分原因，绝不返回空诊断。
    fn guest_diagnose(
        &self,
        lease: &str,
    ) -> Result<crate::capabilities::vm::api::GuestDiagnosis, String> {
        use crate::capabilities::vm::detail::guest_helper as helper;
        use crate::capabilities::vm::domain::diagnose as diag;
        let entry = self
            .guests
            .lock()
            .map_err(|_| "guest 表锁坏了".to_string())?
            .get(lease)
            .cloned()
            .ok_or_else(|| diag::no_live_guest(lease))?;
        let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
        let held = &mut *held;
        // Windows / macOS guest 本期没有 helper 诊断模式（只有 Linux guest 装了 helper）。
        if held.os != GuestOs::Linux {
            return Err(diag::not_linux(held.os.as_str()));
        }
        // 只读系统允许集：**不**按 agent 围栏，只给诊断要读的只读根（rw / private / cwd 全空）。
        let access = guest::GuestAccess {
            agent: "guest-diagnose".to_string(),
            rw: Vec::new(),
            ro: Vec::new(),
            ro_tree: ["/proc", "/sys", "/dev", "/etc"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            private: String::new(),
            cwd: String::new(),
            net: false,
            diagnose: true,
        };
        let run = GuestRun {
            command: String::new(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: diag::DIAGNOSE_TIMEOUT_SECS,
            output_limit: OutputLimit::Head,
        };
        let out = held.vm.channel().exec_fenced(&run, &access);
        if out.ok {
            // 支持诊断的 helper 只打一个 JSON；旧 helper 会把空命令交给 /bin/sh（退出 0、输出为空或非 JSON）。
            if !out.output.contains('{') {
                return Err(diag::helper_unavailable(&first_detail(&out.output)));
            }
            return diag::parse_report(&out.output, lease);
        }
        // 超时优先：helper 的机器标记，或宿主外层的等待预算到点。
        if out.output.contains(helper::HELPER_TIMEOUT_MARKER) || out.output.contains("超时") {
            return Err(diag::timed_out());
        }
        // helper 真进了诊断模式、但 Python 侧自己失败（前缀是诊断专用的）：原因在它的原话里。
        if out.output.contains(helper::HELPER_DIAGNOSE_PREFIX) {
            return Err(diag::exec_failed(&first_detail(&out.output)));
        }
        // 失败后的**有界预检**（命令自带 timeout 10）：区分 helper 不在 / 缺 python3 / 通道不通。
        let probe = held.vm.channel().exec(&GuestRun {
            command: helper::probe_command(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: String::new(),
            timeout_secs: diag::DIAGNOSE_TIMEOUT_SECS,
            output_limit: OutputLimit::Head,
        });
        let said = probe.output.trim().to_string();
        if said.contains(helper::PROBE_NO_HELPER) {
            return Err(diag::helper_unavailable(&first_detail(&said)));
        }
        if said.contains(helper::PROBE_NO_PYTHON) {
            return Err(diag::python_missing(&first_detail(&said)));
        }
        if probe.ok && said.contains(helper::PROBE_OK) {
            return Err(diag::exec_failed(&first_detail(&out.output)));
        }
        let detail = first_detail(&said);
        Err(diag::channel_down(if detail.is_empty() {
            "guest 没有回应诊断预检"
        } else {
            &detail
        }))
    }
}

/// 目的：失败现场只留第一行（QGA / helper 的原话，后面的行多是重复输出）。
fn first_detail(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > 200 {
        line.chars().take(200).collect()
    } else {
        line.to_string()
    }
}

/// 目的：销毁一份租约（杀 guest + 放共享凭据）；等它的在途执行收尾，幂等。
fn stop_lease(entry: &Arc<Mutex<Lease>>) {
    let mut held = entry.lock().unwrap_or_else(|e| e.into_inner());
    held.vm.stop();
    held.guard.release();
}

fn fail(why: &str) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: why.to_string(),
    }
}
