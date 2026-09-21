//! 测试层（T1 单元 + T2 端口与适配器契约）——替身与契约测试同处一层。
//! 目录与命名见 docs/testing/gaps-acceptance.md，替身语义见 docs/testing/doubles.md，
//! 层级与判定见 docs/testing/levels.md，端口矩阵见 docs/testing/port-matrix.md。
//! 硬规矩：不碰真实 `.home/`、真实 `session/`、真实权限或外部网络；只绑本地环回。

mod adapters;
mod api;
mod core;
mod doubles;
mod fakes;
mod intent;
mod ports;
mod routes;

/// 隔离落点：`target/test-scratch/contract/<name>`（target/ 不入库）。每次先清空再建。
pub(crate) fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-scratch")
        .join("contract")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("建契约测试隔离根");
    d
}

/// 起一个内存装配的核心手柄并把能力面拆成 `Ops`（契约测试共用）。
/// 返回手柄是为了能观察事件台与测试注入；只用能力面的用例可以忽略它。
pub(crate) fn ops_with(
    modules: Vec<crate::core::module::Module>,
    core_script: Vec<&str>,
) -> (crate::core::api::CoreHandle, crate::core::api::Ops) {
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"好\"}".to_string()],
    );
    let gateway = doubles::gw(member, core_script.into_iter().map(String::from).collect());
    let handle = crate::core::api::CoreHandle::spawn(doubles::core_with_gateway(modules, gateway))
        .expect("起核心线程");
    let ops = crate::core::api::Ops::from_handle(&handle);
    (handle, ops)
}

/// 一次单 agent 工作的规格（契约测试共用）。
pub(crate) fn single_work(name: &str, modules: &[&str]) -> crate::core::WorkSpec {
    crate::core::WorkSpec {
        name: name.to_string(),
        mode: crate::core::WorkMode::Single,
        agents: vec![crate::core::AgentInstance {
            name: modules[0].to_string(),
            transient: true,
            modules: modules.iter().map(|s| s.to_string()).collect(),
            model: None,
        }],
        task: None,
        delegate: false,
    }
}

/// 边流边等停止的通道：把「生成中」变成可观察状态——取消后下一次回调即返回 false。
/// 上限约 30 秒：只要提前返回，就说明是「停止」生效而不是它自然跑完。
pub(crate) struct SlowChat {
    pub ticks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::core::ports::Chat for SlowChat {
    fn complete(
        &mut self,
        _m: &[crate::core::ports::Msg],
        _opts: crate::core::ports::CompleteOpts<'_>,
        on: &mut dyn FnMut(crate::core::ports::Chunk) -> bool,
    ) -> crate::core::ports::Completion {
        use std::sync::atomic::Ordering;
        if !on(crate::core::ports::Chunk::Start) {
            return crate::core::ports::Completion::text("");
        }
        for _ in 0..6_000 {
            self.ticks.fetch_add(1, Ordering::Relaxed);
            if !on(crate::core::ports::Chunk::Text("·".to_string())) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        crate::core::ports::Completion::text("{\"type\":\"say\",\"text\":\"（慢通道）收到停止\"}")
    }
}

/// 阻塞到被放行的通道：把"生成中"变成**可观察且可控**的状态。
/// 为什么需要它：协作的泵目前**没有取消检查**（见缺口账），只能靠放行来结束，
/// 否则测试会一直等到整段讨论跑完。
pub(crate) struct GatedChat {
    pub started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub release: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl crate::core::ports::Chat for GatedChat {
    fn complete(
        &mut self,
        _m: &[crate::core::ports::Msg],
        _opts: crate::core::ports::CompleteOpts<'_>,
        on: &mut dyn FnMut(crate::core::ports::Chunk) -> bool,
    ) -> crate::core::ports::Completion {
        use std::sync::atomic::Ordering;
        self.started.fetch_add(1, Ordering::Relaxed);
        if !on(crate::core::ports::Chunk::Start) {
            return crate::core::ports::Completion::text("");
        }
        // 等放行；但**也要尊重分片回调**——「停止」正是靠 on 返回 false 在调用中途生效的。
        while !self.release.load(Ordering::Relaxed) {
            if !on(crate::core::ports::Chunk::Text("…".to_string())) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        crate::core::ports::Completion::text("{\"type\":\"agree\",\"text\":\"同意\"}")
    }
}

/// 一律发"阻塞到放行"通道的网关。
pub(crate) struct GatedGateway {
    pub started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub release: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl crate::core::ports::ChatGateway for GatedGateway {
    fn probe_tools(
        &self,
        _c: &crate::core::providers::Channel,
    ) -> Result<crate::core::ports::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(
        &self,
        _c: Option<&crate::core::providers::Channel>,
        _id: &str,
    ) -> (crate::core::ports::BoxedChat, Option<String>) {
        (
            Box::new(GatedChat {
                started: std::sync::Arc::clone(&self.started),
                release: std::sync::Arc::clone(&self.release),
            }),
            None,
        )
    }
    fn core_channel(
        &self,
        _c: Option<&crate::core::providers::Channel>,
    ) -> (crate::core::ports::BoxedChat, bool) {
        (
            Box::new(GatedChat {
                started: std::sync::Arc::clone(&self.started),
                release: std::sync::Arc::clone(&self.release),
            }),
            false,
        )
    }
}

/// 装配一个「生成阻塞到放行」的核心（观察"生成期间读接口不排队"）。
pub(crate) fn gated_ops(
    modules: Vec<crate::core::module::Module>,
) -> (
    crate::core::api::CoreHandle,
    crate::core::api::Ops,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let gateway = GatedGateway {
        started: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        release: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let started = std::sync::Arc::clone(&gateway.started);
    let release = std::sync::Arc::clone(&gateway.release);
    let handle = crate::core::api::CoreHandle::spawn(doubles::core_with_gateway(modules, gateway))
        .expect("起核心线程");
    let ops = crate::core::api::Ops::from_handle(&handle);
    (handle, ops, started, release)
}

/// 一律发慢通道的网关（可选网关参数：慢通道要自定义时序，`ops_with` 覆盖不了）。
pub(crate) struct SlowGateway {
    pub ticks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::core::ports::ChatGateway for SlowGateway {
    fn probe_tools(
        &self,
        _c: &crate::core::providers::Channel,
    ) -> Result<crate::core::ports::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(
        &self,
        _c: Option<&crate::core::providers::Channel>,
        _id: &str,
    ) -> (crate::core::ports::BoxedChat, Option<String>) {
        (
            Box::new(SlowChat {
                ticks: std::sync::Arc::clone(&self.ticks),
            }),
            None,
        )
    }
    fn core_channel(
        &self,
        _c: Option<&crate::core::providers::Channel>,
    ) -> (crate::core::ports::BoxedChat, bool) {
        (
            Box::new(SlowChat {
                ticks: std::sync::Arc::clone(&self.ticks),
            }),
            false,
        )
    }
}

/// 装配一个「生成会一直跑到被停止」的核心（观察 is_running / stop / 编辑禁令）。
pub(crate) fn slow_ops(
    modules: Vec<crate::core::module::Module>,
) -> (
    crate::core::api::CoreHandle,
    crate::core::api::Ops,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let gateway = SlowGateway {
        ticks: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let ticks = std::sync::Arc::clone(&gateway.ticks);
    let handle = crate::core::api::CoreHandle::spawn(doubles::core_with_gateway(modules, gateway))
        .expect("起核心线程");
    let ops = crate::core::api::Ops::from_handle(&handle);
    (handle, ops, ticks)
}
