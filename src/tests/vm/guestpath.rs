//! 目的：虚拟根名与路径桥——宿主路径编进 guest 镜像根，反向也还原。
//! 管：名字解析、命令装配、围栏根、UNC 编码、可表示判据。
//! 不管：manifest 解析（`guest.rs`）。
//! 联动：`src/capabilities/vm/detail/path_bridge.rs`。

use super::super::prelude::*;
use crate::capabilities::vm::domain::guest::{GuestImage, GuestSpec};
use crate::capabilities::vm::ports::ShareBackend;

/// 目的：虚拟根名 → 那一格目录的解析——名字是**一段**，路径与越界写法如实拒绝（不猜、不静默当路径）。
#[test]
pub(crate) fn guest_names_resolve_to_one_slot_and_paths_are_refused() {
    use crate::capabilities::vm::api::resolve_guest;
    let root = crate::tests::root_dir();
    let got = resolve_guest(&root, "debian").expect("清单里的名字要认得");
    assert_eq!(got, root.join(".vm").join("guest").join("debian"));
    assert!(resolve_guest(&root, "  ").is_err(), "空名要拒");
    assert!(resolve_guest(&root, "a/b").is_err(), "路径分隔符要拒");
    assert!(resolve_guest(&root, "a\\b").is_err(), "反斜杠也要拒");
    assert!(resolve_guest(&root, "..").is_err(), "上级目录要拒");
    assert!(resolve_guest(&root, ".").is_err(), "当前目录要拒");
    assert!(
        resolve_guest(&root, "/abs/path").is_err(),
        "绝对路径不是虚拟根名"
    );
}

/// 目的：命令装配按 guest 方言译路径——工作目录与本机路径型环境变量改成 guest 拼法，命令与非路径变量不动。
#[test]
pub(crate) fn translate_run_maps_paths_to_the_guest_dialect() {
    use crate::capabilities::vm::domain::guest::{GuestMount, GuestOs, GuestRun, translate_run};
    // 挂载点用**宿主自己的写法**（宿主路径只能由宿主解析），guest 路径由我们给——断言因此三平台一致。
    let (rt, work) = if cfg!(windows) {
        ("D:\\rt\\py", "D:\\work")
    } else {
        ("/rt/py", "/work")
    };
    let mounts = vec![
        GuestMount {
            host: std::path::PathBuf::from(rt),
            guest: "/opt/rt/py".to_string(),
            read_only: true,
        },
        GuestMount {
            host: std::path::PathBuf::from(work),
            guest: "/guest/work".to_string(),
            read_only: false,
        },
    ];
    let run = GuestRun {
        command: "read_txt".to_string(),
        args_json: "{}".to_string(),
        env: vec![
            ("PATH".to_string(), format!("{}/bin", rt)),
            ("LANG".to_string(), "C.UTF-8".to_string()),
        ],
        cwd: format!("{}/mod", work),
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    };
    let out = translate_run(&run, GuestOs::Linux, &mounts);
    assert_eq!(
        out.cwd, "/guest/work/mod",
        "工作目录按挂载表译成 guest 拼法"
    );
    assert_eq!(
        out.env[0].1, "/opt/rt/py/bin",
        "包目录型环境变量按**挂载点**译（不是镜像根）"
    );
    assert_eq!(out.env[1].1, "C.UTF-8", "非路径变量不动");
    assert_eq!(out.command, run.command, "命令不动");
    assert_eq!(
        out.args_json, run.args_json,
        "参数不动（模型给的就是 guest 拼法）"
    );

    // 挂载表优先于方言：guest 换成 Windows，命中的还是同一个挂载点。
    let same = translate_run(&run, GuestOs::Windows, &mounts);
    assert_eq!(same.cwd, "/guest/work/mod", "挂载表优先");

    // 不在任何挂载里的宿主路径：同方言原样，跨方言才走镜像根。
    let loose = translate_run(
        &GuestRun {
            command: "x".to_string(),
            args_json: "{}".to_string(),
            env: Vec::new(),
            cwd: format!("{}-outside", work),
            timeout_secs: 30,
            output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
        },
        GuestOs::Linux,
        &mounts,
    );
    if cfg!(windows) {
        assert_eq!(
            loose.cwd, "/solomni-host/d/work-outside",
            "不在挂载里就走镜像根"
        );
    } else {
        assert_eq!(loose.cwd, format!("{}-outside", work), "同方言：原样");
    }
}

/// 目的：路径桥同方言恒等；跨方言把宿主路径编进 guest 镜像根（非法字符转义）。
#[test]
pub(crate) fn path_bridge_maps_host_paths_into_the_guest() {
    use crate::capabilities::vm::domain::paths::{PathBridge, PathDialect};
    let id_posix = PathBridge::new("linux", PathDialect::Posix);
    assert_eq!(
        id_posix.to_guest(std::path::Path::new("/work/x")),
        "/work/x",
        "Linux 宿主 + POSIX guest 是同方言：原样"
    );
    let id_win = PathBridge::new("windows", PathDialect::Windows);
    assert_eq!(
        id_win.to_guest(std::path::Path::new("D:\\work")),
        "D:\\work",
        "Windows 宿主 + Windows guest 是同方言：原样"
    );

    // POSIX 宿主 → Windows guest：镜像根 + 非法字符转义（与宿主是哪个平台无关）。
    let w = PathBridge::new("linux", PathDialect::Windows);
    assert_eq!(
        w.to_guest(std::path::Path::new("/home/u/a:b")),
        "C:\\solomni-host\\home\\u\\a%3Ab",
        "Windows guest 走 Windows 镜像根并把 : 转义"
    );

    // Windows 宿主 → POSIX guest：宿主路径由**宿主自己**解析，所以这条只在 Windows 上判
    //（POSIX 上 "D:\\x" 是一个普通组件，拿它当 Windows 路径断言是错的）。
    #[cfg(windows)]
    {
        let b = PathBridge::new("windows", PathDialect::Posix);
        assert_eq!(
            b.to_guest(std::path::Path::new("D:\\AIproject\\Solomni\\work")),
            "/solomni-host/d/AIproject/Solomni/work",
            "Windows 宿主编进 POSIX 镜像根"
        );
    }
}

/// 目的：围栏根经桥落到 guest——跨方言时是镜像根，不再是宿主原路径。
#[test]
pub(crate) fn fence_roots_follow_the_bridge() {
    use crate::capabilities::vm::domain::exec::ExecPlan;
    use crate::capabilities::vm::domain::guest::mounts_from;
    use crate::capabilities::vm::domain::paths::{PathBridge, PathDialect};
    let plan = ExecPlan {
        tier: Tier::Vm,
        net: false,
        guest: None,
        packages: Vec::new(),
    };
    // 跨方言那一侧按宿主取（宿主路径只能由宿主自己解析）：断言的都是"围栏根按桥落到 guest 镜像根"。
    let (mods, work, bridge, want_mods, want_work) = if cfg!(windows) {
        (
            "D:\\mods\\a",
            "D:\\work\\x",
            PathBridge::new("windows", PathDialect::Posix),
            "/solomni-host/d/mods/a",
            "/solomni-host/d/work/x",
        )
    } else {
        (
            "/mods/a",
            "/work/x",
            PathBridge::new("linux", PathDialect::Windows),
            "C:\\solomni-host\\mods\\a",
            "C:\\solomni-host\\work\\x",
        )
    };
    let fence = crate::kernel::api::FenceSpec::standalone(
        std::path::Path::new(mods),
        Some(std::path::Path::new(work)),
        false,
    );
    let mounts = mounts_from(&plan, &fence, &bridge);
    assert!(
        mounts.iter().any(|m| m.guest == want_mods && m.read_only),
        "只读模块目录按镜像根挂：{:?}",
        mounts
    );
    assert!(
        mounts.iter().any(|m| m.guest == want_work && !m.read_only),
        "可写根按镜像根挂：{:?}",
        mounts
    );
}

#[test]
pub(crate) fn guest_mounts_prefix_packages_and_fence_roots() {
    use crate::capabilities::packages::api::PlannedPackage;
    use crate::capabilities::vm::domain::exec::ExecPlan;
    use crate::capabilities::vm::domain::guest::mounts_from;
    let plan = ExecPlan {
        tier: Tier::Vm,
        net: false,
        guest: None,
        packages: vec![PlannedPackage {
            id: "python".to_string(),
            version: "3.12.4".to_string(),
            kind: "prefix".to_string(),
            prefix: "opt/rt/py".to_string(),
            source_dir: std::path::PathBuf::from("/rt/py"),
            provides_paths: Vec::new(),
        }],
    };
    let fence = crate::kernel::api::FenceSpec::standalone(
        std::path::Path::new("/mods/a"),
        Some(std::path::Path::new("/work/x")),
        false,
    );
    // 恒等桥（宿主与 guest 同方言）：这条用例判的是"prefix 与围栏根各自挂到哪"，不是跨方言换算。
    let bridge = crate::capabilities::vm::domain::paths::PathBridge::new(
        "linux",
        crate::capabilities::vm::domain::paths::PathDialect::Posix,
    );
    let mounts = mounts_from(&plan, &fence, &bridge);
    assert!(
        mounts
            .iter()
            .any(|m| m.guest == "/opt/rt/py" && m.read_only),
        "prefix 包只读挂到 /<prefix>：{:?}",
        mounts
    );
    assert!(
        mounts.iter().any(|m| m.guest == "/work/x" && !m.read_only),
        "可写根按宿主原路径挂入：{:?}",
        mounts
    );
    assert!(
        mounts.iter().any(|m| m.guest == "/mods/a" && m.read_only),
        "模块目录只读挂入：{:?}",
        mounts
    );
    let share = crate::capabilities::vm::detail::NinePShare::new()
        .prepare("qemu", &mounts, None)
        .expect("9p 计划要能派生")
        .0;
    let spec = GuestSpec {
        lease: "w".to_string(),
        image: GuestImage::linux(
            "test",
            std::path::PathBuf::from("/base/rootfs.ext4"),
            Some(std::path::PathBuf::from("/base/vmlinuz")),
            Some(std::path::PathBuf::from("/base/initrd.img")),
        ),
        mounts,
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: false,
        cache_disk: None,
    };
    let args = crate::capabilities::vm::detail::qemu::engine_args(
        &spec,
        "host=127.0.0.1,port=1,server=on,wait=off",
        false,
        false,
        None,
        &[],
    );
    assert!(args.contains(&"none".to_string()), "不给网卡");
    assert!(
        !args.iter().any(|a| a.contains("netdev")),
        "没有网卡就没有 netdev"
    );
    assert!(
        args.iter()
            .any(|a| a == "file=/base/rootfs.ext4,if=virtio,format=raw"),
        "根是磁盘镜像（virtio 磁盘）：{:?}",
        args
    );
    assert!(
        !args.iter().any(|a| a.contains("fsbase")),
        "根是磁盘镜像，不该再有 9p 基础根共享：{:?}",
        args
    );
    let accel = args
        .iter()
        .position(|a| a.as_str() == "-machine")
        .map(|i| args[i + 1].as_str());
    assert_eq!(
        accel,
        Some(
            crate::capabilities::vm::detail::qemu::accelerators()
                .expect("本平台有硬件加速（虚拟机档只用硬件加速）")
        ),
        "加速器按平台给（只用硬件加速，不退回软件模拟）"
    );
    let serial = args
        .iter()
        .position(|a| a.as_str() == "-serial")
        .map(|i| args[i + 1].as_str());
    assert_eq!(
        serial,
        Some("stdio"),
        "console=ttyS0 要有对应串口设备（-nodefaults 会去掉默认串口）"
    );
    assert!(
        args.iter().any(|a| a.contains("initrd.img")),
        "initrd 要带上"
    );

    // 共享后端：9p 把额外挂载翻成 -fsdev/-device 与 guest 端 mount 脚本。
    assert_eq!(share.kind, "9p");
    assert!(
        share
            .engine_args
            .iter()
            .any(|a| a.contains("virtio-9p-pci,fsdev=fs0,mount_tag=m0")),
        "额外挂载要接成 virtio-9p 设备：{:?}",
        share.engine_args
    );
    assert!(
        share
            .guest_scripts
            .iter()
            .any(|s| s.contains("mount -t 9p") && s.contains("m0")),
        "guest 端要给出 9p 挂载命令：{:?}",
        share.guest_scripts
    );
    assert!(
        share.guest_scripts.iter().any(|s| s.contains(",ro")),
        "只读挂载脚本要带 ro：{:?}",
        share.guest_scripts
    );
}

/// 目的：路径桥的**反向**——guest 写法译回宿主；不在镜像根里、也不是宿主绝对路径要拒。
#[test]
pub(crate) fn path_bridge_round_trips_guest_and_host() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::{PathDialect, guest_to_host};
    use std::path::Path;
    // POSIX 宿主 + Windows guest：桥按**构造的**宿主平台判，与真实平台无关，所以任何平台都跑——
    // CI 的 POSIX 宿主就靠它；只写进 cfg(unix) 分支会让 Windows 本地漏判（丢根就是这么漏过去的）。
    let l = PathBridge::new("linux", PathDialect::Windows);
    let posix = Path::new("/home/u/a");
    let g = l.to_guest(posix);
    assert_eq!(g, "C:\\solomni-host\\home\\u\\a");
    assert_eq!(
        guest_to_host(l, &g),
        Some(posix.to_path_buf()),
        "解出来要落回同一个绝对路径"
    );
    // 同方言（本机档）恒等。
    let id = PathBridge::new("linux", PathDialect::Posix);
    assert_eq!(guest_to_host(id, "/work/x"), Some(PathBuf::from("/work/x")));
    if cfg!(windows) {
        let b = PathBridge::new("windows", PathDialect::Posix);
        let host = Path::new(r"D:\a\b");
        let g = b.to_guest(host);
        assert_eq!(g, "/solomni-host/d/a/b");
        assert_eq!(
            guest_to_host(b, &g),
            Some(host.to_path_buf()),
            "镜像路径要能解回来"
        );
        assert_eq!(
            guest_to_host(b, "/etc/passwd"),
            None,
            "不在镜像根里、也不是宿主绝对路径"
        );
        assert_eq!(
            guest_to_host(b, r"D:\a\b"),
            None,
            "虚拟机档只认 guest 写法，宿主路径不再直通"
        );
    }
}

/// 目的：per-agent 允许集按挂载表翻成 guest 写法（跨方言也一样）。
#[test]
pub(crate) fn guest_access_translates_the_fence_into_guest_paths() {
    use crate::capabilities::vm::domain::guest::{GuestAccess, GuestMount};
    use crate::kernel::api::FenceSpec;
    let mounts = vec![
        GuestMount {
            host: std::path::PathBuf::from("/work"),
            guest: "/guest/work".to_string(),
            read_only: false,
        },
        GuestMount {
            host: std::path::PathBuf::from("/mods"),
            guest: "/guest/mods".to_string(),
            read_only: true,
        },
    ];
    let fence = FenceSpec {
        agent: "a".to_string(),
        lease: "w--a".to_string(),
        rw: vec![std::path::PathBuf::from("/work/a")],
        ro: vec![std::path::PathBuf::from("/work/ro")],
        ro_tree: vec![std::path::PathBuf::from("/mods/m1")],
        private: std::path::PathBuf::from("/work/a"),
        cwd: std::path::PathBuf::from("/mods/m1"),
        net: false,
    };
    let access = GuestAccess::from_fence(
        &fence,
        crate::capabilities::vm::domain::guest::GuestOs::Linux,
        &mounts,
    );
    assert_eq!(access.agent, "a");
    assert_eq!(access.rw, vec!["/guest/work/a".to_string()]);
    assert_eq!(access.ro, vec!["/guest/work/ro".to_string()]);
    assert_eq!(access.ro_tree, vec!["/guest/mods/m1".to_string()]);
    assert_eq!(access.cwd, "/guest/mods/m1");
}

/// 目的：起 guest 前的可表示判据——稳定根必须能经路径桥往返；盘符与 UNC 都能，
///   卷 GUID 形态与非 UTF-8 不能（编成 guest 写法会丢字节或没有普通写法）。
#[test]
pub(crate) fn mount_plan_round_trips_every_stable_root() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::PathDialect;
    use std::path::Path;
    // 桥按**构造的**宿主平台判（与 path_bridge_round_trips_guest_and_host 同一口径），跨平台都跑。
    let windows_to_posix = PathBridge::new("windows", PathDialect::Posix);
    if cfg!(windows) {
        assert!(
            windows_to_posix.round_trips(Path::new(r"D:\proj\work")),
            "盘符路径要能往返"
        );
        assert!(
            windows_to_posix.round_trips(Path::new(r"\\server\share\work")),
            "UNC 走保留段编码后要能往返（Windows 虚拟机档的外部工作区靠它）"
        );
        assert!(
            !windows_to_posix.round_trips(Path::new(r"\\?\D:\proj\work")),
            "扩展长度前缀的写法本身不能原样往返（编码后是普通盘符写法）"
        );
        assert!(
            !windows_to_posix.round_trips(Path::new(
                r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\work"
            )),
            "卷 GUID 没有 guest 写法：不能往返"
        );
    }
    // 同方言恒等：宿主写法原样即 guest 写法，往返成立。
    let identity = PathBridge::new(
        std::env::consts::OS,
        if cfg!(windows) {
            PathDialect::Windows
        } else {
            PathDialect::Posix
        },
    );
    let here = std::env::current_dir().expect("取当前目录");
    assert!(identity.round_trips(&here), "同方言路径要能往返");
    // 跨方言（POSIX 宿主 → Windows guest）同样成立；这条在任何平台都按构造的平台判。
    let posix_to_windows = PathBridge::new("linux", PathDialect::Windows);
    assert!(
        posix_to_windows.round_trips(Path::new("/home/u/work")),
        "POSIX 路径编进 Windows 镜像根后要能解回"
    );
}

/// 目的：UNC 的 guest 写法——保留段 unc 编出服务器名与共享名，逆变换还原成 \\server\share\…。
/// 约束：盘符段只可能是单字母，unc 段不会撞；..、空段、反斜杠走私一律不认（宁可不解，也不拼出另一条路径）。
#[cfg(windows)]
#[test]
pub(crate) fn unc_workspace_encodes_into_the_posix_mirror() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::PathDialect;
    use crate::kernel::ports::PathCodec;
    use std::path::Path;

    let bridge = PathBridge::new("windows", PathDialect::Posix);
    let host = Path::new(r"\\server\share\dir\a.txt");
    assert_eq!(
        bridge.to_view(host),
        "/solomni-host/unc/server/share/dir/a.txt"
    );
    assert_eq!(
        bridge.to_host("/solomni-host/unc/server/share/dir/a.txt"),
        Some(host.to_path_buf())
    );
    // 共享根本身也在编码内（挂载点可以是共享下的子目录，也可以是共享根——绑定层另有拦截）。
    let share = Path::new(r"\\server\share");
    assert_eq!(bridge.to_view(share), "/solomni-host/unc/server/share");
    assert!(bridge.round_trips(share));
    // 段非法：..、空段、反斜杠走私、缺共享名。
    for bad in [
        "/solomni-host/unc/server/share/../x",
        "/solomni-host/unc/server//x",
        "/solomni-host/unc/server/share/a\\b",
        "/solomni-host/unc/server",
        "/solomni-host/unc",
    ] {
        assert_eq!(bridge.to_host(bad), None, "不该认：{bad}");
    }
}

/// 目的：Windows 虚拟机档的外部工作区是 UNC 时，挂载计划把 UNC 根当稳定根、guest 侧用编码后的镜像路径。
/// 约束：不碰网络——稳定根与私沙都只是路径：check_stable_roots 只做编码往返，mounts_from 只做翻译。
#[cfg(windows)]
#[test]
pub(crate) fn unc_external_workspace_plans_a_guest_mount() {
    use crate::capabilities::vm::api::{ExecPlan, PathBridge, mounts_from};
    use crate::capabilities::vm::domain::paths::PathDialect;
    use crate::capabilities::workspace::api::{Sandbox, Sandboxes};
    use crate::kernel::api::{FenceSpec, Tier};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    let session_root = PathBuf::from(r"D:\product\session\w");
    let shared = PathBuf::from(r"\\server\share\proj");
    let sandboxes = Sandboxes {
        shared: shared.clone(),
        session_root: session_root.clone(),
        list: vec![Sandbox {
            work_name: "w".to_string(),
            session: "w".to_string(),
            agent: "a".to_string(),
            shared: shared.clone(),
            private: session_root.join("a"),
            modules: BTreeMap::new(),
            modules_with_userdata: Default::default(),
            permissions: Default::default(),
            path_codec: None,
            shared_grant: crate::capabilities::session::api::WorkspaceGrant::Session,
        }],
        codec: None,
    };
    let bridge = PathBridge::new("windows", PathDialect::Posix);
    crate::capabilities::conductor::service::guest_job::check_stable_roots(&sandboxes, &bridge)
        .expect("UNC 外部工作区要能起 guest");
    let (rw, ro_tree) =
        crate::capabilities::conductor::service::guest_job::stable_roots(&sandboxes);
    assert!(rw.contains(&shared), "UNC 外部根要单独进稳定根：{:?}", rw);
    let boot = FenceSpec {
        agent: String::new(),
        lease: "unc".to_string(),
        rw,
        ro: Vec::new(),
        ro_tree,
        private: PathBuf::new(),
        cwd: PathBuf::new(),
        net: false,
    };
    let plan = ExecPlan {
        tier: Tier::Vm,
        net: false,
        guest: Some("windows".to_string()),
        packages: Vec::new(),
    };
    let mounts = mounts_from(&plan, &boot, &bridge);
    let unc = mounts
        .iter()
        .find(|m| m.host == shared)
        .expect("UNC 根要有挂载");
    assert_eq!(unc.guest, "/solomni-host/unc/server/share/proj");
    assert!(!unc.read_only, "外部工作区根要可写");
}

/// 目的：外部工作区的 guest 路径可能含单引号（宿主目录名带 ' 时镜像路径会原样带上）；四个共享后端
///   拼进 guest shell 的路径必须统一走 sh_quote，否则挂载脚本会在 guest 里被拆成两条命令。
#[test]
pub(crate) fn share_backend_scripts_quote_guest_paths() {
    use crate::capabilities::vm::detail::{copy, share_ninep, share_smb, share_virtiofs};
    use crate::capabilities::vm::domain::guest::{GuestMount, sh_quote};
    let quoted_path = "/solomni-host/c/it's here";
    let mounts = [
        GuestMount {
            host: std::path::PathBuf::from("C:/host/a"),
            guest: quoted_path.to_string(),
            read_only: false,
        },
        GuestMount {
            host: std::path::PathBuf::from("C:/host/b"),
            guest: "/solomni-host/c/ro dir".to_string(),
            read_only: true,
        },
    ];
    // 单引号的 POSIX 拼法：闭引号 + 反斜杠转义的引号 + 重开引号（sh_quote 的唯一合法写法）。
    assert!(
        sh_quote(quoted_path).contains("'\\''"),
        "sh_quote 要按 POSIX 拆合单引号：{}",
        sh_quote(quoted_path)
    );
    let plans = [
        ("9p", share_ninep::ninep_plan(&mounts)),
        (
            "virtiofs",
            share_virtiofs::virtiofs_plan(&mounts, Vec::new()),
        ),
        ("smb", share_smb::smb_plan(&mounts, 45678)),
        ("copy", copy::copy_plan(&mounts)),
    ];
    for (id, plan) in plans {
        assert_eq!(
            plan.guest_scripts.len(),
            mounts.len(),
            "{} 每条挂载一条脚本",
            id
        );
        for (i, script) in plan.guest_scripts.iter().enumerate() {
            let quoted = sh_quote(&mounts[i].guest);
            assert!(
                script.contains(&quoted),
                "{} 的 guest 路径没走 sh_quote：{}",
                id,
                script
            );
            // 把已引用的写法剥掉后，不能还剩裸路径——否则 shell 会在含引号/空格的路径上拆错。
            assert!(
                !script.replace(&quoted, "").contains(&mounts[i].guest),
                "{} 里还有没走 sh_quote 的裸 guest 路径：{}",
                id,
                script
            );
        }
    }
    // copy 后端的同步脚本（tar -C / rm 的路径）同样要引用。
    let temp = "/tmp/solomni-copy-in-0.tar";
    let quoted = sh_quote(quoted_path);
    for script in [
        copy::copy_in_script(temp, quoted_path),
        copy::copy_out_script(temp, quoted_path),
    ] {
        assert!(
            script.contains(&quoted),
            "copy 同步脚本没走 sh_quote：{}",
            script
        );
        assert!(
            !script.replace(&quoted, "").contains(quoted_path),
            "copy 同步脚本里还有裸 guest 路径：{}",
            script
        );
    }
    assert!(
        copy::copy_cleanup_script(temp).contains(&sh_quote(temp)),
        "删临时包的脚本也要引用路径"
    );
}
