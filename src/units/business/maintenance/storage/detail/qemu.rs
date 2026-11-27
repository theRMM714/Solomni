//! 目的：qcow2 的事实与压缩——actual/virtual size、corrupt 标记、qemu-img convert。
//! 管：只读 info 与显式压缩；不决定该不该删。
//! 不管：构建缓存卷怎么建（在 builds）、guest 怎么起（在 vm）。
//! 联动：storage::service 用它算实际占用；工具与构建共用项目内的 qemu-img。

use crate::kernel::api::fs;
use std::path::{Path, PathBuf};

/// 目的：一块磁盘镜像的 facts。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskInfo {
    pub actual: u64,
    pub virtual_size: u64,
    pub corrupt: bool,
}

/// 目的：项目内便携 QEMU 的 qemu-img；不在场给 None（调用方退回文件长度）。
pub fn find(root: &Path) -> Option<PathBuf> {
    let name = format!("qemu-img{}", crate::kernel::api::platform::EXE_SUFFIX);
    let p = crate::units::resource::vmstore::api::paths(root)
        .engines()
        .join("bin")
        .join(name);
    if p.is_file() { Some(p) } else { None }
}

/// 目的：读一块 qcow2 的 actual/virtual 与 corrupt；失败给 None。
pub fn info(qemu: &Path, path: &Path) -> Option<DiskInfo> {
    let args = vec![
        "info".to_string(),
        "--output=json".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    let out = crate::kernel::api::process::run_to_end(&crate::kernel::api::process::Run {
        program: qemu,
        args: &args,
        env: &[],
        cwd: None,
        timeout: std::time::Duration::from_secs(120),
        stdin: None,
    })
    .ok()?;
    if !out.ok() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&out.stdout_text()).ok()?;
    let actual = v.get("actual-size").and_then(|x| x.as_u64())?;
    let virtual_size = v.get("virtual-size").and_then(|x| x.as_u64()).unwrap_or(0);
    let corrupt = v
        .get("format-specific")
        .and_then(|x| x.get("data"))
        .and_then(|x| x.get("corrupt"))
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    Some(DiskInfo {
        actual,
        virtual_size,
        corrupt,
    })
}

/// 目的：把一块未被使用的 qcow2 压回宿主实际占用；返回压缩后的 actual。
/// 约束：同目录临时文件 + 双 rename；失败保留原卷并如实报错。
pub fn convert(qemu: &Path, path: &Path) -> Result<u64, String> {
    let temp = path.with_extension("compact.qcow2");
    let backup = path.with_extension("compact.bak");
    let _ = fs::remove_tree(&temp);
    let _ = fs::remove_tree(&backup);
    let args = vec![
        "convert".to_string(),
        "-O".to_string(),
        "qcow2".to_string(),
        path.to_string_lossy().into_owned(),
        temp.to_string_lossy().into_owned(),
    ];
    let out = crate::kernel::api::process::run_to_end(&crate::kernel::api::process::Run {
        program: qemu,
        args: &args,
        env: &[],
        cwd: None,
        timeout: std::time::Duration::from_secs(1800),
        stdin: None,
    })
    .map_err(|e| format!("起不了 qemu-img：{}", e))?;
    if !out.ok() {
        let _ = fs::remove_tree(&temp);
        return Err(format!("压缩失败：{}", out.stderr_text().trim()));
    }
    fs::move_file(path, &backup).map_err(|fault| {
        let _ = fs::remove_tree(&temp);
        format!("替换原卷失败：{}", fault.message)
    })?;
    if let Err(fault) = fs::move_file(&temp, path) {
        let _ = fs::move_file(&backup, path);
        let _ = fs::remove_tree(&temp);
        return Err(format!("放回压缩卷失败：{}", fault.message));
    }
    let _ = fs::remove_tree(&backup);
    Ok(info(qemu, path).map(|i| i.actual).unwrap_or(0))
}
