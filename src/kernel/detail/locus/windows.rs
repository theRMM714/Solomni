//! 目的：`locus` 的 Windows 后端——卷序列号 + 文件 id，映射盘的网络判定。
//! 管：取落点身份、判 UNC 与映射盘。
//! 不管：逐级祖先比较（在门面）；路径文本规范化。
//! 联动：由 `src/kernel/detail/locus.rs` 按平台选中。

use crate::kernel::domain::path::Locus;
use std::path::Path;

/// 目的：取路径的落点身份（跟随符号链接）。
pub(super) fn locus_of(path: &Path) -> std::io::Result<Locus> {
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

/// 目的：判 UNC 或映射到网络共享的盘符（按盘符类型判，不触网）。
pub(super) fn network_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_REMOTE;

    if crate::kernel::domain::path::is_unc(path) {
        return true;
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };
    let letter = match prefix.kind() {
        Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d,
        _ => return false,
    };
    let root = [letter as u16, b':' as u16, b'\\' as u16, 0];
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
}
