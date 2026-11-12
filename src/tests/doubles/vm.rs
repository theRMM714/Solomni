//! 目的：虚拟机引擎与 guest 通道替身——不起真 VM，只记录启动/执行/停止。
//! 管：启动规格与延迟、挂载与共享后端、guest 通道执行、诊断回放。
//! 不管：真实引擎（`src/capabilities/vm/detail/qemu.rs`）、工作区（`workspace.rs`）。
//! 联动：`src/capabilities/vm/ports.rs` 与 `src/capabilities/vm/api.rs`。

use super::*;

/// 目的：VM 引擎的测试替身——不起真 VM，只记录 start / run / stop。
/// 约束：`boot_delay` 让"起 guest 很慢"可复现（验核心线程不被占）；`boot_error` 让起不来可复现（验失败不留痕）；`fenced_output` 给围栏执行预设输出（None = 恒 `fake-guest`，默认行为不变），诊断用例用它给回一份 JSON；`specs` 记每次 `boot` 的启动规格——**挂载计划**（哪些宿主根真的挂进 guest）只在引擎边界看得见。
#[derive(Clone, Default)]
pub(crate) struct FakeEngine {
    pub calls: Arc<Mutex<Vec<String>>>,
    pub boot_delay: Option<std::time::Duration>,
    pub boot_error: Option<String>,
    pub fenced_output: Option<String>,
    // 每次 `boot()` 收到的 `GuestSpec` 快照（含 `mounts`；失败尝试也记——"试过哪个计划"同样是事实）。
    pub specs: Arc<Mutex<Vec<crate::capabilities::vm::domain::guest::GuestSpec>>>,
}

impl FakeEngine {
    /// 目的：空记录起步。
    pub(crate) fn new() -> FakeEngine {
        FakeEngine::default()
    }
}

impl crate::capabilities::vm::ports::VmEngine for FakeEngine {
    fn id(&self) -> &'static str {
        "fake"
    }

    fn supports(&self, _guest: &crate::capabilities::vm::domain::guest::GuestImage) -> bool {
        true
    }

    fn boot(
        &self,
        spec: &crate::capabilities::vm::domain::guest::GuestSpec,
        _share: &crate::capabilities::vm::ports::SharePlan,
    ) -> Result<Box<dyn crate::capabilities::vm::ports::GuestVm>, String> {
        if let Some(delay) = self.boot_delay {
            std::thread::sleep(delay);
        }
        self.specs.lock().expect("锁").push(spec.clone());
        if let Some(why) = &self.boot_error {
            return Err(why.clone());
        }
        self.calls
            .lock()
            .expect("锁")
            .push(format!("start:{}", spec.lease));
        Ok(Box::new(FakeVm {
            channel: FakeChannel {
                lease: spec.lease.clone(),
                calls: self.calls.clone(),
                fenced_output: self.fenced_output.clone(),
            },
            lease: spec.lease.clone(),
            calls: self.calls.clone(),
        }))
    }
}

/// 目的：共享后端的测试替身——恒可落、计划为空；记录 prepare / sync_in / sync_out / release。
#[derive(Clone, Default)]
pub(crate) struct FakeShare {
    pub calls: Arc<Mutex<Vec<String>>>,
}

impl FakeShare {
    /// 目的：空记录起步。
    pub(crate) fn new() -> FakeShare {
        FakeShare::default()
    }
}

impl crate::capabilities::vm::ports::ShareBackend for FakeShare {
    fn id(&self) -> &'static str {
        "fake"
    }

    fn supports(
        &self,
        _engine: &str,
        _guest: &crate::capabilities::vm::domain::guest::GuestImage,
    ) -> bool {
        true
    }

    fn prepare(
        &self,
        _engine: &str,
        _mounts: &[crate::capabilities::vm::domain::guest::GuestMount],
        _share_tool: Option<&str>,
    ) -> Result<
        (
            crate::capabilities::vm::ports::SharePlan,
            Box<dyn crate::capabilities::vm::ports::ShareGuard>,
        ),
        String,
    > {
        self.calls.lock().expect("锁").push("prepare".to_string());
        Ok((
            crate::capabilities::vm::ports::SharePlan {
                kind: "fake",
                engine_args: Vec::new(),
                net_options: Vec::new(),
                guest_scripts: Vec::new(),
                needs_net: false,
                needs_shared_mem: false,
            },
            Box::new(FakeGuard {
                calls: self.calls.clone(),
            }),
        ))
    }

    fn sync_in(
        &self,
        _channel: &mut dyn crate::capabilities::vm::ports::GuestChannel,
        _mounts: &[crate::capabilities::vm::domain::guest::GuestMount],
    ) -> Result<(), String> {
        self.calls.lock().expect("锁").push("sync_in".to_string());
        Ok(())
    }

    fn sync_out(
        &self,
        _channel: &mut dyn crate::capabilities::vm::ports::GuestChannel,
        _mounts: &[crate::capabilities::vm::domain::guest::GuestMount],
    ) -> Result<(), String> {
        self.calls.lock().expect("锁").push("sync_out".to_string());
        Ok(())
    }
}

/// 目的：带名字与"落得上吗"开关的共享替身——验起 guest 时按候选挑档、同步走**这一台**选中的那一档。
pub(crate) struct NamedShare {
    pub(crate) name: &'static str,
    pub(crate) ok: bool,
    pub(crate) emergency: bool,
    pub(crate) calls: Arc<Mutex<Vec<String>>>,
}

impl NamedShare {
    /// 目的：建一个替身：`name` 是它的 id，`ok` 是 `supports` 的答案。
    pub(crate) fn new(name: &'static str, ok: bool) -> NamedShare {
        NamedShare {
            name,
            ok,
            emergency: false,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 目的：把它标成 emergency 档（验空候选不自动选、显式候选才允许）。
    pub(crate) fn emergency(mut self) -> NamedShare {
        self.emergency = true;
        self
    }

    /// 目的：它被调用过的动作（按发生顺序）。
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("锁").clone()
    }
}

impl crate::capabilities::vm::ports::ShareBackend for NamedShare {
    fn id(&self) -> &'static str {
        self.name
    }

    fn is_emergency(&self) -> bool {
        self.emergency
    }

    fn supports(
        &self,
        _engine: &str,
        _guest: &crate::capabilities::vm::domain::guest::GuestImage,
    ) -> bool {
        self.ok
    }

    fn prepare(
        &self,
        _engine: &str,
        _mounts: &[crate::capabilities::vm::domain::guest::GuestMount],
        _share_tool: Option<&str>,
    ) -> Result<
        (
            crate::capabilities::vm::ports::SharePlan,
            Box<dyn crate::capabilities::vm::ports::ShareGuard>,
        ),
        String,
    > {
        self.calls.lock().expect("锁").push("prepare".to_string());
        Ok((
            crate::capabilities::vm::ports::SharePlan {
                kind: self.name,
                engine_args: Vec::new(),
                net_options: Vec::new(),
                guest_scripts: Vec::new(),
                needs_net: false,
                needs_shared_mem: false,
            },
            Box::new(crate::tests::doubles::FakeGuard2 {
                calls: self.calls.clone(),
            }),
        ))
    }

    fn sync_in(
        &self,
        _channel: &mut dyn crate::capabilities::vm::ports::GuestChannel,
        _mounts: &[crate::capabilities::vm::domain::guest::GuestMount],
    ) -> Result<(), String> {
        self.calls.lock().expect("锁").push("sync_in".to_string());
        Ok(())
    }

    fn sync_out(
        &self,
        _channel: &mut dyn crate::capabilities::vm::ports::GuestChannel,
        _mounts: &[crate::capabilities::vm::domain::guest::GuestMount],
    ) -> Result<(), String> {
        self.calls.lock().expect("锁").push("sync_out".to_string());
        Ok(())
    }
}

/// 目的：NamedShare 的凭据替身——释放记一笔（与 FakeGuard 同形，不复用是为了不共用同一个记录）。
pub(crate) struct FakeGuard2 {
    pub(crate) calls: Arc<Mutex<Vec<String>>>,
}

impl crate::capabilities::vm::ports::ShareGuard for FakeGuard2 {
    fn release(&mut self) {
        self.calls.lock().expect("锁").push("release".to_string());
    }
}

/// 目的：共享凭据替身——释放记录一笔。
struct FakeGuard {
    calls: Arc<Mutex<Vec<String>>>,
}

impl crate::capabilities::vm::ports::ShareGuard for FakeGuard {
    fn release(&mut self) {
        self.calls.lock().expect("锁").push("release".to_string());
    }
}

struct FakeVm {
    channel: FakeChannel,
    lease: String,
    calls: Arc<Mutex<Vec<String>>>,
}

struct FakeChannel {
    lease: String,
    calls: Arc<Mutex<Vec<String>>>,
    /// 目的：围栏执行的预设输出（None = 恒 `fake-guest`）：诊断用例给它一份 helper 形状的 JSON。
    fenced_output: Option<String>,
}

impl crate::capabilities::vm::ports::GuestChannel for FakeChannel {
    /// 目的：替身通道就当 agent 已在——等待用例不必真等。
    fn ping(&mut self) -> bool {
        true
    }

    fn exec(
        &mut self,
        run: &crate::capabilities::vm::domain::guest::GuestRun,
    ) -> crate::kernel::api::ToolOutcome {
        self.calls
            .lock()
            .expect("锁")
            .push(format!("run:{}:{}", self.lease, run.command));
        crate::kernel::api::ToolOutcome {
            ok: true,
            output: "fake-guest".to_string(),
        }
    }

    fn exec_fenced(
        &mut self,
        _run: &crate::capabilities::vm::domain::guest::GuestRun,
        access: &crate::capabilities::vm::domain::guest::GuestAccess,
    ) -> crate::kernel::api::ToolOutcome {
        // 记录形状沿用既有前缀（既有用例按 `fenced:<租约>:<agent>:<cwd>:<rw>` 断言），
        // 后面再追加只读根与诊断标记——诊断用例据此断言「走的是只读系统允许集」。
        self.calls.lock().expect("锁").push(format!(
            "fenced:{}:{}:{}:{}:{}:{}",
            self.lease,
            access.agent,
            access.cwd,
            access.rw.join(","),
            access.ro_tree.join(","),
            access.diagnose
        ));
        let output = self
            .fenced_output
            .clone()
            .unwrap_or_else(|| "fake-guest".to_string());
        crate::kernel::api::ToolOutcome { ok: true, output }
    }
}

impl crate::capabilities::vm::ports::GuestVm for FakeVm {
    fn channel(&mut self) -> &mut dyn crate::capabilities::vm::ports::GuestChannel {
        &mut self.channel
    }

    fn stop(&mut self) {
        self.calls
            .lock()
            .expect("锁")
            .push(format!("stop:{}", self.lease));
    }
}

/// 目的：vm 用例面的测试替身——记录 guest_run，恒成功（验证"命令送进了 guest"）。
#[derive(Default)]
pub(crate) struct RecordingVm {
    pub runs: Mutex<Vec<String>>,
    // fenced 执行的（脚本、超时、输出截断方式）；shell 契约断言用。
    pub fenced_runs: Mutex<
        Vec<(
            String,
            u64,
            crate::capabilities::vm::domain::guest::OutputLimit,
        )>,
    >,
    // 已拉起的租约（`guest_leases` 的事实来源；`guest_start` 记、`guest_stop` 删）。
    pub leases: Mutex<Vec<String>>,
    // 每个租约启动时装载的运行包（判定新子会话要不要重建）。
    pub packages: Mutex<std::collections::BTreeMap<String, Vec<String>>>,
    // fenced 执行的返回值覆盖（None = 恒成功）；超时 / 停止回执的成员侧断言用。
    pub fenced_result: Mutex<Option<crate::kernel::api::ToolOutcome>>,
}

impl crate::capabilities::vm::api::Vm for RecordingVm {
    fn guest_start(
        &self,
        spec: &crate::capabilities::vm::domain::guest::GuestSpec,
    ) -> Result<(), String> {
        self.leases.lock().expect("锁").push(spec.lease.clone());
        Ok(())
    }

    fn guest_run(
        &self,
        _lease: &str,
        run: &crate::capabilities::vm::domain::guest::GuestRun,
    ) -> crate::kernel::api::ToolOutcome {
        self.runs.lock().expect("锁").push(run.command.clone());
        crate::kernel::api::ToolOutcome {
            ok: true,
            output: "guest".to_string(),
        }
    }

    fn guest_run_fenced(
        &self,
        _lease: &str,
        run: &crate::capabilities::vm::domain::guest::GuestRun,
        _fence: &crate::kernel::api::FenceSpec,
    ) -> crate::kernel::api::ToolOutcome {
        self.runs
            .lock()
            .expect("锁")
            .push(format!("fenced:{}", run.command));
        self.fenced_runs.lock().expect("锁").push((
            run.command.clone(),
            run.timeout_secs,
            run.output_limit,
        ));
        self.fenced_result
            .lock()
            .expect("锁")
            .clone()
            .unwrap_or(crate::kernel::api::ToolOutcome {
                ok: true,
                output: "guest".to_string(),
            })
    }

    fn stage_exec(
        &self,
        _lease: &str,
        _module: &str,
        _host: &std::path::Path,
        entry: &str,
    ) -> Result<String, String> {
        // 替身没有 guest 本地副本：按声明的入口原样返回（调用方重写后命令不变）。
        Ok(entry.to_string())
    }

    fn guest_packages(&self, lease: &str) -> Option<Vec<String>> {
        self.packages.lock().expect("锁").get(lease).cloned()
    }

    fn guest_set_packages(&self, lease: &str, packages: Vec<String>) {
        self.packages
            .lock()
            .expect("锁")
            .insert(lease.to_string(), packages);
    }

    fn guest_wait_agent(&self, _lease: &str, _timeout: std::time::Duration) -> bool {
        true
    }

    fn guest_stop(&self, lease: &str) {
        self.leases.lock().expect("锁").retain(|l| l != lease);
        self.packages.lock().expect("锁").remove(lease);
    }

    fn guest_leases(&self) -> Vec<String> {
        self.leases.lock().expect("锁").clone()
    }
}

/// 目的：测试用的 **vm 能力面**——把 guest 替身装进 `VmService`（与生产同一条路，R12）。
/// 约束：共享档替身同时持 `9p` 与 `smb`（都是用户链里的挂载式档，不需要外部工具），Linux/macOS 的真实候选能挑到 9p，Windows 能挑到 smb；copy 是 emergency 档，空候选不会被自动选中。
pub(crate) fn test_vm() -> Arc<dyn crate::capabilities::vm::api::Vm + Send + Sync> {
    Arc::new(crate::capabilities::vm::service::VmService::new(
        Arc::new(FakeEngine::new()),
        vec![
            Arc::new(NamedShare::new("9p", true)),
            Arc::new(NamedShare::new("smb", true)),
        ],
    ))
}

/// 目的：带可控"起 guest 耗时 / 失败"的 vm 能力面——验核心不被慢步骤占住、失败不留痕。
pub(crate) fn test_vm_with_engine(
    engine: FakeEngine,
) -> Arc<dyn crate::capabilities::vm::api::Vm + Send + Sync> {
    Arc::new(crate::capabilities::vm::service::VmService::new(
        Arc::new(engine),
        vec![
            Arc::new(NamedShare::new("9p", true)),
            Arc::new(NamedShare::new("smb", true)),
        ],
    ))
}

/// 目的：测试用的 **packages 能力面**——把内存包库装进 `PackagesService`（与生产同一条路，R12）。
pub(crate) fn test_packages(
    packages: Arc<dyn PackageSource + Send + Sync>,
) -> Arc<dyn crate::capabilities::packages::api::Packages + Send + Sync> {
    Arc::new(
        crate::capabilities::packages::service::PackagesService::with_base(
            packages,
            crate::capabilities::packages::api::BaseFacts::default(),
        ),
    )
}

/// 目的：测试用的 packages 能力面，带"宿主回退可用"的事实（由测试 probe 声明它有什么）。
pub(crate) fn test_packages_with_base(
    packages: Arc<dyn PackageSource + Send + Sync>,
    base: crate::capabilities::packages::api::BaseFacts,
) -> Arc<dyn crate::capabilities::packages::api::Packages + Send + Sync> {
    Arc::new(crate::capabilities::packages::service::PackagesService::with_base(packages, base))
}

/// 目的：模块启停覆盖的内存存储（测试用；与 yaml 实现同一条端口）。
#[derive(Default)]
pub(crate) struct InMemoryModuleState {
    pub(crate) state: Mutex<crate::capabilities::modules::domain::state::EnabledState>,
}
