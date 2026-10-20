//! 目的：一段"要等 guest"的活（起 / 停），**在调用方线程上跑**——核心只负责准备参数与提交结果。
//! 管：起 guest 前的并发准入与规格装配、起之后的运行包软链；停 guest 的租约清单。
//! 不管：会话建/改/删的校验与落盘（在 work.rs / history.rs 的 prepare、commit 两段里）。
//! 联动：由 `Conductor::guest_boot` / `history_delete_bookkeeping` 产出，`api` 层的句柄在调用方线程上 `run()`；
//!   契约见 docs/conductor/README.md 的「长步骤」与 docs/vm/README.md 的「并发」。

use crate::capabilities::vm::api::{
    admit_guest, classify_base, mounts_from, package_setup_scripts, resolve_guest, ExecPlan,
    GuestRun, GuestSpec, Vm,
};
use crate::capabilities::workspace::api::Sandboxes;
use crate::kernel::api::FenceSpec;
use crate::kernel::ports::{HostProbe, Log};
use std::path::PathBuf;
use std::sync::Arc;

/// 目的：一次 guest 启动的全部参数（核心准备好，调用方线程执行）。
/// 约束：字段全部是拥有所有权的数据或 `Arc`——它要跨线程，不能借用核心状态。
pub struct GuestBoot {
    vm: Arc<dyn Vm + Send + Sync>,
    root: PathBuf,
    probe: Arc<dyn HostProbe + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
    /// 同时台上限（0 = 不限制）；到上限在 run 里如实拒绝。
    max_guests: u32,
    /// 用户授权的只读根（进围栏规格，与工具执行那一份同源）。
    fence_read: Vec<PathBuf>,
    engine_path: Option<String>,
    share_path: Option<String>,
    shares: Vec<&'static str>,
    lease: String,
    /// 这条会话要拉起的 guest 规格里与"会话"有关的那几项。
    net: bool,
    plan: ExecPlan,
    sandboxes: Sandboxes,
    /// 拉起之前先停掉同一租约已有的 guest（改配要重建）。
    restart: bool,
}

impl GuestBoot {
    /// 目的：组装备好的那一段（核心侧调用，毫秒级）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        vm: Arc<dyn Vm + Send + Sync>,
        root: PathBuf,
        probe: Arc<dyn HostProbe + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        max_guests: u32,
        fence_read: Vec<PathBuf>,
        engine_path: Option<String>,
        share_path: Option<String>,
        shares: Vec<&'static str>,
        lease: String,
        net: bool,
        plan: ExecPlan,
        sandboxes: Sandboxes,
        restart: bool,
    ) -> GuestBoot {
        GuestBoot {
            vm,
            root,
            probe,
            log,
            max_guests,
            fence_read,
            engine_path,
            share_path,
            shares,
            lease,
            net,
            plan,
            sandboxes,
            restart,
        }
    }

    /// 目的：这段启动要占的 guest 租约（调用方据此串行同一台 guest 的启动）。
    pub(crate) fn lease(&self) -> &str {
        &self.lease
    }

    /// 目的：真正把 guest 拉起来（慢：QEMU 启动 + 等 QGA 就绪；改配时先停旧的）。
    /// 错误：准入到上限、虚拟根不可用、引擎起不来都如实返回；此时**没有任何状态被提交**。
    pub fn run(self) -> Result<(), String> {
        let GuestBoot {
            vm,
            root,
            probe,
            log,
            max_guests,
            fence_read,
            engine_path,
            share_path,
            shares,
            lease,
            net,
            plan,
            sandboxes,
            restart,
        } = self;
        // 并发闸门：每台 guest 固定 -m 2048 -smp 2，按设置里的上限如实拒绝（同一租约重启不占新名额）。
        admit_guest(&vm.guest_leases(), &lease, max_guests)?;
        // 另一个请求可能刚把它启动好：非重启语义下不再拉第二台（否则会把刚起来的替换掉）。
        if !restart && vm.guest_leases().iter().any(|l| l == &lease) {
            return Ok(());
        }
        // 改配：先停掉这个租约的旧 guest（同一租约不并存两台）。
        if restart {
            vm.guest_stop(&lease);
        }
        let name = plan
            .guest
            .as_deref()
            .ok_or_else(|| "这条虚拟机档会话没有选定虚拟根".to_string())?;
        let dir = resolve_guest(&root, name)?;
        let image = classify_base(&dir, probe.as_ref())?;
        let mut rw: Vec<PathBuf> = Vec::new();
        let mut ro: Vec<PathBuf> = Vec::new();
        let mut ro_tree: Vec<PathBuf> = Vec::new();
        for sb in &sandboxes.list {
            let f = FenceSpec::from_sandbox(sb, net).with_read_only(fence_read.clone());
            rw.extend(f.rw);
            ro.extend(f.ro);
            ro_tree.extend(f.ro_tree);
        }
        let fence = FenceSpec {
            agent: String::new(),
            lease: lease.clone(),
            rw,
            ro: ro.clone(),
            ro_tree,
            private: PathBuf::new(),
            cwd: PathBuf::new(),
            net,
        };
        // 挂载点走那座桥：同方言时是宿主原路径，跨方言时是 guest 镜像根。
        let bridge = image.bridge();
        let mounts = mounts_from(&plan, &fence, &bridge);
        let spec = GuestSpec {
            lease: lease.clone(),
            image,
            mounts,
            net,
            engine_path,
            share_path,
            shares,
            seed: None,
            // 会话要每租约可写层：多台并发不能同写一份基础镜像。
            isolated: true,
        };
        vm.guest_start(&spec)?;
        // 已装载的 prefix 包：把 `<prefix>/bin/*` 软链进 guest 的 /usr/local/bin，
        // 这样"模块声明 runtimes: [X] + 裸名命令"在 guest 里真的能跑起来（失败如实记一条，不假装装上了）。
        for script in package_setup_scripts(&plan) {
            let out = vm.guest_run(
                &lease,
                &GuestRun {
                    command: script,
                    args_json: "{}".to_string(),
                    env: Vec::new(),
                    cwd: String::new(),
                },
            );
            if !out.ok {
                log.warn(
                    "conductor::guest_boot",
                    &format!("运行包的 PATH 软链没建上：{}", out.output),
                );
            }
        }
        Ok(())
    }
}

/// 目的：一次 guest 销毁（慢：先请 guest 断电并等它落盘，超时才杀）——同样在调用方线程上跑。
pub struct GuestStop {
    vm: Arc<dyn Vm + Send + Sync>,
    leases: Vec<String>,
}

impl GuestStop {
    /// 目的：按租约清单销毁（幂等）。
    pub(crate) fn new(vm: Arc<dyn Vm + Send + Sync>, leases: Vec<String>) -> GuestStop {
        GuestStop { vm, leases }
    }

    /// 目的：执行销毁——这里要等 guest 落盘，别在核心线程上跑。
    pub fn run(self) {
        for lease in &self.leases {
            self.vm.guest_stop(lease);
        }
    }
}

/// 目的：改配时要等 guest 的那一段——切回本机档只停；切入或留在虚拟机档要起（可能先停旧的）。
pub enum EditGuest {
    /// 起或重建 guest（`restart` = 先停同一租约的旧 guest）。
    Boot(Box<GuestBoot>),
    /// 切回本机档：把旧的 guest 销毁。
    Stop(GuestStop),
}

impl EditGuest {
    /// 目的：执行那一段（慢：QEMU 启动 / 关机落盘），在调用方线程上跑。
    pub fn run(self) -> Result<(), String> {
        match self {
            EditGuest::Boot(boot) => boot.run(),
            EditGuest::Stop(stop) => {
                stop.run();
                Ok(())
            }
        }
    }
}
