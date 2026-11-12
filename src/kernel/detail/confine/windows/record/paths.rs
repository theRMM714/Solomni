//! 目的：台账里路径的归属判定——产品根是哪个目录、一个路径在不在根内。
//! 管：产品根（.home 的父目录）的推导，以及「根内存原始安全描述符、根外只记 ACE 摘要」的判定。
//! 不管：路径怎么规范化成文本（conductor 侧）；按判定结果选记账方式的是 journal.rs。
//! 联动：windows/mod.rs 的 prepare_fence 用 product_root 分组授权。

use std::path::{Component, Path, PathBuf};

/// 目的：取产品根（.home 的父目录）：授权落在产品根内还是根外，决定收尾用快照还原还是精确撤权。
pub(crate) fn product_root(home: &Path) -> PathBuf {
    match home.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => home.to_path_buf(),
    }
}

/// 目的：判断一个路径是否在产品根内（逐分量比，大小写不敏感；能规范化就按规范化路径比）。
pub(crate) fn inside_root(path: &Path, root: &Path) -> bool {
    let canon = |p: &Path| {
        std::fs::canonicalize(p)
            .map(|c| crate::kernel::domain::path::native(&c))
            .ok()
    };
    let (path_owned, root_owned) = match (canon(path), canon(root)) {
        (Some(p), Some(r)) => (p, r),
        // 规范化失败时不做词法比较：含上级段的路径无法证明仍在根内，
        // 按根外处理（否则 root/../escape 会被误判为根内而多存快照）。
        _ if path.components().any(|c| matches!(c, Component::ParentDir)) => {
            return false;
        }
        _ => (path.to_path_buf(), root.to_path_buf()),
    };
    let mut left = path_owned.components();
    for want in root_owned.components() {
        let ok = left.next().map(|got| {
            got.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&want.as_os_str().to_string_lossy())
        });
        if ok != Some(true) {
            return false;
        }
    }
    true
}
