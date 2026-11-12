//! 目的：guest 通道与围栏执行——没有围栏就 fail-closed。
//! 管：helper 脚本、Landlock、稳定根挂载、模块入口与断网事实。
//! 不管：诊断报告（`diagnose.rs`）。
//! 联动：`src/kernel/detail/confine` 与 `RUNTIME_SPEC.md`。

use crate::capabilities::vm::api::Vm;
use crate::capabilities::vm::domain::guest::{GuestImage, GuestRun};

/// 目的：guest 通道没有实现围栏时，必须 fail-closed，不能悄悄按无围栏执行。
#[test]
pub(crate) fn a_channel_without_fencing_refuses_fenced_runs() {
    use crate::capabilities::vm::domain::guest::{GuestAccess, GuestRun};
    use crate::capabilities::vm::ports::GuestChannel;
    struct Bare;
    impl GuestChannel for Bare {
        fn exec(&mut self, _run: &GuestRun) -> crate::kernel::api::ToolOutcome {
            crate::kernel::api::ToolOutcome {
                ok: true,
                output: "bare".to_string(),
            }
        }
    }
    let access = GuestAccess {
        agent: "a".to_string(),
        rw: Vec::new(),
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: String::new(),
        cwd: String::new(),
        net: true,
        diagnose: false,
    };
    let run = GuestRun {
        command: "true".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    let out = Bare.exec_fenced(&run, &access);
    assert!(!out.ok, "没实现围栏必须失败");
    assert!(out.output.contains("围栏"), "{}", out.output);
}

/// 目的：助手脚本必须真的走 Landlock + no_new_privs，并把私有沙箱设成 HOME/TMP。
#[test]
pub(crate) fn guest_helper_is_a_fail_closed_landlock_launcher() {
    let script = crate::capabilities::vm::detail::guest_helper::HELPER_SCRIPT;
    assert!(script.contains("landlock_create_ruleset"), "{}", script);
    assert!(script.contains("landlock_restrict_self"), "{}", script);
    assert!(
        script.contains("PR_SET_NO_NEW_PRIVS") || script.contains("prctl(38"),
        "{}",
        script
    );
    assert!(script.contains("TMPDIR"), "{}", script);
    assert!(
        script.contains("write_file = 1 << 1") && script.contains("base == \"/dev/null\""),
        "标准重定向要能写 /dev/null（否则 2>/dev/null 在围栏里 EACCES）：{}",
        script
    );
    assert!(
        script.contains("env_names"),
        "环境白名单要按模块隐秘字段名注入：{}",
        script
    );
    assert!(
        script.contains("subprocess.Popen") && script.contains("start_new_session"),
        "要用独立进程组启动，才能杀整棵树：{}",
        script
    );
    assert!(
        script.contains("os.killpg")
            && script.contains("timeout_secs")
            && script.contains("policy.get(\"timeout_secs\""),
        "超时要按本次 timeout_secs 杀进程组：{}",
        script
    );
    assert!(
        script.contains("signal.SIGTERM")
            && script.contains("on_term")
            && script.contains("CHILD")
            && script.contains("os.killpg"),
        "host 取消要经 SIGTERM 杀掉子进程组：{}",
        script
    );
    assert!(
        script.contains("/etc/ld.so.cache") && !script.contains("for base in (\n        \"/usr\",\n        \"/bin\",\n        \"/sbin\",\n        \"/lib\",\n        \"/lib64\",\n        \"/etc\","),
        "只读基线要与本机 Linux 对齐，不能整棵 /etc：{}",
        script
    );
    assert!(
        script.contains("KEEP_FDS.append(pfd)") && !script.contains("os.close(pfd)"),
        "规则路径的 O_PATH fd 要保留到 helper 退出：9p cache=none 不复用 inode，提前关闭会让 Landlock 子目录规则失配：{}",
        script
    );
    assert_eq!(
        crate::capabilities::vm::detail::guest_helper::HELPER_TIMEOUT_SECS,
        30,
        "guest 超时与本机守门进程同一口径"
    );
    assert!(
        crate::capabilities::vm::detail::guest_helper::HELPER_PATH.contains("solomni-guest-fence"),
        "助手路径要稳定"
    );
    assert!(
        script.contains(crate::capabilities::vm::detail::guest_helper::HELPER_TIMEOUT_MARKER)
            && script.contains(crate::capabilities::vm::detail::guest_helper::HELPER_SIGNAL_MARKER),
        "超时 / 被信号结束要用机器标记（人读文案由通道产出）：{}",
        script
    );
    // 诊断模式：策略里 diagnose=true 时在 Landlock restrict 之后只采 /proc、打一个 JSON，不 spawn 命令。
    let helper = crate::capabilities::vm::domain::diagnose::HELPER_DIAGNOSE_PREFIX;
    assert!(
        script.contains(helper),
        "诊断出错要用专用前缀（宿主据此区分「助手在、Python 侧失败」）：{}",
        script
    );
    assert!(
        script.contains("policy.get(\"diagnose\")") && script.contains("run_diagnosis()"),
        "policy 带 diagnose 时要走诊断模式：{}",
        script
    );
    assert!(
        script.contains("collect_diagnosis")
            && script.contains("/proc/loadavg")
            && script.contains("/proc/meminfo")
            && script.contains("os.statvfs"),
        "诊断的数据源要固定（loadavg / meminfo / 挂载容量）：{}",
        script
    );
    assert!(
        script.contains("process_total") && script.contains("shrink_report"),
        "进程表要带总数并在超限时自封顶（通道 16k 上限不能被撞到）：{}",
        script
    );
    assert!(
        !script.contains("工具进程超时，已终止整棵进程树"),
        "helper 不再写人读收尾句（回执形状由通道统一装配）：{}",
        script
    );
}

/// 目的：VmService 的围栏执行要把 agent 与 guest 路径交给通道，并记录启动时装载的运行包。
#[test]
pub(crate) fn vm_service_runs_fenced_and_records_packages() {
    use crate::capabilities::vm::domain::guest::{GuestMount, GuestSpec};
    use crate::capabilities::vm::service::VmService;
    use crate::kernel::api::FenceSpec;
    use std::sync::Arc;
    let engine = Arc::new(crate::tests::doubles::FakeEngine::new());
    let share = Arc::new(crate::tests::doubles::NamedShare::new("9p", true));
    let svc = VmService::new(engine.clone(), vec![share]);
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/base/rootfs.ext4"),
            None,
            None,
        ),
        mounts: vec![GuestMount {
            host: std::path::PathBuf::from("/work"),
            guest: "/guest/work".to_string(),
            read_only: false,
        }],
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: false,
        cache_disk: None,
    };
    svc.guest_start(&spec).expect("起替身 guest");
    svc.guest_set_packages("w", vec!["python@3.12".to_string()]);
    assert_eq!(
        svc.guest_packages("w"),
        Some(vec!["python@3.12".to_string()])
    );
    let fence = FenceSpec {
        agent: "a".to_string(),
        lease: "w--a".to_string(),
        rw: vec![std::path::PathBuf::from("/work/a")],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: std::path::PathBuf::from("/work/a"),
        cwd: std::path::PathBuf::from("/work/a"),
        net: false,
    };
    let out = svc.guest_run_fenced(
        "w",
        &GuestRun {
            command: "echo hi".to_string(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: "/work/a".to_string(),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        },
        &fence,
    );
    assert!(out.ok, "{}", out.output);
    let calls = engine.calls.lock().expect("锁").clone();
    assert!(
        calls.iter().any(|c| c.contains("fenced:w:a:/guest/work/a")),
        "围栏执行要带 agent 与 guest 路径：{:?}",
        calls
    );
}

/// 目的：共享 guest 只挂稳定根（会话树 + 外部工作区根 + 模块根），新 agent / 模块按状态出现，不逐项枚举。
/// 约束：外部布局下两个根各挂一条，且**不能**挂 shared.parent()——旧实现会把外部根的父目录整棵暴露给 guest。
#[test]
pub(crate) fn stable_roots_cover_the_work_tree_and_module_tree() {
    use crate::capabilities::workspace::api::{Sandbox, Sandboxes};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    let sb = |agent: &str, shared: &PathBuf, session_root: &PathBuf, module: &str| Sandbox {
        work_name: "w".to_string(),
        session: "w".to_string(),
        agent: agent.to_string(),
        shared: shared.clone(),
        private: session_root.join(agent),
        modules: BTreeMap::from([(module.to_string(), PathBuf::from("/modules").join(module))]),
        modules_with_userdata: Default::default(),
        permissions: Default::default(),
        path_codec: None,
    };
    // 内部布局：shared = <会话树>/work，已隐含在会话树根下，不重复挂。
    let session_root = PathBuf::from("/session/w");
    let internal_shared = session_root.join("work");
    let internal = Sandboxes {
        shared: internal_shared.clone(),
        session_root: session_root.clone(),
        list: vec![
            sb("a", &internal_shared, &session_root, "m1"),
            sb("b", &internal_shared, &session_root, "m2"),
        ],
        codec: None,
    };
    let (rw, ro_tree) = crate::capabilities::conductor::service::guest_job::stable_roots(&internal);
    assert!(rw.contains(&session_root), "会话树根要整棵挂：{:?}", rw);
    assert!(
        rw.contains(&PathBuf::from("/modules")),
        "模块树根要整棵挂：{:?}",
        rw
    );
    assert!(
        !rw.contains(&internal_shared),
        "shared 在会话树内不再重复挂：{:?}",
        rw
    );
    assert!(
        !rw.iter().any(|p| p.ends_with("a") || p.ends_with("b")),
        "不再逐 agent 枚举：{:?}",
        rw
    );
    assert!(ro_tree.is_empty(), "模块 userdata 要可写，稳定根进 rw");

    // 外部布局：shared 不在会话树内（外部工作区绑定），两个根各挂一条。
    let external_shared = PathBuf::from("/external/root");
    let external = Sandboxes {
        shared: external_shared.clone(),
        session_root: session_root.clone(),
        list: vec![
            sb("a", &external_shared, &session_root, "m1"),
            sb("b", &external_shared, &session_root, "m2"),
        ],
        codec: None,
    };
    let (rw, ro_tree) = crate::capabilities::conductor::service::guest_job::stable_roots(&external);
    assert!(
        rw.contains(&session_root) && rw.contains(&external_shared),
        "会话树根与外部工作区根都要挂：{:?}",
        rw
    );
    assert!(
        !rw.contains(&PathBuf::from("/external")),
        "不能挂 shared.parent()（会把外部根的父目录整棵暴露）：{:?}",
        rw
    );
    assert!(
        !rw.iter().any(|p| p.ends_with("a") || p.ends_with("b")),
        "外部布局也不逐 agent 枚举：{:?}",
        rw
    );
    assert!(ro_tree.is_empty(), "外部根与模块根都不进只读稳定根");
}

/// 目的：把「不在挂载表就 fail-closed」变成可判定属性——外部工作区布局下，per-call 围栏里的每个根
///   （私沙 / 共享区 / 模块 / cwd）翻成 guest 写法后，都必须落在某个稳定挂载的 guest 路径之下。
/// 约束：同方言与跨方言各验一次（跨方言 = 本机宿主 → 另一侧方言的 guest，走镜像根）。
#[test]
pub(crate) fn guest_access_roots_are_all_covered_by_stable_mounts() {
    use crate::capabilities::vm::api::{mounts_from, ExecPlan, PathBridge};
    use crate::capabilities::vm::domain::guest::{GuestAccess, GuestOs};
    use crate::capabilities::vm::domain::paths::PathDialect;
    use crate::capabilities::workspace::api::{Sandbox, Sandboxes};
    use crate::kernel::api::{FenceSpec, Tier};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    // 真的绝对路径（scratch 里，不落仓库）：会话树与外部根互不包含，和外部工作区绑定后的布局同形。
    let base = crate::tests::scratch("mount-coverage");
    let session_root = base.join("session");
    let external = base.join("outside");
    let module_root = base.join("modules").join("m1");
    let private = session_root.join("a");
    let sandboxes = Sandboxes {
        shared: external.clone(),
        session_root: session_root.clone(),
        list: vec![Sandbox {
            work_name: "w".to_string(),
            session: "w".to_string(),
            agent: "a".to_string(),
            shared: external.clone(),
            private: private.clone(),
            modules: BTreeMap::from([("m1".to_string(), module_root.clone())]),
            modules_with_userdata: Default::default(),
            permissions: Default::default(),
            path_codec: None,
        }],
        codec: None,
    };
    let covers = |mount_guest: &str, path: &str| {
        let base = mount_guest.trim_end_matches('/');
        path == base || path.starts_with(&format!("{base}/"))
    };
    let check = |bridge: PathBridge, os: GuestOs| {
        let (rw, ro_tree) =
            crate::capabilities::conductor::service::guest_job::stable_roots(&sandboxes);
        let plan = ExecPlan {
            tier: Tier::Vm,
            net: false,
            guest: Some("debian".to_string()),
            packages: Vec::new(),
        };
        // boot fence 照 guest_job 的造法：稳定根进 rw，host_read_roots 为空。
        let boot = FenceSpec {
            agent: String::new(),
            lease: "w".to_string(),
            rw,
            ro: Vec::new(),
            ro_tree,
            private: PathBuf::new(),
            cwd: PathBuf::new(),
            net: false,
        };
        let mounts = mounts_from(&plan, &boot, &bridge);
        assert!(
            mounts.iter().any(|m| m.host == session_root),
            "会话树根要在挂载表里：{:?}",
            mounts
        );
        assert!(
            mounts.iter().any(|m| m.host == external),
            "外部工作区根要在挂载表里：{:?}",
            mounts
        );
        assert!(
            !mounts
                .iter()
                .any(|m| Some(m.host.as_path()) == external.parent()),
            "外部根的父目录不许进挂载表：{:?}",
            mounts
        );
        // 一次 shell 调用的围栏：私沙可写、共享区与模块只读（与 FenceSpec::for_call 同形）。
        let per_call = FenceSpec {
            agent: "a".to_string(),
            lease: "w".to_string(),
            rw: vec![private.clone()],
            ro: Vec::new(),
            ro_tree: vec![external.clone(), module_root.clone()],
            private: private.clone(),
            cwd: private.clone(),
            net: true,
        };
        let access = GuestAccess::from_fence(&per_call, os, &mounts);
        let mut roots: Vec<String> = vec![access.private.clone(), access.cwd.clone()];
        roots.extend(access.rw.iter().cloned());
        roots.extend(access.ro.iter().cloned());
        roots.extend(access.ro_tree.iter().cloned());
        for guest_path in roots {
            assert!(!guest_path.is_empty(), "per-call 根不能为空");
            assert!(
                mounts.iter().any(|m| covers(&m.guest, &guest_path)),
                "per-call 根 {} 不在任何稳定挂载之下（helper 会 fail-closed）：{:?}",
                guest_path,
                mounts.iter().map(|m| m.guest.clone()).collect::<Vec<_>>()
            );
        }
    };
    // 同方言：本平台宿主 → 本平台写法的 guest（恒等桥）。
    check(
        PathBridge::new(
            std::env::consts::OS,
            if cfg!(windows) {
                PathDialect::Windows
            } else {
                PathDialect::Posix
            },
        ),
        if cfg!(windows) {
            GuestOs::Windows
        } else {
            GuestOs::Linux
        },
    );
    // 跨方言：Windows 宿主 → POSIX guest（本机是 Windows 时就是生产那条链）；否则反方向。
    if cfg!(windows) {
        check(
            PathBridge::new("windows", PathDialect::Posix),
            GuestOs::Linux,
        );
    } else {
        check(
            PathBridge::new("linux", PathDialect::Windows),
            GuestOs::Windows,
        );
    }
}

/// 目的：模块入口落 guest 本地副本——落点在 /usr/local/lib/solomni（guest 内 helper 的只读基线已放行 /usr），
///   脚本先建目录、再复制、最后给执行位；声明里的 `./` 前缀不能带进落点。
#[test]
pub(crate) fn module_entries_are_staged_into_a_local_copy() {
    use crate::capabilities::vm::domain::guest::{stage_script, staged_entry_path};
    assert_eq!(
        staged_entry_path("indexer", "build/indexer"),
        "/usr/local/lib/solomni/modules/indexer/build/indexer"
    );
    assert_eq!(
        staged_entry_path("indexer", "./tools/build"),
        "/usr/local/lib/solomni/modules/indexer/tools/build",
        "声明里的 ./ 前缀不能带进落点"
    );
    let script = stage_script(
        "/solomni-host/d/proj/modules/indexer/build/indexer",
        "/usr/local/lib/solomni/modules/indexer/build/indexer",
    );
    assert!(
        script.starts_with("mkdir -p '/usr/local/lib/solomni/modules/indexer/build'"),
        "{}",
        script
    );
    assert!(
        script.contains("&& cp -f ") && script.contains("&& chmod 0755 "),
        "{}",
        script
    );
}

/// 目的：新子会话只在活 guest 缺包时才重建；包已覆盖就直接加入，不做运行包动态加载。
#[test]
pub(crate) fn package_rebuild_only_happens_when_the_running_guest_is_missing_one() {
    use crate::capabilities::conductor::service::needs_guest_packages_rebuild;
    let want = vec!["python@3.12".to_string()];
    assert!(!needs_guest_packages_rebuild(false, &want, None));
    assert!(!needs_guest_packages_rebuild(
        true,
        &want,
        Some(&["python@3.12".to_string()])
    ));
    assert!(needs_guest_packages_rebuild(
        true,
        &want,
        Some(&["node@22".to_string()])
    ));
    assert!(!needs_guest_packages_rebuild(true, &[], Some(&[])));
    assert!(needs_guest_packages_rebuild(true, &want, None));
}

/// 目的：per-call 断网事实从 FenceSpec 进 GuestAccess（helper 再据此 unshare 网络命名空间）。
#[test]
pub(crate) fn guest_access_carries_the_per_call_net_flag() {
    use crate::capabilities::vm::api::GuestOs;
    use crate::capabilities::vm::domain::guest::GuestAccess;
    use crate::kernel::api::FenceSpec;
    use std::path::PathBuf;

    let base = FenceSpec {
        agent: "a".to_string(),
        lease: "w".to_string(),
        rw: Vec::new(),
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: PathBuf::new(),
        cwd: PathBuf::new(),
        net: false,
    };
    assert!(!GuestAccess::from_fence(&base, GuestOs::Linux, &[]).net);
    let mut open = base.clone();
    open.net = true;
    assert!(GuestAccess::from_fence(&open, GuestOs::Linux, &[]).net);
}
