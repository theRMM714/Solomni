//! 目的：进程入口的两段分派（取根前 / 取根后）——隐藏探针自带参数失败路径，不碰真实的用户根。
//! 管：`before_root` / `after_root` 的分支走向与退出码。
//! 不管：探针自身的事实输出与平台差异（在 `src/entry/probes/`）；参数表之外的启动装配（在 `src/main.rs`）。
//! 联动：`src/main.rs` 的两处调用点；参数表与分支顺序在 `src/entry/args.rs`。

use crate::entry::args::{after_root, before_root};
use crate::kernel::detail::confine::FENCE_FAILED;

/// 目的：按真实调用的形状造 argv（第 0 位是程序名）。
fn argv(rest: &[&str]) -> Vec<String> {
    let mut out = vec!["solomni".to_string()];
    out.extend(rest.iter().map(|s| s.to_string()));
    out
}

/// 目的：临时产品根——探针要一个已规范化的根，用完连目录一起收掉。
struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("solomni-entry-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时根");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 目的：没有隐藏参数时两段分派都放行（`None` = 继续启动），这是正常启动的唯一走向。
#[test]
fn no_hidden_flag_lets_the_startup_continue() {
    let root = TempRoot::new("plain");
    assert_eq!(before_root(&argv(&[])), None, "取根前不拦");
    assert_eq!(after_root(root.path(), &argv(&[])), None, "取根后也不拦");
}

/// 目的：取根前的隐藏探针缺参数时快速失败——不装配核心、不碰产品根。
/// 约束：失败码用各探针自己的约定（argparse 类用法错是 2，围栏协议坏是 FENCE_FAILED）。
#[test]
fn probes_before_root_fail_fast_when_their_arguments_are_missing() {
    assert_eq!(
        before_root(&argv(&["--unpack"])),
        Some(2),
        "--unpack 缺归档与目标目录"
    );
    assert_eq!(
        before_root(&argv(&["--fence-verify"])),
        Some(FENCE_FAILED),
        "--fence-verify 缺守门 JSON：如实报协议坏，不是假装通过"
    );
    assert_eq!(
        before_root(&argv(&["--print-fence-env"])),
        Some(FENCE_FAILED),
        "--print-fence-env 同样只认守门 JSON"
    );
}

/// 目的：路由目录是二进制契约——`--print-routes` 打成它就该以 0 退出（内容由入口契约测试钉住）。
#[test]
fn print_routes_is_a_successful_one_shot() {
    assert_eq!(before_root(&argv(&["--print-routes"])), Some(0));
}

/// 目的：取根后的维护探针在空根上给出结论就退出，不需要先装好任何东西。
#[test]
fn probes_after_root_answer_on_a_bare_root() {
    let root = TempRoot::new("probe");
    assert_eq!(
        after_root(root.path(), &argv(&["--work-gc"])),
        Some(2),
        "--work-gc 缺工作名：给用法就退出，不去扫码"
    );
    assert_eq!(
        after_root(root.path(), &argv(&["--storage"])),
        Some(0),
        "空根也能报存储空间（只读事实）"
    );
    assert_eq!(
        after_root(root.path(), &argv(&["--gc"])),
        Some(0),
        "默认 dry-run：没有候选也如实说没有"
    );
}

/// 目的：`--fence-revoke` 只有一套解析、两种 arity——单参数撤机器级条目，双参数走旧的 SID+路径语义。
/// 约束：断的是参数分派（不真机写 ACL）；路径不在时 Windows 报失败、非 Windows 没有 ACL 可撤。
#[test]
fn fence_revoke_splits_arity_without_crossing_meanings() {
    let root = TempRoot::new("revoke");
    std::fs::create_dir_all(root.path().join(".home")).expect("建私有区");
    let here = root.path().to_str().expect("临时路径是 UTF-8").to_string();
    assert_eq!(
        after_root(root.path(), &argv(&["--fence-revoke", here.as_str()])),
        Some(0),
        "单参数：撤该路径的机器级条目（没有记录也照成功退）"
    );
    let inline = format!("--fence-revoke={here}");
    assert_eq!(
        after_root(root.path(), &argv(&[inline.as_str()])),
        Some(0),
        "单参数的 = 形式同义"
    );
    assert_eq!(
        after_root(root.path(), &argv(&["--fence-revoke", "S-1-15-2-1"])),
        Some(2),
        "单参数只给 SID：缺路径按用法错，不能把 SID 当路径吃掉"
    );
    let missing = root.path().join("missing");
    let missing = missing.to_str().expect("临时路径是 UTF-8");
    let expected = if cfg!(windows) { 1 } else { 0 };
    assert_eq!(
        after_root(root.path(), &argv(&["--fence-revoke", missing])),
        Some(expected),
        "单参数 + 路径不在：Windows 失败 1，非 Windows 无 ACL 可撤"
    );
    assert_eq!(
        after_root(
            root.path(),
            &argv(&["--fence-revoke", "S-1-5-21-1-2-3-1001", missing])
        ),
        Some(1),
        "双参数：按 SID+路径撤一条，路径不在是失败 1（不是用法错 2）"
    );
    assert_eq!(
        after_root(root.path(), &argv(&["--fence-revoke"])),
        Some(2),
        "零参数：只给两行用法"
    );
}
