//! 目的：便携维护（--setup）的单元测试：PATH 分段纯函数、端口替身驱动的 path/unset/undo、shim 与状态 JSON。
//! 管：分段去重与幂等、备份与还原、端口错误透传、shim 渲染、状态字段。
//! 不管：注册表与广播的真实行为（归 kernel 的 UserPath 适配器）；真实用户 PATH 与用户目录不被触碰。
//! 联动：被测面 src/entry/setup.rs 与 setup/path.rs、setup/shim.rs；端口契约见 src/kernel/ports.rs 的 UserPath。

use crate::entry::setup::path as p;
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
    assert!(
        crate::entry::setup::shim::shim_path(&root).exists(),
        "shim 已落盘"
    );
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
    let text = crate::entry::setup::shim::render(&exe, &root);
    assert!(text.contains("--root"), "{text}");
    assert!(text.contains("%*"), "{text}");
    assert!(text.contains(&root.display().to_string()), "{text}");
    let (_, changed) = crate::entry::setup::shim::ensure(&root).expect("生成 shim");
    assert!(changed);
    let (present, current) = crate::entry::setup::shim::inspect(&root);
    assert!(present && current);
    let (_, changed) = crate::entry::setup::shim::ensure(&root).expect("再生成");
    assert!(!changed, "内容相同不重写");
}

/// 目的：--setup 分派：未知模式算用法错；status 只输出可解析的 JSON，不改任何状态。
#[test]
fn run_rejects_unknown_mode_and_status_is_json() {
    let root = crate::tests::scratch("setup-dispatch");
    assert_eq!(crate::entry::setup::run(&root, "bogus"), 2);
    assert_eq!(crate::entry::setup::run(&root, "status"), 0);
}

/// 目的：状态 JSON 只描述可观察事实：平台、根、PATH 段与围栏缺项都在，且结构稳定。
#[test]
fn status_json_reports_platform_root_and_fence() {
    let root = crate::tests::scratch("setup-status");
    let text = crate::entry::setup::status_json(&root).expect("状态要读得到");
    let v: serde_json::Value = serde_json::from_str(&text).expect("stdout 必须是 JSON");
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
    for mode in ["path", "unset", "undo", "fence", "all"] {
        assert_eq!(
            crate::entry::setup::run(&root, mode),
            0,
            "POSIX 的 {mode} 只打印指引"
        );
    }
    assert!(crate::entry::setup::fence_hint(&root), "非 Windows 无缺项");
    let home = root.join(".home");
    let err =
        crate::entry::setup::fence::ensure(&root, &home).expect_err("非 Windows 无机器级预授");
    assert!(err.contains("本平台"), "{err}");
    let err =
        crate::kernel::detail::confine::machine_missing(&root).expect_err("非 Windows 无预授");
    assert!(err.contains("本平台"), "{err}");
}
