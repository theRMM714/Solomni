//! 目的：便携维护（--setup）的单元测试：PATH 分段纯函数、端口替身驱动的 path/unset/undo、shim 与状态 JSON。
//! 管：分段去重与幂等、备份与还原、端口错误透传、shim 渲染、状态字段。
//! 不管：注册表与广播的真实行为（归 kernel 的 UserPath 适配器）；真实用户 PATH 与用户目录不被触碰。
//! 联动：被测面 src/capabilities/system/detail/setup/ 下的 mod.rs、path.rs、shim.rs、fence.rs；端口契约见 src/kernel/ports.rs 的 UserPath。

use crate::capabilities::system::api::{SetupMode, SetupOps};
use crate::capabilities::system::detail::setup::SetupHost;
#[cfg(not(windows))]
use crate::capabilities::system::detail::setup::fence;
use crate::capabilities::system::detail::setup::path as p;
#[cfg(windows)]
use crate::capabilities::system::detail::setup::shim;
use crate::kernel::ports::{PathValue, UserPath};
use std::sync::Mutex;

/// 目的：UserPath 端口的替身：一次读、写调用全记录、广播计数；错误用 Err 注入。
struct FakeUserPath {
    value: Mutex<Result<PathValue, String>>,
    writes: Mutex<Vec<PathValue>>,
    broadcasts: Mutex<usize>,
}

impl FakeUserPath {
    fn new(text: &str, expand: bool) -> Self {
        Self {
            value: Mutex::new(Ok(PathValue {
                text: text.to_string(),
                expand,
            })),
            writes: Mutex::new(Vec::new()),
            broadcasts: Mutex::new(0),
        }
    }

    #[cfg(windows)]
    fn failing(why: &str) -> Self {
        let fake = Self::new("", false);
        *fake.value.lock().expect("锁") = Err(why.to_string());
        fake
    }

    fn text(&self) -> String {
        self.writes
            .lock()
            .expect("锁")
            .last()
            .map(|v| v.text.clone())
            .expect("有写入")
    }

    fn write_count(&self) -> usize {
        self.writes.lock().expect("锁").len()
    }

    #[cfg(windows)]
    fn broadcast_count(&self) -> usize {
        *self.broadcasts.lock().expect("锁")
    }
}

impl UserPath for FakeUserPath {
    fn read(&self) -> Result<PathValue, String> {
        self.value.lock().expect("锁").clone()
    }

    fn write(&self, value: &PathValue) -> Result<(), String> {
        self.writes.lock().expect("锁").push(value.clone());
        *self.value.lock().expect("锁") = Ok(value.clone());
        Ok(())
    }

    fn broadcast(&self) -> Result<(), String> {
        *self.broadcasts.lock().expect("锁") += 1;
        Ok(())
    }
}

/// 目的：比较用的规范化：引号、尾部分隔符、大小写都不影响判定。
#[test]
fn norm_ignores_quotes_trailing_separator_and_case() {
    assert_eq!(
        p::norm("  \"C:\\Tools\\Solomni\\bin\\\" "),
        "c:\\tools\\solomni\\bin"
    );
    assert!(p::contains(
        "C:\\Windows;\"C:\\Tools\\Solomni\\bin\\\"",
        "c:\\tools\\solomni\\bin"
    ));
    assert!(!p::contains(
        "C:\\Windows;C:\\Tools\\Solomni\\binx",
        "c:\\tools\\solomni\\bin"
    ));
}

/// 目的：追加幂等：重复执行只保留一段，原有段不丢、顺序不变，重复段摘掉。
#[test]
fn append_keeps_one_copy_at_the_end() {
    let bin = "C:\\Tools\\Solomni\\bin";
    let once = p::append("C:\\Windows;C:\\Other", bin);
    assert_eq!(once, format!("C:\\Windows;C:\\Other;{}", bin));
    assert_eq!(p::append(&once, bin), once, "重复执行不再增长");
    let messy = format!("C:\\Windows;{};C:\\Other;{}", bin, bin.to_lowercase());
    assert_eq!(
        p::append(&messy, bin),
        format!("C:\\Windows;C:\\Other;{}", bin)
    );
    assert_eq!(p::append("", bin), bin, "空 PATH 不产生前导分号");
}

/// 目的：摘除只动我们那段（所有位置），别的段原样保留。
#[test]
fn remove_takes_only_our_segment() {
    let bin = "C:\\Tools\\Solomni\\bin";
    let (text, found) = p::remove(&format!("C:\\Windows;{};C:\\Other", bin), bin);
    assert!(found);
    assert_eq!(text, "C:\\Windows;C:\\Other");
    let (text, found) = p::remove("C:\\Windows", bin);
    assert!(!found);
    assert_eq!(text, "C:\\Windows");
}

/// 目的：备份文件是可读回的 JSON（undo 的唯一依据）。
#[test]
fn backup_roundtrips_through_json() {
    let backup = p::PathBackup {
        v: 1,
        original: "C:\\Windows".to_string(),
        expand: true,
        bin: "C:\\Tools\\Solomni\\bin".to_string(),
        at: 42,
    };
    let text = serde_json::to_string(&backup).expect("序列化");
    let back: p::PathBackup = serde_json::from_str(&text).expect("反序列化");
    assert_eq!(back, backup);
    assert!(text.contains("\"v\":1") && text.contains("\"expand\":true"));
}

/// 目的：无备份时 undo 如实失败并给出下一步，不猜原串。
#[test]
fn undo_without_backup_fails_with_hint() {
    let root = crate::tests::scratch("setup-undo-none");
    let port = FakeUserPath::new("C:\\Windows", false);
    let err = p::do_undo(&port, &root.join(".home")).expect_err("没有备份要失败");
    assert!(
        err.contains("setup-path.json") && err.contains("--setup=unset"),
        "{err}"
    );
    assert_eq!(port.write_count(), 0, "失败不写");
}

/// 目的：Windows 上 path → 再 path（幂等）→ unset → undo 的完整往返：备份原串、类型不变、广播一次。
#[cfg(windows)]
#[test]
fn path_unset_undo_roundtrip() {
    let root = crate::tests::scratch("setup-path");
    let home = root.join(".home");
    let bin = p::bin_text(&root);
    let port = FakeUserPath::new("C:\\Windows;C:\\Other", true);

    let line = p::do_path(&port, &home, &root).expect("追加");
    assert!(line.contains("REG_EXPAND_SZ"), "{line}");
    assert_eq!(port.text(), format!("C:\\Windows;C:\\Other;{}", bin));
    assert_eq!(port.broadcast_count(), 1);
    assert!(shim::shim_path(&root).exists(), "shim 已落盘");
    let backup = std::fs::read_to_string(p::backup_path(&home)).expect("有备份");
    assert!(backup.contains("C:\\\\Windows;C:\\\\Other"), "{backup}");

    // 再跑一次：已有那段，不写、不再备份。
    let line = p::do_path(&port, &home, &root).expect("幂等");
    assert!(line.contains("无需改动"), "{line}");
    assert_eq!(port.write_count(), 1);

    let line = p::do_unset(&port, &root).expect("摘除");
    assert!(line.contains("摘掉"), "{line}");
    assert_eq!(port.text(), "C:\\Windows;C:\\Other");
    assert!(p::do_unset(&port, &root).expect("再摘").contains("未含"));
    assert_eq!(port.write_count(), 2);

    let line = p::do_undo(&port, &home).expect("还原");
    assert!(line.contains("还原"), "{line}");
    assert_eq!(port.text(), "C:\\Windows;C:\\Other");
    assert_eq!(port.write_count(), 3);
    assert!(!p::backup_path(&home).exists(), "还原后备份删除");
}

/// 目的：端口读失败如实上报，不写。
#[cfg(windows)]
#[test]
fn path_reports_port_read_failure() {
    let root = crate::tests::scratch("setup-path-err");
    let port = FakeUserPath::failing("注册表不可读");
    let err = p::do_path(&port, &root.join(".home"), &root).expect_err("要失败");
    assert!(err.contains("注册表不可读"), "{err}");
    assert_eq!(port.write_count(), 0);
}

/// 目的：shim 内容固定：带 --root 与参数透传，重复生成不重写。
#[cfg(windows)]
#[test]
fn shim_renders_root_and_passes_args() {
    let root = crate::tests::scratch("setup-shim");
    let exe = root.join("solomni.exe");
    let text = shim::render(&exe, &root);
    assert!(text.contains("--root"), "{text}");
    assert!(text.contains("%*"), "{text}");
    assert!(text.contains(&root.display().to_string()), "{text}");
    let (_, changed) = shim::ensure(&root).expect("生成 shim");
    assert!(changed);
    let (present, current) = shim::inspect(&root);
    assert!(present && current);
    let (_, changed) = shim::ensure(&root).expect("再生成");
    assert!(!changed, "内容相同不重写");
}

/// 目的：模式名只认表里那几条，省略值等于状态；状态只读，不改任何状态。
#[test]
fn unknown_mode_is_refused_and_omitted_mode_means_status() {
    let root = crate::tests::scratch("setup-dispatch");
    assert!(SetupMode::parse("bogus").is_none());
    assert_eq!(SetupMode::parse(""), Some(SetupMode::Status));
    let value = SetupHost::new(&root)
        .apply(SetupMode::Status, &mut |_| {})
        .expect("状态要读得到");
    assert_eq!(value["platform"], std::env::consts::OS);
}

/// 目的：状态只描述可观察事实：平台、根、PATH 段与围栏缺项都在，且结构稳定。
#[test]
fn status_reports_platform_root_and_fence() {
    let root = crate::tests::scratch("setup-status");
    let v = SetupHost::new(&root)
        .apply(SetupMode::Status, &mut |_| {})
        .expect("状态要读得到");
    assert_eq!(v["platform"], std::env::consts::OS);
    assert_eq!(v["root"], root.display().to_string());
    assert!(
        v["path"]["bin"]
            .as_str()
            .expect("bin 字段")
            .ends_with("bin")
    );
    assert!(v["fence"]["ready"].is_boolean());
}

/// 目的：unset 只摘我们那段且幂等；undo 按备份还原原串与原类型——两条都与平台无关（端口注入）。
#[test]
fn unset_and_undo_are_driven_by_the_port() {
    let root = crate::tests::scratch("setup-unset-undo");
    let home = root.join(".home");
    let bin = p::bin_text(&root);
    let port = FakeUserPath::new(&format!("C:\\Windows;{};C:\\Other", bin), true);

    let line = p::do_unset(&port, &root).expect("摘除");
    assert!(line.contains("摘掉"), "{line}");
    assert_eq!(port.text(), "C:\\Windows;C:\\Other");
    assert!(p::do_unset(&port, &root).expect("再摘").contains("未含"));
    assert_eq!(port.write_count(), 1, "第二次没写");

    std::fs::create_dir_all(&home).expect("建私有区");
    let backup = p::PathBackup {
        v: 1,
        original: "C:\\Windows".to_string(),
        expand: false,
        bin: bin.clone(),
        at: 42,
    };
    std::fs::write(
        p::backup_path(&home),
        serde_json::to_string(&backup).expect("序列化"),
    )
    .expect("写备份");

    let line = p::do_undo(&port, &home).expect("还原");
    assert!(line.contains("还原"), "{line}");
    assert_eq!(port.text(), "C:\\Windows");
    assert!(!p::backup_path(&home).exists(), "还原后备份删除");
}

/// 目的：POSIX 上维护命令只打印指引、退出码 0，不碰真实用户状态；机器级预授如实报未支持。
#[cfg(not(windows))]
#[test]
fn posix_modes_print_guidance_and_report_fence_unsupported() {
    let root = crate::tests::scratch("setup-posix");
    let host = SetupHost::new(&root);
    for mode in ["path", "unset", "undo", "fence", "all"] {
        let mode = SetupMode::parse(mode).expect("表里的模式");
        host.apply(mode, &mut |_| {})
            .unwrap_or_else(|e| panic!("POSIX 的 {:?} 只打印指引：{}", mode, e.message));
    }
    assert!(
        crate::entry::portable::fence_hint(&root),
        "非 Windows 无缺项"
    );
    let home = root.join(".home");
    // 首次授权那一趟在非 Windows 也走一次：如实收尾、不打印、不阻断。
    crate::entry::portable::ensure_after_authorize(&root);
    let err = fence::ensure(&root, &home, &mut |_| {}).expect_err("非 Windows 无机器级预授");
    assert!(err.contains("本平台"), "{err}");
    let err =
        crate::kernel::detail::confine::machine_missing(&root).expect_err("非 Windows 无预授");
    assert!(err.contains("本平台"), "{err}");
}

/// 目的：模式词的两种写法成对——命令面与 JSON 回执里的 id 再 parse 回来还是同一个模式。
#[test]
fn setup_modes_round_trip_through_their_ids() {
    for mode in [
        SetupMode::Path,
        SetupMode::Unset,
        SetupMode::Undo,
        SetupMode::Status,
        SetupMode::Fence,
        SetupMode::All,
    ] {
        assert_eq!(
            SetupMode::parse(mode.id()),
            Some(mode),
            "{} 要认回来",
            mode.id()
        );
    }
    // 省略模式词等于 status；认不出的词不猜。
    assert_eq!(SetupMode::parse("  "), Some(SetupMode::Status));
    assert_eq!(SetupMode::parse("bogus"), None);
    for id in SetupMode::list().split(" / ") {
        assert!(SetupMode::parse(id).is_some(), "清单里的 {} 必须能认", id);
    }
}

/// 目的：启动期的围栏对账——两个档位（有/没有 ACL 一步）与三种结果（就绪、缺项、对账失败）都说对话。
#[test]
fn portable_fence_report_covers_every_branch() {
    use crate::capabilities::system::api::Fault;
    use crate::entry::portable::{ensure_grants, fence_report};

    /// 目的：便携维护端口的替身：缺项与预授结果由字段决定，收到的进度行全记录。
    struct FakeHost {
        missing: Result<Vec<String>, String>,
        grant: Result<(), String>,
    }

    impl SetupOps for FakeHost {
        fn apply(
            &self,
            _mode: SetupMode,
            _on_line: &mut dyn FnMut(&str),
        ) -> Result<serde_json::Value, Fault> {
            Err(Fault::failed("用例不写"))
        }

        fn fence_missing(&self) -> Result<Vec<String>, String> {
            self.missing.clone()
        }

        fn fence_ensure(&self, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
            on_line("已预授");
            self.grant.clone()
        }
    }

    /// 目的：取出口收到的行（拷一份，免得借用与写入撞车）。
    fn said(cell: &Mutex<Vec<String>>) -> Vec<String> {
        cell.lock().expect("锁").clone()
    }

    fn host(missing: Result<Vec<String>, String>, grant: Result<(), String>) -> FakeHost {
        FakeHost { missing, grant }
    }

    let hints: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let warns: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let say = |l: &str| hints.lock().expect("锁").push(l.to_string());
    let warn = |l: &str| warns.lock().expect("锁").push(l.to_string());

    // 没有 ACL 一步的平台：不对账、也不提示。
    let plain = host(Err("本平台没有 ACL 一步".to_string()), Ok(()));
    assert!(fence_report(&plain, false, &say, &warn));
    assert!(said(&hints).is_empty(), "不该有提示");
    assert!(said(&warns).is_empty());

    // 就绪：什么都不说。
    let ready = host(Ok(Vec::new()), Ok(()));
    assert!(fence_report(&ready, true, &say, &warn));
    assert!(said(&hints).is_empty());

    // 缺项：一条提示，点名还缺几处、缺什么，并给出补的写法。
    let partial = host(
        Ok(vec!["产品根读属性".to_string(), "工具链落点".to_string()]),
        Ok(()),
    );
    assert!(!fence_report(&partial, true, &say, &warn));
    let hint = said(&hints).last().cloned().expect("有提示");
    assert!(
        hint.contains("还缺 2 处") && hint.contains("工具链落点") && hint.contains("--setup=fence"),
        "{hint}"
    );

    // 对账本身失败：只告警，按已就绪放行（不拦启动）。
    let broken = host(Err("注册表读不到".to_string()), Ok(()));
    assert!(fence_report(&broken, true, &say, &warn));
    let warning = said(&warns).last().cloned().expect("有告警");
    assert!(warning.contains("注册表读不到"), "{warning}");

    // 首次授权后的预授：没有 ACL 一步时静默走一趟（不声明、不打进度）。
    let quiet_out: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let quiet_say = |l: &str| quiet_out.lock().expect("锁").push(l.to_string());
    let quiet = host(Ok(Vec::new()), Err("不会走到".to_string()));
    ensure_grants(&quiet, false, &quiet_say, &warn);
    assert!(said(&quiet_out).is_empty(), "非 ACL 平台不声明也不打进度");

    // 有 ACL 一步：先声明一次，再把进度逐条交出来。
    let granted_out: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let granted_say = |l: &str| granted_out.lock().expect("锁").push(l.to_string());
    let granted = host(Ok(Vec::new()), Ok(()));
    ensure_grants(&granted, true, &granted_say, &warn);
    let lines = said(&granted_out);
    assert!(lines.iter().any(|l| l.contains("首次授权")), "{lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("[便携] 已预授")),
        "{lines:?}"
    );

    // 预授失败：告警一句并给出重跑写法，不拦重启。
    let failed_warns: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let failed_warn = |l: &str| failed_warns.lock().expect("锁").push(l.to_string());
    let failed = host(Ok(Vec::new()), Err("没有管理员权限".to_string()));
    ensure_grants(&failed, true, &granted_say, &failed_warn);
    let text = said(&failed_warns).join("\n");
    assert!(
        text.contains("没有管理员权限") && text.contains("--setup=fence"),
        "{text}"
    );
}

/// 目的：启动期的两个钩子对着真实端口走一趟：没有 ACL 一步的平台静默放行，端口自身如实报不可用。
/// 约束：有 ACL 一步的平台（Windows）由 --setup=fence 真机冒烟与三平台探针验收，这里只钉只读那一半。
#[test]
fn portable_hooks_pass_quietly_without_an_acl_step() {
    let root = std::env::temp_dir().join(format!("solomni-portable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("建根");

    // 只读对账：非 Windows 一律按已就绪放行（没有要预授的落点），不打印任何东西。
    let ready = crate::entry::portable::fence_hint(&root);
    if !crate::kernel::api::HAS_EXTERNAL_ACL_STEP {
        assert!(ready, "无 ACL 一步的平台不该提示缺项");
    }

    // 授权后那一趟：非 Windows 静默走一趟（结果被吞掉），不报错、也不写用户环境。
    crate::entry::portable::ensure_after_authorize(&root);

    // 端口自身不静默成功：非 Windows 上没有机器级落点，如实报不可用。
    if !crate::kernel::api::HAS_EXTERNAL_ACL_STEP {
        let err = SetupHost::new(&root)
            .fence_missing()
            .expect_err("非 Windows 没有机器级落点");
        assert!(err.contains("本平台"), "{err}");
    }

    let _ = std::fs::remove_dir_all(&root);
}
