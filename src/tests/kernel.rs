//! 目的：kernel 共享机制的契约测试——路径写法（native / UNC）与落点身份。
//! 管：本机制的不变式（verbatim 还原、UNC 拆分、落点身份与物理包含、网络判定）。
//! 不管：工作区绑定策略（`src/tests/workspace.rs`）与虚拟机档挂载（`src/tests/vm.rs`）。
//! 联动：职责与契约见 docs/kernel/unit-map.md 与 ARCHITECTURE.md。

use crate::kernel::api::{is_unc, locus_of, native, place_contains, unc_parts};

/// 目的：native 还原两种有普通形态的 verbatim 前缀，UNC 不能被剥成相对路径。
#[cfg(windows)]
#[test]
pub(crate) fn native_restores_verbatim_unc_and_disk_prefixes() {
    use std::path::{Path, PathBuf};
    assert_eq!(
        native(Path::new(r"\\?\UNC\server\share\dir")),
        PathBuf::from(r"\\server\share\dir"),
        r"UNC 的 verbatim 形态要还原成 \\server\share\…（不是相对路径 UNC\…）"
    );
    assert_eq!(
        native(Path::new(r"\\?\C:\dir\a.txt")),
        PathBuf::from(r"C:\dir\a.txt")
    );
    // 卷 GUID 没有普通写法：原样保留，免得还原成一条错路径。
    let volume = Path::new(r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\dir");
    assert_eq!(native(volume), volume.to_path_buf());
    // 普通写法不动。
    assert_eq!(native(Path::new(r"D:\proj")), PathBuf::from(r"D:\proj"));
}

/// 目的：UNC 的两种写法（\\ 与 //）都认得；盘符与普通目录不是 UNC。
#[cfg(windows)]
#[test]
pub(crate) fn unc_forms_are_recognised_and_split() {
    use std::path::Path;
    for p in [
        r"\\server\share",
        r"//server/share",
        r"\\server\share\dir\a.txt",
        r"\\?\UNC\server\share\dir",
    ] {
        assert!(is_unc(Path::new(p)), "要认成 UNC：{p}");
    }
    for p in [r"C:\proj", r"\\?\C:\proj", r"relative\dir"] {
        assert!(!is_unc(Path::new(p)), "不该认成 UNC：{p}");
    }
    let (server, share, rest) =
        unc_parts(Path::new(r"\\server\share\dir\a.txt")).expect("UNC 要能拆");
    assert_eq!(server.to_string_lossy(), "server");
    assert_eq!(share.to_string_lossy(), "share");
    assert_eq!(rest, std::path::PathBuf::from(r"dir\a.txt"));
    let (_, _, root_rest) = unc_parts(Path::new(r"\\server\share")).expect("共享根也是 UNC");
    assert!(root_rest.as_os_str().is_empty(), "共享根的相对路径为空");
    assert!(unc_parts(Path::new(r"C:\proj")).is_none(), "盘符不是 UNC");
}

/// 目的：落点身份按物理对象算——硬链接与本体是同一个落点，目录与其子孙不是。
/// 约束：两边都取得到身份才断言（取不到时调用方按「不知道」处理，这里不替它猜）。
#[test]
pub(crate) fn locus_identifies_hardlinks_as_the_same_place() {
    let dir = super::scratch("kernel-locus");
    let file = dir.join("a.txt");
    std::fs::write(&file, b"x").expect("写文件");
    let link = dir.join("b.txt");
    if let Err(e) = std::fs::hard_link(&file, &link) {
        // 某些文件系统（FAT / 部分容器挂载）不支持硬链接：跳过身份比较，不假装验过。
        eprintln!("[探针] 这个文件系统不支持硬链接（{e}）：跳过落点身份用例");
        return;
    }
    let a = locus_of(&file).expect("取文件身份");
    let b = locus_of(&link).expect("取硬链接身份");
    assert_eq!(a.key(), b.key(), "硬链接与本体是同一个落点");
    assert_ne!(
        locus_of(&dir).expect("取目录身份").key(),
        a.key(),
        "目录不是它里面的文件"
    );
    assert!(place_contains(&dir, &file), "目录物理包含它的文件");
    assert!(!place_contains(&file, &dir), "文件不包含它的目录");
    assert!(place_contains(&file, &link), "同一落点算包含");
}

/// 目的：网络路径的判定——UNC 是网络路径；本机盘符不是。
/// 约束：映射盘的判定只读盘符类型，不连共享；这里不依赖本机真有一块映射盘。
#[cfg(windows)]
#[test]
pub(crate) fn network_paths_are_recognised_on_windows() {
    use crate::kernel::api::is_network_path;
    use std::path::Path;
    assert!(is_network_path(Path::new(r"\\server\share\dir")));
    assert!(is_network_path(Path::new(r"//server/share")));
    assert!(!is_network_path(Path::new(r"C:\proj")));
    assert!(!is_network_path(&super::scratch("kernel-network")));
}
