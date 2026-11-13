//! 目的：confine 的单元测试：JSON 形状、解释器解析、程序名改写、边界说明与归属身份。
//! 管：不触真实围栏的纯函数行为与注入开关语义。
//! 不管：真实平台围栏——Unix 的 Landlock / seatbelt 是进程级不可逆的，由平台探针在子进程里验收。
//! 联动：被测项在 `confine.rs` / `job.rs` / `paths.rs` / `texts.rs`；真机探针在 `tests/<平台>/`。

#[cfg(windows)]
use std::ffi::OsString;

use super::ledger::ReconcileReport;
use super::paths::*;
use super::texts::*;
use super::*;

/// 测试专用注入开关：打开它，机制验证必须确定性地报「本机不允许」。
/// 这是覆盖 EnvUnavailable 那一路的唯一确定性手段——三平台探针都靠它。
///
/// **不能在进程内调未注入的真实后端**：unix 上 Landlock 与 seatbelt 都是**进程级且不可逆**的
/// ——装了它，测试进程此后连 `target/` 都写不了（macOS CI 上真抓到过：后续 123 个用例全挂在
/// "建契约测试隔离根：Operation not permitted"）。真实后端的行为由平台探针（子进程里）验收，
/// 这里只钉注入开关本身的语义。
#[test]
fn selfcheck_injection_switch_reports_env_unavailable() {
    let spec = FenceSpec {
        agent: "a".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![PathBuf::from("demo").join("work")],
        cwd: PathBuf::from("mods").join("m0"),
        ro: Vec::new(),
        net: false,
    };
    // 运行期不该有这个开关（探针自己给守门进程带）。
    assert!(!selfcheck_forced_unavailable(), "运行期不该有这个开关");
    // 注入后：必须在触到后端之前就返回「本机不允许」，且带得出注入标记（探针据此把"环境结论"与"我们写错了"分开）。
    // SAFETY: 只改本用例独占的探针开关，用例内即改即清。
    unsafe {
        std::env::set_var(SELFCHECK_FAIL_FLAG, "1");
    }
    let injected = verify(&spec, "true");
    // SAFETY: 同上，清掉本用例自己注入的开关。
    unsafe {
        std::env::remove_var(SELFCHECK_FAIL_FLAG);
    }
    match injected {
        FenceVerdict::EnvUnavailable(why) => assert!(
            why.contains(SELFCHECK_FAIL_FLAG),
            "理由要带得出注入标记：{}",
            why
        ),
        other => panic!("注入后必须是本机不允许，实际 {:?}", other),
    }
    // 开关关掉之后必须回到真实判定入口（不被上一次注入粘住）。
    assert!(
        !selfcheck_forced_unavailable(),
        "注入是一次性的，不该留下状态"
    );
}
/// 安装目录上溯**绝不能停在文件系统根**：/bin 的父目录就是 /，
/// 一旦返回 / 就等于把整盘放行（macOS 的 seatbelt 会因此形同虚设）。
#[test]
fn install_dir_never_climbs_to_the_filesystem_root() {
    let root = if cfg!(windows) {
        PathBuf::from("C:\\")
    } else {
        PathBuf::from("/")
    };
    assert_eq!(
        install_dir(&root.join("bin")),
        root.join("bin"),
        "根下的 bin 不再上溯"
    );
    assert_eq!(install_dir(&root), root, "根就是根");
    let deep = root.join("home").join("u").join(".venv").join("bin");
    assert_eq!(
        install_dir(&deep),
        root.join("home").join("u").join(".venv"),
        "普通布局上溯一层"
    );
    let scripts = root.join("home").join("u").join("env").join("Scripts");
    assert_eq!(
        install_dir(&scripts),
        root.join("home").join("u").join("env"),
        "Scripts 布局同样上溯"
    );
    assert_eq!(
        install_dir(&root.join("usr").join("local").join("bin")),
        root.join("usr").join("local"),
        "usr/local/bin 上溯到 usr/local"
    );
}

/// cmd 只认程序名里的反斜杠：`build/indexer build` 会被读成命令 build + 开关 /indexer（模块工具会因此跑不起来）。
/// 参数里的正斜杠必须原样保留——`node tools/report.js` 正是靠它。
#[cfg(windows)]
#[test]
fn windows_program_separators_rewrites_only_the_program_name() {
    assert_eq!(
        windows_program_separators("build/indexer build"),
        "build\\indexer build"
    );
    assert_eq!(
        windows_program_separators("node tools/report.js"),
        "node tools/report.js"
    );
    assert_eq!(
        windows_program_separators("python tools/scan.py extra"),
        "python tools/scan.py extra"
    );
    assert_eq!(
        windows_program_separators(".tools/mingw64/bin/g++.exe -O2"),
        ".tools\\mingw64\\bin\\g++.exe -O2"
    );
    assert_eq!(
        windows_program_separators("build\\indexer build"),
        "build\\indexer build"
    );
    assert_eq!(windows_program_separators("\"a/b\" rest"), "\"a\\b\" rest");
    assert_eq!(windows_program_separators("plain"), "plain");
}

/// 容器执行前把程序名改写成相对 cwd：AppContainer 里绝对路径的程序会被拒（真机：同一文件
/// `D:\…\x.exe` 报 Access denied、`..\x.exe` 正常）。裸名 / 跨卷 / 本来就相对时原样返回。
#[cfg(windows)]
#[test]
fn container_program_command_rewrites_only_path_shaped_programs() {
    let sep = std::path::MAIN_SEPARATOR;
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cwd = repo.join("target").join("prog-cwd");
    let prog = repo.join("target").join("prog-bin").join("x.exe");
    // 同卷绝对路径 → 相对 cwd。
    assert_eq!(
        container_program_command(&format!("{} run", prog.display()), &cwd),
        format!("..{sep}prog-bin{sep}x.exe run")
    );
    // 带分隔符的相对路径 → 只把分隔符归一（与 windows_program_separators 同一结果）。
    assert_eq!(
        container_program_command("build/indexer build", &cwd),
        "build\\indexer build"
    );
    // 裸名不碰：cmd 的内建命令（echo / type）与 PATH 里同名程序分不开，改写会把它们打坏。
    assert_eq!(
        container_program_command("echo container-ok", &cwd),
        "echo container-ok"
    );
    assert_eq!(
        container_program_command("no-such-tool hi", &cwd),
        "no-such-tool hi"
    );
    // 跨卷 → 原样（系统临时目录与仓库常不在同一个卷）。
    let temp = std::env::temp_dir();
    if temp.components().next() != repo.components().next() {
        let cmd = format!(
            "\"{}\" /c echo",
            repo.join("target").join("x.exe").display()
        );
        assert_eq!(container_program_command(&cmd, &temp), cmd);
    }
}

/// 视图盘生效时程序名在产品根内也要译成盘符写法：容器 cwd 是视图路径，程序若仍是宿主的绝对路径，
/// 容器里既不在 cwd 内、又会被 AppContainer 按绝对路径拒掉。
#[cfg(windows)]
#[test]
fn view_program_command_translates_programs_inside_the_root() {
    use crate::kernel::detail::view_drive::ViewDrive;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let view = ViewDrive::for_test("Z:", &root);
    let prog = root.join(".tools").join("python").join("python.exe");
    assert_eq!(
        view_program_command(&format!("{} -V", prog.display()), &view),
        r"Z:\.tools\python\python.exe -V"
    );
    // 带空格的程序名保留引号。
    let spaced = root.join("Program Files").join("x.exe");
    assert_eq!(
        view_program_command(&format!("\"{}\" /c", spaced.display()), &view),
        r#""Z:\Program Files\x.exe" /c"#
    );
    // 根外、裸名、相对路径原样。
    let outside = PathBuf::from(r"C:\Windows\System32\cmd.exe");
    let cmd = format!("{} /C echo", outside.display());
    assert_eq!(view_program_command(&cmd, &view), cmd);
    assert_eq!(view_program_command("pwsh -NoLogo", &view), "pwsh -NoLogo");
    assert_eq!(view_program_command("./x.exe", &view), "./x.exe");
}

/// 守门进程的入参是**扁平** JSON：FenceSpec 的字段同层再加一个 prepared；
/// 缺字段一律报错（不默认成"有授权"——那会把容器送进一个读不到东西的环境）。
#[test]
fn fence_job_round_trips_and_rejects_incomplete_json() {
    let spec = FenceSpec {
        agent: "a".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![PathBuf::from("demo").join("work")],
        cwd: PathBuf::from("mods").join("m0"),
        ro: Vec::new(),
        net: false,
    };
    let job = FenceJob {
        spec,
        prepared: true,
        home: Some(PathBuf::from("home")),
        policy: Default::default(),
    };
    let text = job.to_json();
    assert!(text.contains("\"prepared\":true"), "{}", text);
    assert_eq!(FenceJob::from_json(&text).expect("回读守门进程入参"), job);
    // 没有台账可落是合法形态（探针），所以 home 允许缺省；prepared 缺了才报错。
    let bare = FenceSpec {
        agent: "b".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![PathBuf::from("demo").join("work")],
        cwd: PathBuf::from("mods").join("m1"),
        ro: Vec::new(),
        net: true,
    };
    let no_home = FenceJob {
        spec: bare,
        prepared: false,
        home: None,
        policy: Default::default(),
    };
    assert_eq!(
        FenceJob::from_json(&no_home.to_json()).expect("回读"),
        no_home
    );
    assert!(FenceJob::from_json("{}").is_err(), "缺字段必须报错，不猜");
    assert!(FenceJob::from_json("这不是 JSON").is_err());
}

/// 解析解释器不能假设环境形状：PATHEXT 缺席（真机 CI 上见过）时靠标准兜底扩展名照样找到 .exe，
/// 带引号的 PATH 项照样能用——否则容器里的工具连解释器都找不到。
#[test]
fn interpreter_dirs_resolves_without_pathext_and_with_quoted_path_entries() {
    let root = crate::tests::scratch("interpreter-dirs");
    let pydir = root.join("pydir");
    std::fs::create_dir_all(&pydir).expect("建解释器目录");
    let exe = pydir.join(if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    });
    std::fs::write(&exe, b"#!/bin/sh\nexit 0\n").expect("放一个假解释器");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&exe).expect("读权限位").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&exe, perm).expect("加执行位");
    }
    let path = pydir.as_os_str();
    assert_eq!(
        interpreter_dirs_in("python tools/x.py", path, None),
        vec![pydir.clone()],
        "PATHEXT 缺席也要解析出解释器目录"
    );
    assert_eq!(
        interpreter_dirs_in("python tools/x.py", path, Some(".EXE;.BAT")),
        vec![pydir.clone()],
        "PATHEXT 在场照走 PATHEXT"
    );
    let quoted = std::ffi::OsString::from(format!("\"{}\"", pydir.display()));
    assert_eq!(
        interpreter_dirs_in("python tools/x.py", &quoted, None),
        vec![pydir.clone()],
        "带引号的 PATH 项也要能解析"
    );
    assert!(
        interpreter_dirs_in(&format!("cat {}", exe.display()), path, None)
            .iter()
            .all(|d| d != &pydir),
        "数据文件路径不算解释器（否则等于给围栏开洞）"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 命令里的解释器要按 PATH 解析出真实路径，并给出它的安装目录；
/// 数据文件路径不算解释器（否则会把那个文件放行，等于开洞）。
#[test]
fn interpreter_dirs_resolves_programs_but_not_data_files() {
    let cmd = if cfg!(windows) {
        "cmd /C echo hi"
    } else {
        "sh -c 'echo hi'"
    };
    let dirs = interpreter_dirs(cmd);
    assert!(!dirs.is_empty(), "系统 shell 应当能被解析出来：{:?}", dirs);
    for d in &dirs {
        assert!(d.is_dir(), "只报真实存在的目录：{:?}", d);
        assert!(d.parent().is_some(), "绝不报文件系统根：{:?}", d);
    }
    // 明确的非程序路径（一个不存在的文件）不该被当成解释器。
    let missing = if cfg!(windows) {
        "C:\\nope\\nope.exe"
    } else {
        "/nope/nope"
    };
    assert!(
        interpreter_dirs(&format!("cat {}", missing))
            .iter()
            .all(|d| !d.ends_with("nope")),
        "数据/缺失路径不该被当成解释器"
    );
}

/// 目的：命令经目录重解析点（符号链接 / 联接）进来时，Windows 只授**真身**安装根；
///   链接对象自身的 DACL 不参与跟随（真机：链接没有 AAPP 也能跑 exe）。
#[cfg(windows)]
#[test]
fn interpreter_roots_skip_a_reparse_point_parent_and_use_the_real_root() {
    let root = crate::tests::scratch("interpreter-reparse");
    let real = root.join("real");
    let link = root.join("nodejs");
    std::fs::create_dir_all(&real).expect("建真身目录");
    std::fs::write(real.join("node.exe"), b"stub").expect("放假 node");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&real)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !made {
        eprintln!("[探针] 本机建不出目录联接（env-skip）：跳过重解析点授权口径");
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    assert_eq!(
        interpreter_roots_for_exe(&link.join("node.exe")),
        vec![real.clone()],
        "只授真身安装根，不授链接父目录"
    );
    let _ = std::fs::remove_dir(&link);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：只有 exe 本身是链接、父目录是实目录时，两边都留（运行期兄弟文件可能在链接旁）。
#[cfg(windows)]
#[test]
fn interpreter_roots_keep_both_sides_for_a_file_symlink() {
    use std::os::windows::fs::symlink_file;
    let root = crate::tests::scratch("interpreter-file-link");
    let real = root.join("real");
    let tools = root.join("tools");
    std::fs::create_dir_all(&real).expect("建真身目录");
    std::fs::create_dir_all(&tools).expect("建工具目录");
    std::fs::write(real.join("node.exe"), b"stub").expect("放假 node");
    let made = symlink_file(real.join("node.exe"), tools.join("node.exe")).is_ok();
    if !made {
        eprintln!("[探针] 本机建不出文件符号链接（env-skip）：跳过文件链接的安装根口径");
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let roots = interpreter_roots_for_exe(&tools.join("node.exe"));
    assert!(
        roots.contains(&tools),
        "链接父目录要留（运行期兄弟文件）：{:?}",
        roots
    );
    assert!(roots.contains(&real), "真身安装根也要留：{:?}", roots);
    let _ = std::fs::remove_dir_all(&root);
}

/// 解释器基线：`node <文件>` 的命令要带上"跳过 realpath"的两个开关；别的解释器、命令里的数据文件路径都不带。
/// 判据与解释器目录同一份 PATH 解析（PATHEXT 兜底、带引号的 PATH 项、按名解析才认）。
#[cfg(windows)]
#[test]
fn node_commands_get_the_realpath_skip_and_nothing_else_does() {
    let root = crate::tests::scratch("interpreter-node");
    let dir = root.join("nodedir");
    std::fs::create_dir_all(&dir).expect("建解释器目录");
    std::fs::write(dir.join("node.exe"), b"stub").expect("放假 node");
    std::fs::write(dir.join("python.exe"), b"stub").expect("放假 python");
    let path = dir.as_os_str();

    let (key, value) =
        node_realpath_env_in("node tools/report.js", path, None).expect("node 命令要带上开关");
    assert_eq!(key, OsString::from("NODE_OPTIONS"));
    let value = value.to_string_lossy().to_string();
    // 两个开关都要：只开一个，另一半照样 realpath（真机上量过）。
    for flag in ["--preserve-symlinks", "--preserve-symlinks-main"] {
        assert!(value.contains(flag), "缺开关 {}：{}", flag, value);
    }
    assert!(
        node_realpath_env_in("node.exe tools/report.js", path, None).is_some(),
        "带扩展名写 node 也要认（标准兜底扩展名）"
    );
    let quoted = std::ffi::OsString::from(format!("\"{}\"", dir.display()));
    assert!(
        node_realpath_env_in("node tools/report.js", &quoted, None).is_some(),
        "带引号的 PATH 项也要能解析"
    );
    assert!(
        node_realpath_env_in("python tools/x.py", path, None).is_none(),
        "别的解释器不带它（免得在不需要的地方改模块解析语义）"
    );
    assert!(
        node_realpath_env_in("cmd /C echo hi", path, None).is_none(),
        "命令里没有 node 就不带"
    );
    assert!(
        node_realpath_env_in("cat C:\\nope\\nope.exe", path, None).is_none(),
        "数据文件路径不算程序（与解释器目录同一口径）"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 非 node 命令在三平台都不带解释器开关：那是 Windows 容器独有的补丁（另两个平台 realpath 正常）。
#[test]
fn non_node_commands_carry_no_interpreter_switch() {
    let spec = FenceSpec {
        agent: "a".to_string(),
        lease: String::new(),
        rw: Vec::new(),
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: PathBuf::new(),
        cwd: PathBuf::from("mods").join("m0"),
        net: false,
    };
    let keys: Vec<String> = fence_env(&spec, "python tools/x.py")
        .into_iter()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    assert!(
        !keys.iter().any(|k| k == "NODE_OPTIONS"),
        "非 node 命令不该带 NODE_OPTIONS：{:?}",
        keys
    );
}

/// 围栏边界说明只在围栏内**失败**时给：成功不打扰、没强制生效不谈边界。
#[test]
fn fence_boundary_note_only_on_enforced_failure() {
    let spec = FenceSpec {
        agent: "a".to_string(),
        lease: String::new(),
        rw: vec![PathBuf::from("demo").join("sandbox")],
        ro: Vec::new(),
        ro_tree: vec![PathBuf::from("mods").join("m0")],
        private: PathBuf::from("demo").join("sandbox"),
        cwd: PathBuf::from("mods").join("m0"),
        net: false,
    };
    assert!(
        fence_boundary_note(&spec, true, true, 0).is_none(),
        "成功不打扰"
    );
    assert!(
        fence_boundary_note(&spec, false, true, 1).is_none(),
        "没强制生效就不谈边界"
    );
    let note = fence_boundary_note(&spec, true, true, 1).expect("围栏内失败要有边界说明");
    assert!(note.contains("范围外的访问被围栏拒绝"), "{}", note);
    assert!(note.contains("（网：关）"), "{}", note);
    assert!(note.contains("mods"), "要报出可达范围：{}", note);
    // 网络没装上时不能说“关”：那会让用户以为断网生效了。
    let degraded = fence_boundary_note(&spec, true, false, 1).expect("文件系统围栏失败也要说明");
    assert!(degraded.contains("（网：未生效）"), "{}", degraded);
    // 放行网络的失败仍报“开”，不把允许说成阻断。
    let mut allowed = spec;
    allowed.net = true;
    let note = fence_boundary_note(&allowed, true, true, 1).expect("失败说明");
    assert!(note.contains("（网：开）"), "{}", note);
}

/// 可达范围摘要：重复的根只报一次；根多时折叠，失败说明不因根多而失控。
#[test]
fn reachable_roots_dedupes_and_collapses() {
    let mut spec = FenceSpec {
        agent: "a".to_string(),
        lease: String::new(),
        rw: vec![PathBuf::from("r1")],
        ro: Vec::new(),
        ro_tree: vec![PathBuf::from("r1")],
        private: PathBuf::from("r1"),
        cwd: PathBuf::from("r1"),
        net: false,
    };
    assert_eq!(reachable_roots(&spec), "r1", "重复的根只报一次");
    spec.rw = (1..=6).map(|i| PathBuf::from(format!("r{}", i))).collect();
    let many = reachable_roots(&spec);
    assert!(many.starts_with("r1"), "{}", many);
    assert!(many.ends_with("等 6 处"), "{}", many);
}

/// 空范围/空根与平台 shell 启动都要有确定行为：说明不能凭空编路径，shell 不能拼字符串。
#[test]
fn boundary_helpers_cover_empty_scope_and_platform_shell() {
    let empty = FenceSpec {
        agent: "a".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: Vec::new(),
        cwd: PathBuf::new(),
        ro: Vec::new(),
        net: false,
    };
    assert_eq!(reachable_roots(&empty), "（无）");
    note_fence_boundary(&empty, true, false, 1);
    note_fence_boundary(&empty, false, false, 1);

    #[cfg(not(windows))]
    {
        let cmd = shell_command("echo hi");
        assert_eq!(cmd.get_program(), "sh");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["-c".to_string(), "echo hi".to_string()]);
    }
}

/// 解释器是符号链接时，安装目录要跟到真身所在的一层（只授链接目录会漏掉同目录的库）。
#[cfg(unix)]
#[test]
fn interpreter_roots_follow_symlink_to_the_real_install_dir() {
    use std::os::unix::fs::symlink;
    let root = crate::tests::scratch("confine-interpreter-link");
    let real_bin = root.join("venv").join("bin");
    std::fs::create_dir_all(&real_bin).expect("建真实解释器目录");
    let real = real_bin.join("python");
    std::fs::write(&real, b"#!/bin/sh\n").expect("写真实解释器");
    let link = root.join("python-link");
    symlink(&real, &link).expect("建符号链接");

    let dirs = interpreter_roots_for_exe(&link);
    assert!(
        dirs.iter().any(|d| d.ends_with("venv")),
        "要跟到真身的安装目录：{:?}",
        dirs
    );
    std::fs::remove_dir_all(&root).ok();
}

/// 会话租约进归属身份：同一进程里同名 agent 的两个会话是**两个**归属，释放其中一个不许动另一个。
#[test]
fn owner_lease_tells_sessions_apart() {
    let mk = |lease: &str| Owner {
        pid: 1,
        start: 2,
        lease: lease.to_string(),
    };
    let a = mk("s1");
    let b = mk("s2");
    assert_eq!(a, a.clone(), "完全相同的归属用于去重/撤销");
    assert_ne!(a, b, "同进程的两个会话是两个归属");
    assert_eq!(b.lease, "s2");
}

/// 对账摘要必须把"回收了什么 / 跳过了什么 / 失败了什么"都说清楚：这是启动期唯一的安全交代。
#[test]
fn reconcile_summary_reports_every_outcome_without_silent_success() {
    let empty = ReconcileReport::default();
    assert!(!empty.has_activity(), "全零且无错误时保持安静");
    assert_eq!(empty.summary(), "没有陈旧授权，无需回收");

    let skipped = ReconcileReport {
        skipped_unjudgeable: 2,
        ..Default::default()
    };
    assert!(skipped.has_activity());
    assert!(
        skipped.summary().contains("2 条无法判定已跳过"),
        "{}",
        skipped.summary()
    );

    let orphan = ReconcileReport {
        orphan_profiles: vec!["p1".to_string(), "p2".to_string()],
        ..Default::default()
    };
    assert!(orphan.has_activity());
    assert!(
        orphan.summary().contains("台账外还有 2 个容器 profile"),
        "{}",
        orphan.summary()
    );

    let note = ReconcileReport {
        note: "本平台没有台账".to_string(),
        orphan_profiles: vec!["p1".to_string()],
        ..Default::default()
    };
    assert!(note.summary().starts_with("本平台没有台账"));
    assert!(note.summary().contains("台账外还有 1 个容器 profile"));

    let reclaimed = ReconcileReport {
        reclaimed_grants: 3,
        restored_snapshots: 2,
        deleted_profiles: 1,
        kept: 4,
        ..Default::default()
    };
    assert!(reclaimed.has_activity());
    let text = reclaimed.summary();
    assert!(text.contains("还原快照 2 处"), "{}", text);
    assert!(text.contains("撤销授权 3 条"), "{}", text);
    assert!(text.contains("删除 profile 1 个"), "{}", text);
    assert!(text.contains("保留 4 条"), "{}", text);

    let reclaimed_skipped = ReconcileReport {
        reclaimed_grants: 1,
        skipped_unjudgeable: 5,
        ..Default::default()
    };
    assert!(
        reclaimed_skipped
            .summary()
            .contains("另有 5 条无法判定已跳过")
    );

    let failed_only = ReconcileReport {
        errors: vec!["写回被拒".to_string()],
        ..Default::default()
    };
    assert!(failed_only.summary().contains("台账保留供重试"));
    assert!(
        !failed_only.summary().contains("另有"),
        "没有跳过就不编跳过"
    );

    let failed = ReconcileReport {
        errors: vec!["写回被拒".to_string(), "路径不在了".to_string()],
        restored_snapshots: 1,
        reclaimed_grants: 2,
        deleted_profiles: 3,
        skipped_unjudgeable: 4,
        orphan_profiles: vec!["p".to_string()],
        ..Default::default()
    };
    let text = failed.summary();
    assert!(
        text.starts_with("对账未完成：写回被拒；路径不在了"),
        "{}",
        text
    );
    assert!(text.contains("已还原 1 处"), "{}", text);
    assert!(text.contains("撤销 2 条"), "{}", text);
    assert!(text.contains("删 3 个 profile"), "{}", text);
    assert!(text.contains("另有 4 条无法判定已跳过"), "{}", text);
    assert!(text.contains("台账外还有 1 个容器 profile"), "{}", text);
    assert!(failed.has_activity());

    #[cfg(not(windows))]
    {
        let home = std::path::Path::new("unused");
        assert!(reconcile(home).summary().contains("本平台"));
        assert!(ledger(home).note.contains("本平台"));
        assert!(restore_one(home, std::path::Path::new("a")).is_err());
        assert!(revoke_grant(home, "S-1", std::path::Path::new("a")).is_err());
        assert!(remove_profile_one(home, "p").is_err());
    }
}
