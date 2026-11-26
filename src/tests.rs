//! 目的：装配测试层（T1 单元 + T2 端口契约）的公共前置与替身。
//! 管：root_dir、test_guest、scratch、ops_with 等夹具；Chat/Probe 替身；slow_ops 等驱动辅助。
//! 不管：真实 .home/、真实 session/、真实权限或外网；业务断言归各测试文件。
//! 联动：docs/testing/gaps-acceptance.md、docs/testing/doubles.md、docs/testing/levels.md。

mod api;
pub(crate) mod boot;
mod builders;
mod builds;
mod cli;
mod collab;
mod commands;
mod detail;
mod diagnostics;
pub(crate) mod doubles;
mod entry;
mod env;
mod fakes;
mod fence_reach;
mod flow;
mod kernel;
mod lifecycle;
mod llm;
mod mcp;
mod modules;
mod op;
mod packages;
mod permission;
mod ports;
mod prelude;
mod prompt;
mod provision;
mod proxy;
mod registry;
mod residents;
mod routes;
mod runtime;
mod secrets;
mod session;
mod setup;
mod shell;
mod slate;
mod startup;
mod storage;
mod taskchain;
mod toolchains;
mod tools;
mod vm;
mod workspace;

/// 目的：测试用的**产品根**——当前工作目录（测试在仓库根跑，虚拟根就解析到 `data/.vm/guest/<名>` 这条相对路径上）。
pub(crate) fn root_dir() -> std::path::PathBuf {
    std::env::current_dir().expect("取当前目录")
}

/// 目的：造一个测试用的**虚拟根**（`data/.vm/guest/<名>` + 过布局判定的占位磁盘镜像与内核），返回它的名字。
/// 约束：只碰这一格，收尾删掉它（不清父目录——本机可能真装了别的虚拟根）。
pub(crate) fn test_guest(name: &str) -> String {
    let dir = crate::kernel::domain::layout::vm(&root_dir())
        .join("guest")
        .join(name);
    std::fs::create_dir_all(&dir).expect("建虚拟根目录");
    std::fs::write(dir.join("rootfs.ext4"), b"img").expect("写占位磁盘镜像");
    std::fs::write(dir.join("vmlinuz"), b"stub").expect("写占位内核");
    // 测试虚拟根声明三种客户端：Linux 用 9p/virtiofs，Windows 用 cifs——同一份夹具在三平台都能承载。
    std::fs::write(
        dir.join("guest.yaml"),
        format!("id: {name}\nclients: [9p, virtiofs, cifs]\n"),
    )
    .expect("写 guest.yaml");
    name.to_string()
}

/// 目的：隔离落点 `target/test-scratch/contract/<name>`（target/ 不入库）。每次先清空再建。
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

/// 目的：真机围栏用例的**串行闸**。写/还原目录 DACL 是本机全局副作用——可继承 ACE 会触发
///   整树继承传播，快照还原会改写同一祖先下别的用例正在用的目录；并发跑会互相打断
///   （实测：一个用例还原祖先快照时，另一个正在跑的容器打不开自己的上级目录）。
/// 约束：只给 `#[cfg(windows)]` 的用例用；持锁期间可以起线程（锁不在子线程里重复取）。
#[cfg(windows)]
pub(crate) fn fence_acl_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 目的：起一个内存装配的核心手柄并把入站面拆成 `Faces`（契约测试共用）。
///   返回手柄是为了能观察事件台与测试注入；只用入站面的用例可以忽略它。
pub(crate) fn ops_with(
    modules: Vec<crate::units::business::catalog::modules::api::Module>,
    core_script: Vec<&str>,
) -> (
    crate::units::business::runtime::api::Handle,
    crate::presentation::faces::Faces,
) {
    let mut member = std::collections::BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"好\"}".to_string()],
    );
    let gateway = doubles::gw(member, core_script.into_iter().map(String::from).collect());
    let handle = crate::units::business::runtime::api::Handle::spawn(doubles::core_with_gateway(
        modules, gateway,
    ))
    .expect("起核心线程");
    let faces = crate::presentation::faces::Faces::from_handle(&handle);
    (handle, faces)
}

/// 目的：注入**指定 vm 入站面与会话历史**的内存装配（guest 诊断动作要断言"回包就是 guest 报的事实"）。
/// 约束：与 `core_with_gateway` 同形，只把 vm 与历史换成本次用例给的替身。
pub(crate) fn ops_with_vm(
    modules: Vec<crate::units::business::catalog::modules::api::Module>,
    history: std::sync::Arc<doubles::InMemoryHistory>,
    vm: std::sync::Arc<dyn crate::units::business::vm::api::Vm + Send + Sync>,
) -> (
    crate::units::business::runtime::api::Handle,
    crate::presentation::faces::Faces,
) {
    let gateway: std::sync::Arc<dyn crate::units::business::llm::ports::ChatGateway + Send + Sync> =
        std::sync::Arc::new(doubles::gw(std::collections::BTreeMap::new(), Vec::new()));
    let llm = doubles::test_llm(
        std::sync::Arc::clone(&gateway),
        std::sync::Arc::new(doubles::FakeCatalog::new(vec!["m".to_string()])),
    );
    let core = crate::units::business::runtime::service::Core::new(
        root_dir(),
        doubles::registry_service(
            doubles::InMemorySettings::new(),
            std::sync::Arc::clone(&llm),
        ),
        doubles::test_history_of(history),
        doubles::test_workspace(std::sync::Arc::new(doubles::InMemoryWorkspace::new())),
        vm,
        doubles::test_packages(std::sync::Arc::new(doubles::InMemoryPackages::empty())),
        doubles::test_provision(),
        doubles::test_modules(std::sync::Arc::new(doubles::VecSource(modules))),
        std::sync::Arc::new(crate::units::business::maintenance::builds::api::NoBuilds),
        llm,
        doubles::test_tools_svc_with(
            std::sync::Arc::new(builders::SilentRunner),
            std::sync::Arc::new(doubles::InMemorySysIo::new()),
            std::sync::Arc::new(doubles::NoFenceHost),
        ),
        doubles::test_prompt(),
        doubles::test_tools_svc(),
        std::sync::Arc::new(crate::kernel::ports::NoopLog),
        doubles::test_host_probe(),
        std::sync::Arc::new(crate::units::business::residents::api::NoResidents),
        std::sync::Arc::new(crate::units::business::catalog::secrets::api::NoSecrets),
        doubles::test_profiles(),
    );
    let handle = crate::units::business::runtime::api::Handle::spawn(core).expect("起核心线程");
    let faces = crate::presentation::faces::Faces::from_handle(&handle);
    (handle, faces)
}

/// 目的：一次单 agent 工作的规格（契约测试共用）。
pub(crate) fn single_work(
    name: &str,
    modules: &[&str],
) -> crate::units::business::work::api::WorkSpec {
    crate::units::business::work::api::WorkSpec {
        flows: None,
        freshness: None,
        name: name.to_string(),
        kind: crate::units::business::session_model::api::SINGLE.to_string(),
        agents: vec![crate::units::pure::registry_model::api::AgentMeta {
            name: modules[0].to_string(),
            transient: true,
            modules: modules.iter().map(|s| s.to_string()).collect(),
            model: None,
            chara: String::new(),
        }],
        task: None,
        delegate: false,
        permission: None,
        tier: crate::units::pure::exec_model::api::Tier::Host,
        guest: None,
        workspace: None,
    }
}

/// 目的：边流边等停止的通道：把「生成中」变成可观察状态——取消后下一次回调即返回 false。
///   上限约 30 秒：只要提前返回，就说明是「停止」生效而不是它自然跑完。
pub(crate) struct SlowChat {
    pub ticks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::units::business::llm::api::Chat for SlowChat {
    fn complete(
        &mut self,
        _m: &[crate::units::business::llm::api::Msg],
        _opts: crate::units::business::llm::api::CompleteOpts<'_>,
        on: &mut dyn FnMut(crate::units::business::llm::api::Chunk) -> bool,
    ) -> crate::units::business::llm::api::Completion {
        use std::sync::atomic::Ordering;
        if !on(crate::units::business::llm::api::Chunk::Start) {
            return crate::units::business::llm::api::Completion::text("");
        }
        for _ in 0..6_000 {
            self.ticks.fetch_add(1, Ordering::Relaxed);
            if !on(crate::units::business::llm::api::Chunk::Text(
                "·".to_string(),
            )) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        crate::units::business::llm::api::Completion::text(
            "{\"type\":\"say\",\"text\":\"（慢通道）收到停止\"}",
        )
    }
}

/// 目的：阻塞到被放行的通道：把"生成中"变成**可观察且可控**的状态。
///   为什么需要它：协作的泵目前**没有取消检查**（见缺口账），只能靠放行来结束，
///   否则测试会一直等到整段讨论跑完。
pub(crate) struct GatedChat {
    pub started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub release: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl crate::units::business::llm::api::Chat for GatedChat {
    fn complete(
        &mut self,
        _m: &[crate::units::business::llm::api::Msg],
        _opts: crate::units::business::llm::api::CompleteOpts<'_>,
        on: &mut dyn FnMut(crate::units::business::llm::api::Chunk) -> bool,
    ) -> crate::units::business::llm::api::Completion {
        use std::sync::atomic::Ordering;
        let n = self.started.fetch_add(1, Ordering::Relaxed);
        if !on(crate::units::business::llm::api::Chunk::Start) {
            return crate::units::business::llm::api::Completion::text("");
        }
        // 第一次调用立刻返回（让"已产生的行"真的落下来），之后才阻塞：
        // 这样既能观察"生成中途"的状态，又能靠放行结束。
        if n == 0 {
            return crate::units::business::llm::api::Completion::text(
                "{\"type\":\"say\",\"text\":\"我先说\"}",
            );
        }
        // 等放行；但**也要尊重分片回调**——「停止」正是靠 on 返回 false 在调用中途生效的。
        while !self.release.load(Ordering::Relaxed) {
            if !on(crate::units::business::llm::api::Chunk::Text(
                "…".to_string(),
            )) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        crate::units::business::llm::api::Completion::text("{\"type\":\"agree\",\"text\":\"同意\"}")
    }
}

/// 目的：一律发"阻塞到放行"通道的网关。
pub(crate) struct GatedGateway {
    pub started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub release: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// 目的：记录每次问询收到的消息，再转发给内层通道。
pub(crate) struct RecordingChat {
    pub inner: crate::units::business::llm::api::BoxedChat,
    pub seen: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
}

impl crate::units::business::llm::api::Chat for RecordingChat {
    fn complete(
        &mut self,
        m: &[crate::units::business::llm::api::Msg],
        o: crate::units::business::llm::api::CompleteOpts<'_>,
        on: &mut dyn FnMut(crate::units::business::llm::api::Chunk) -> bool,
    ) -> crate::units::business::llm::api::Completion {
        self.seen
            .lock()
            .expect("锁")
            .push(m.iter().map(|x| x.content.clone()).collect());
        self.inner.complete(m, o, on)
    }

    fn last_usage(&self) -> Option<crate::units::business::llm::api::TokenUsage> {
        self.inner.last_usage()
    }
}

/// 目的：包一层记录器：把每个成员通道收到的消息记下来（判"发送视图"用）。
pub(crate) struct RecordingGateway {
    pub inner: crate::tests::doubles::ScriptGateway,
    pub seen: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
}

impl crate::units::business::llm::ports::ChatGateway for RecordingGateway {
    fn probe_tools(
        &self,
        c: &crate::units::business::llm::api::Channel,
    ) -> Result<crate::units::business::llm::api::ProbeOutcome, String> {
        self.inner.probe_tools(c)
    }
    fn member_channel(
        &self,
        c: &crate::units::business::llm::api::Channel,
        id: &str,
    ) -> crate::units::business::llm::api::BoxedChat {
        let chat = self.inner.member_channel(c, id);
        Box::new(RecordingChat {
            inner: chat,
            seen: std::sync::Arc::clone(&self.seen),
        })
    }
    fn core_channel(
        &self,
        c: &crate::units::business::llm::api::Channel,
    ) -> crate::units::business::llm::api::BoxedChat {
        self.inner.core_channel(c)
    }
}

impl crate::units::business::llm::ports::ChatGateway for GatedGateway {
    fn probe_tools(
        &self,
        _c: &crate::units::business::llm::api::Channel,
    ) -> Result<crate::units::business::llm::api::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(
        &self,
        _c: &crate::units::business::llm::api::Channel,
        _id: &str,
    ) -> crate::units::business::llm::api::BoxedChat {
        Box::new(GatedChat {
            started: std::sync::Arc::clone(&self.started),
            release: std::sync::Arc::clone(&self.release),
        })
    }
    fn core_channel(
        &self,
        _c: &crate::units::business::llm::api::Channel,
    ) -> crate::units::business::llm::api::BoxedChat {
        Box::new(GatedChat {
            started: std::sync::Arc::clone(&self.started),
            release: std::sync::Arc::clone(&self.release),
        })
    }
}

/// 目的：装配一个「生成阻塞到放行」的核心（观察"生成期间读接口不排队"）。
pub(crate) fn gated_ops(
    modules: Vec<crate::units::business::catalog::modules::api::Module>,
) -> (
    crate::units::business::runtime::api::Handle,
    crate::presentation::faces::Faces,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let gateway = GatedGateway {
        started: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        release: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let started = std::sync::Arc::clone(&gateway.started);
    let release = std::sync::Arc::clone(&gateway.release);
    let handle = crate::units::business::runtime::api::Handle::spawn(doubles::core_with_gateway(
        modules, gateway,
    ))
    .expect("起核心线程");
    let faces = crate::presentation::faces::Faces::from_handle(&handle);
    (handle, faces, started, release)
}

/// 目的：一律发慢通道的网关（可选网关参数：慢通道要自定义时序，`ops_with` 覆盖不了）。
pub(crate) struct SlowGateway {
    pub ticks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::units::business::llm::ports::ChatGateway for SlowGateway {
    fn probe_tools(
        &self,
        _c: &crate::units::business::llm::api::Channel,
    ) -> Result<crate::units::business::llm::api::ProbeOutcome, String> {
        Err("脚本替身没有真实供应商，测不了工具调用支持".to_string())
    }
    fn member_channel(
        &self,
        _c: &crate::units::business::llm::api::Channel,
        _id: &str,
    ) -> crate::units::business::llm::api::BoxedChat {
        Box::new(SlowChat {
            ticks: std::sync::Arc::clone(&self.ticks),
        })
    }
    fn core_channel(
        &self,
        _c: &crate::units::business::llm::api::Channel,
    ) -> crate::units::business::llm::api::BoxedChat {
        Box::new(SlowChat {
            ticks: std::sync::Arc::clone(&self.ticks),
        })
    }
}

/// 目的：装配一个「生成会一直跑到被停止」的核心（观察 is_running / stop / 编辑禁令）。
pub(crate) fn slow_ops(
    modules: Vec<crate::units::business::catalog::modules::api::Module>,
) -> (
    crate::units::business::runtime::api::Handle,
    crate::presentation::faces::Faces,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let gateway = SlowGateway {
        ticks: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let ticks = std::sync::Arc::clone(&gateway.ticks);
    let handle = crate::units::business::runtime::api::Handle::spawn(doubles::core_with_gateway(
        modules, gateway,
    ))
    .expect("起核心线程");
    let faces = crate::presentation::faces::Faces::from_handle(&handle);
    (handle, faces, ticks)
}
