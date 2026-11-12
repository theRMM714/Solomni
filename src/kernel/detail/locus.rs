//! 目的：落点身份——同一物理目录的不同写法（盘符 / 8.3 短名 / UNC 别名 / 映射盘）归一到同一个身份。
//! 管：取路径的身份、判两条路径是否同一落点、判一个根是否物理包含另一个根、判路径是不是网络路径（UNC 与映射盘）。
//! 不管：路径的文本规范化（在 kernel::domain::path）；谁能绑哪个根（策略在 conductor）。
//! 联动：工作区绑定、加载复检与跨会话锁按它判「同一目录」——文本比较会被 UNC 别名与映射盘骗过去。

use crate::kernel::domain::path::Locus;
use std::path::Path;

/// 目的：取一条路径的落点身份（跟随符号链接，取真身）。
/// 约束：网络共享不保证返回稳定 id；失败如实报错，由调用方按「不知道」处理。
pub fn locus_of(path: &Path) -> std::io::Result<Locus> {
    locus_raw(path)
}

/// 目的：ancestor 是否物理包含 descendant（同一落点也算包含）。
/// 约束：沿 descendant 的各级祖先逐级取身份；任一级取不到就跳过。UNC 别名（\\localhost\D$ 与 D:\）
///   在这里判成同一个根，文本 starts_with 判不出来。
pub fn place_contains(ancestor: &Path, descendant: &Path) -> bool {
    let Ok(target) = locus_of(ancestor) else {
        return false;
    };
    descendant
        .ancestors()
        .any(|p| locus_of(p).map(|l| l == target).unwrap_or(false))
}

/// 目的：这条路径是不是网络路径——UNC（\\server\share 与 //server/share）或映射到网络共享的盘符。
/// 约束：非 Windows 恒为 false（POSIX 的 //x 是普通绝对路径，没有网络语义）。
pub fn is_network_path(path: &Path) -> bool {
    network_path(path)
}

#[cfg(windows)]
fn locus_raw(path: &Path) -> std::io::Result<Locus> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };

    // 目录要 FILE_FLAG_BACKUP_SEMANTICS 才打得开；文件同样可用。
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = std::mem::MaybeUninit::<FILE_ID_INFO>::zeroed();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    Ok(Locus {
        volume: info.VolumeSerialNumber,
        id: u128::from_le_bytes(info.FileId.Identifier),
    })
}

#[cfg(unix)]
fn locus_raw(path: &Path) -> std::io::Result<Locus> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::metadata(path)?;
    Ok(Locus {
        volume: md.dev(),
        id: u128::from(md.ino()),
    })
}

#[cfg(not(any(windows, unix)))]
fn locus_raw(_path: &Path) -> std::io::Result<Locus> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "这个平台没有落点身份",
    ))
}

#[cfg(windows)]
fn network_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_REMOTE;

    if crate::kernel::domain::path::is_unc(path) {
        return true;
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };
    // 映射盘：句柄的 final path 是 UNC，但用户给的是盘符——按盘符类型判，不触网。
    let letter = match prefix.kind() {
        Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d,
        _ => return false,
    };
    let root = [letter as u16, b':' as u16, b'\\' as u16, 0];
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
}

#[cfg(not(windows))]
fn network_path(_path: &Path) -> bool {
    false
}
