//! 目的：围栏、包、依赖与日志的宿主替身——记下每次调用，供断言“有没有碰宿主”。
//! 管：围栏释放记录、包清单、依赖状态与任务、空日志。
//! 不管：宿主探测（`probe.rs`）、常驻服务（`residents.rs`）。
//! 联动：`src/kernel/ports.rs` 与 `src/units/packages/ports.rs`。

use super::*;
use crate::units::conversation::session::api::MEETING;

/// 目的：记录型围栏端口：记录按会话（`lease_releases`）与按次（`released` / `leases`）两类释放请求。
pub(crate) struct RecordingFence {
    pub(crate) released: Mutex<Vec<String>>,
    // 每次按次撤销请求携带的租约（一次性调用断言用）。
    pub(crate) leases: Mutex<Vec<String>>,
    // 按会话租约撤销的请求（会话改配 / 删除的断言用）。
    pub(crate) lease_releases: Mutex<Vec<String>>,
    pub(crate) fail: Option<String>,
}

impl RecordingFence {
    pub(crate) fn new() -> RecordingFence {
        RecordingFence {
            released: Mutex::new(Vec::new()),
            leases: Mutex::new(Vec::new()),
            lease_releases: Mutex::new(Vec::new()),
            fail: None,
        }
    }
    /// 目的：注入失败：release 返回该原因（撤销失败必须如实传播，不能被当成已撤销）。
    pub(crate) fn fail_with(mut self, msg: &str) -> RecordingFence {
        self.fail = Some(msg.to_string());
        self
    }
}

impl crate::kernel::ports::FenceHost for RecordingFence {
    fn release_by_lease(&self, lease: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.lease_releases
            .lock()
            .expect("锁")
            .push(lease.to_string());
        Ok(())
    }

    fn release_call(&self, spec: &crate::kernel::api::fence::FencePlan) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.released.lock().expect("锁").push(spec.agent.clone());
        self.leases.lock().expect("锁").push(spec.lease.clone());
        Ok(())
    }
}

/// 目的：内存运行包库（测试组合根）：直接给出包清单，不碰盘。
pub(crate) struct InMemoryPackages(Vec<PackageManifest>);

impl InMemoryPackages {
    pub(crate) fn empty() -> InMemoryPackages {
        InMemoryPackages(Vec::new())
    }
    /// 目的：用 yaml 文本造包（顺带覆盖清单解析）。
    pub(crate) fn with(yamls: &[&str]) -> InMemoryPackages {
        InMemoryPackages(yamls.iter().map(|y| pkg_yaml(y)).collect())
    }
    /// 目的：直接给清单（需要设置 source_dir 等扫描事实时用）。
    pub(crate) fn from(manifests: Vec<PackageManifest>) -> InMemoryPackages {
        InMemoryPackages(manifests)
    }
}

impl PackageSource for InMemoryPackages {
    fn scan(&self) -> Library {
        Library::build(self.0.clone(), Vec::new())
    }
    fn dir(&self) -> PathBuf {
        abs(&["runtimes"])
    }
}

/// 目的：真机验收里**共用同一个虚拟根**的用例之间的互斥。
/// 约束：共用同一个虚拟根的真机用例会同时写基础镜像（guest 的系统状态也在里面），两台 guest 同时起会互相踩坏；凡是要起 guest 的真机用例都先拿它，串起来跑。
pub(crate) fn live_guest_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 目的：用 yaml 造一份包清单（顺带覆盖清单解析）。
pub(crate) fn pkg_yaml(y: &str) -> PackageManifest {
    yaml_serde::from_str(y).expect("包清单必须能解析")
}

/// 目的：造一个 prefix 类包（独立前缀 opt/rt/&lt;id&gt;-&lt;version&gt;）。
pub(crate) fn pkg(id: &str, version: &str) -> PackageManifest {
    pkg_yaml(&format!(
        "id: {}
version: {}
prefix: opt/rt/{}-{}",
        id, version, id, version
    ))
}

pub(crate) fn module_of(id: &str) -> Module {
    Module {
        manifest: ModuleManifest {
            build: None,
            id: id.to_string(),
            version: "0.1.0".to_string(),
            brief: format!("{} 的能力", id),
            system: format!("用法：{}", id),
            enabled: true,
            runtimes: Vec::new(),
            tools: BTreeMap::new(),
            services: Default::default(),
            secrets: Default::default(),
        },
        root: abs(&[id]),
        has_userdata: false,
    }
}

/// 目的：声明了运行能力的模块（工具的可用性按它判定）。
pub(crate) fn module_with_runtimes(id: &str, caps: &[&str]) -> Module {
    let mut m = module_of(id);
    m.manifest.runtimes = caps.iter().map(|s| s.to_string()).collect();
    m
}

/// 目的：测试用静默回合 IO（不流式、不派发短暂事件）。
pub(crate) fn with_turn_io<T>(
    f: impl FnOnce(&TurnIo, &std::sync::atomic::AtomicBool, &mut dyn FnMut(SessionEvent)) -> T,
) -> T {
    let io = TurnIo {
        llm: Default::default(),
        ask: None,
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut noop = |_e: SessionEvent| {};
    f(&io, &cancel, &mut noop)
}

/// 目的：把模块包成"每模块一个临时 agent"（协作的成员语义）。
pub(crate) fn agents_of(modules: &[&str]) -> Vec<AgentMeta> {
    modules
        .iter()
        .map(|m| AgentMeta {
            name: m.to_string(),
            transient: true,
            modules: vec![m.to_string()],
            model: None,
            chara: String::new(),
        })
        .collect()
}

/// 目的：测试用工作规格（模型留空 = 走核心默认 m）。单 agent 形态：1 个 agent（模块数不限；单模块时以模块名给 agent 起名，便于断言说话人）；协作形态：每模块一个 agent（成员 id = agent 名 = 模块名）。
pub(crate) fn work(name: &str, kind: &str, modules: &[&str]) -> WorkSpec {
    let agents = if kind == MEETING {
        agents_of(modules)
    } else {
        let agent_name = if modules.len() == 1 {
            modules[0]
        } else {
            "组合"
        };
        vec![AgentMeta {
            name: agent_name.to_string(),
            transient: true,
            modules: modules.iter().map(|s| s.to_string()).collect(),
            model: None,
            chara: String::new(),
        }]
    };
    WorkSpec {
        flows: None,
        freshness: None,
        name: name.to_string(),
        kind: kind.to_string(),
        agents,
        task: None,
        delegate: false,
        permission: None,
        tier: crate::units::exec_model::api::Tier::Host,
        guest: None,
        workspace: None,
    }
}

/// 目的：假依赖补齐面——清单为空、动作不能真跑（runtime 只读它的虚拟根清单；测试不碰网络与下载）。
pub(crate) fn test_provision()
-> Arc<dyn crate::units::provision::api::Provision + Send + Sync> {
    test_provision_with(Vec::new())
}

/// 目的：同上，但报一份给定的清单（不解析产品根：兜底路径原样回清单里的 probe）。
pub(crate) fn test_provision_with(
    deps: Vec<crate::units::provision::api::DepStatus>,
) -> Arc<dyn crate::units::provision::api::Provision + Send + Sync> {
    test_provision_with_root(deps, None)
}

/// 目的：同上，并给出产品根——验"设置里没登记时采用项目内已装的那一个"要把 probe 落成绝对路径。
pub(crate) fn test_provision_with_root(
    deps: Vec<crate::units::provision::api::DepStatus>,
    root: Option<&std::path::Path>,
) -> Arc<dyn crate::units::provision::api::Provision + Send + Sync> {
    struct FixedProvision(
        Vec<crate::units::provision::api::DepStatus>,
        Option<std::path::PathBuf>,
    );
    impl crate::units::provision::api::Provision for FixedProvision {
        fn status(&self) -> Result<Vec<crate::units::provision::api::DepStatus>, String> {
            Ok(self.0.clone())
        }
        fn guest_bases(
            &self,
        ) -> Result<Vec<crate::units::provision::api::GuestBase>, String> {
            Ok(Vec::new())
        }
        fn processes(&self) -> Result<Vec<crate::units::provision::api::ProcInfo>, String> {
            Ok(Vec::new())
        }
        fn job(&self) -> crate::units::exec_model::api::JobState {
            Default::default()
        }
        fn start(&self, _ids: Vec<String>) -> Result<(), String> {
            Err("测试不跑补齐作业".to_string())
        }
        fn cancel(&self) {}
        fn reset_guest(&self, _guest: &str, _on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
            Err("测试不跑虚拟根还原".to_string())
        }
        fn ensure(&self, _ids: &[String], _on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
            Err("测试不跑补齐".to_string())
        }
        fn ensure_spec(&self, _spec: &str, _on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
            Err("测试不跑补齐".to_string())
        }
        fn tool_path(
            &self,
            tool: crate::units::provision::api::HostTool,
            registered: &str,
        ) -> Option<String> {
            let registered = registered.trim();
            if !registered.is_empty() {
                return Some(registered.to_string());
            }
            let prefix = match tool {
                crate::units::provision::api::HostTool::Qemu => "vm.qemu.",
                crate::units::provision::api::HostTool::Virtiofsd => "vm.virtiofsd.",
                crate::units::provision::api::HostTool::Smbd => "vm.smbd.",
            };
            let dep = self
                .0
                .iter()
                .find(|d| d.present && d.id.starts_with(prefix))?;
            Some(match &self.1 {
                Some(root) => root.join(&dep.probe).to_string_lossy().into_owned(),
                None => dep.probe.clone(),
            })
        }
    }
    Arc::new(FixedProvision(deps, root.map(std::path::Path::to_path_buf)))
}

/// 目的：一条"已装在项目内"的依赖（`probe` 是相对产品根的写法，与清单同构）。
pub(crate) fn installed_dep(
    id: &str,
    probe: &str,
) -> crate::units::provision::api::DepStatus {
    crate::units::provision::api::DepStatus {
        id: id.to_string(),
        guest: None,
        present: true,
        stale: false,
        install: String::new(),
        probe: probe.to_string(),
        source: String::new(),
        url: None,
        size_mb: 0,
        note: String::new(),
    }
}

/// 目的：只跑执行流程的临时拼装规格（执行席只要 1 个 agent，含需求）。
pub(crate) fn execute_work(name: &str, modules: &[&str], task: &str) -> WorkSpec {
    // 每个模块一个 agent：执行流程按负责人派节点，名字要能与脚本对上。
    let mut w = work(name, "", modules);
    w.agents = agents_of(modules);
    w.flows = Some(vec!["execute".to_string()]);
    w.task = Some(task.to_string());
    w
}

/// 目的：协作工作规格（含需求）。
pub(crate) fn collab_work(name: &str, modules: &[&str], delegate: bool, task: &str) -> WorkSpec {
    let mut w = work(name, MEETING, modules);
    w.delegate = delegate;
    w.task = Some(task.to_string());
    w
}

pub(crate) fn scripted(s: Vec<String>) -> BoxedChat {
    Box::new(FakeChat::new(s))
}

/// 目的：共享脚本队列：多条核心响应按 complete 次序弹出（末条重复兜底）。与真实通道时序一致：建通道时不消费，调用时才消费。Arc 分身共享（网关与测试两侧）。共享脚本队列（Mutex 版：需跨线程 Send+Sync）。
pub(crate) struct SharedScript {
    pub(crate) q: Arc<Mutex<Vec<String>>>,
    // 需要验“精确上下文用量”时注入；None = 这条通道拿不到 usage。
    pub(crate) usage: Option<crate::units::llm::api::TokenUsage>,
}
