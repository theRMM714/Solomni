//! 目的：本机残留清扫——容器 profile、孤儿 ACE 与空的按次目录。
//! 管：记一条 profile、按前缀找本程序建的 profile、删 profile、按路径扫孤儿 ACE、向下清扫空目录。
//! 不管：台账条目的处置（release.rs / catalog.rs）与启动对账的决策（reconcile.rs）。
//! 联动：容器名与 SID 派生在 container.rs；ACE 的读法在 acl.rs；台账写事务在 store.rs。

use super::super::*;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile;
use windows_sys::Win32::Security::PSID;

/// 目的：记下我们建过这个容器 profile（与 prepare_fence 共用同一份台账），供收尾精确回收。
/// 约束：owner 带会话租约——会话结束时按租约回收，进程崩溃时按进程回收。
pub(crate) fn record_profile(home: &Path, name: &str, lease: &str) {
    let _lock = match lock_ledger(home) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[围栏] 容器 profile 台账加锁失败（影响收尾的精确回收）：{}",
                e
            );
            return;
        }
    };
    let mut rec = load_record(home);
    if rec.profiles.iter().any(|p| p.name == name) {
        return;
    }
    let mut owner = current_owner();
    owner.lease = lease.to_string();
    rec.profiles.push(ProfileEntry {
        name: name.to_string(),
        at: now_secs(),
        owners: vec![owner],
    });
    if let Err(e) = save_record(home, &rec) {
        eprintln!(
            "[围栏] 容器 profile 台账落盘失败（影响收尾的精确回收）：{}",
            e
        );
    }
}

/// 目的：本程序建过的容器 profile 名（Windows 把包目录名转小写，按前缀不区分大小写筛）。
/// 约束：profile 可能来自没有台账的路径（探针、夹具的台账被删、进程被杀），所以按名字前缀扫。
pub(crate) fn our_profile_names() -> Result<Vec<String>, String> {
    let root = match std::env::var_os("LOCALAPPDATA") {
        Some(v) => PathBuf::from(v).join("Packages"),
        None => return Err("取不到 LOCALAPPDATA（容器 profile 的存储根）".to_string()),
    };
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        // 没有 Packages 目录 = 本机没有容器 profile。
        Err(_) => return Ok(Vec::new()),
    };
    Ok(entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_our_profile(n))
        .collect())
}

/// 目的：这个具名 profile 还在不在（列清单时如实报"挂着的还在不在"）。
pub(crate) fn profile_exists(name: &str) -> bool {
    our_profile_names()
        .map(|names| names.iter().any(|n| n.eq_ignore_ascii_case(name)))
        .unwrap_or(false)
}

/// 目的：扫掉本程序建过的整族容器 profile。
/// 约束：台账只记“我们知道写过什么”，而 profile 可能来自没有台账的路径，所以按独有的名字前缀扫；
///   DeleteAppContainerProfile 连该容器的存储一起删；返回扫掉的个数。
pub fn sweep_profiles() -> Result<usize, String> {
    let mut deleted = 0usize;
    for name in our_profile_names()? {
        if delete_profile(&name) {
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// 目的：删掉一个具名的容器 profile（连该容器的存储一起删）——整族清扫与测试的定向清理共用。
pub(crate) fn delete_profile(name: &str) -> bool {
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    (unsafe { DeleteAppContainerProfile(wide.as_ptr()) }) >= 0
}

/// 目的：孤儿授权清扫——在产品根内找台账之外、我们写过的显式 ACE 并连树撤掉。
/// 约束：台账是精确回收的依据，但账会断（夹具/临时 home 被删、进程被杀、写入后没来得及记账），
///   断了账不代表没有残留。反向兜底两条：本程序建过的容器 profile 名派生 SID；以及任何显式、
///   非继承的允许 ACE，其 SID 形如 S-1-15-2-*（排除 ALL APPLICATION PACKAGES 与 ALL RESTRICTED
///   两个基线）或是本产品的能力 SID（S-1-15-3-*，只认自己派生的那一个）——profile 已删或从没建过
///   的残留也能回收。自产品根向下在**最上层**命中处连树撤掉
///   （授权只会以某个目录为根整树写下去）。只扫产品根内：根外落点（解释器目录、根外只读根）仍
///   只由台账管。**必须在 sweep_profiles 之前调用**：profile 删了就派生不出 SID 了。
pub fn sweep_orphan_aces(root: &Path) -> Result<usize, String> {
    let mut sids: Vec<PSID> = Vec::new();
    for name in our_profile_names()? {
        match container_sid(&name) {
            Ok(s) => sids.push(s),
            Err(e) => eprintln!("[围栏] {}：该容器的孤儿扫描跳过", e),
        }
    }
    // 一个 profile 都派生不出 SID 时也要扫：台账外的残留可能来自根本没建过 profile 的路径。
    let mut swept = 0usize;
    let mut errors: Vec<String> = Vec::new();
    sweep_dir_down(root, &sids, &mut swept, &mut errors);
    for s in &sids {
        free_sid(*s);
    }
    if errors.is_empty() {
        Ok(swept)
    } else {
        Err(errors.join("；"))
    }
}

/// 自上而下扫一个目录：最上层命中就整树撤、不再下钻；没命中才继续走子目录。
/// 约束：读不到 DACL 或撤权失败都记进 errors 并继续扫别处，最后如实返回 Err（清不掉不能当清干净）。
fn sweep_dir_down(dir: &Path, sids: &[PSID], swept: &mut usize, errors: &mut Vec<String>) {
    let known = sids.iter().any(|s| has_any_ace_for(*s, dir));
    let orphans = match orphan_package_aces(dir) {
        Ok(list) => list,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    if known || !orphans.is_empty() {
        for s in sids {
            if let Err(e) = revoke_one(*s, dir, true) {
                errors.push(format!("孤儿撤权未完成（{}）：{}", dir.display(), e));
            }
        }
        for text in orphans {
            match sid_from_string(&text) {
                Ok(sid) => {
                    if let Err(e) = revoke_one(sid, dir, true) {
                        errors.push(format!(
                            "孤儿撤权未完成（{}，{}）：{}",
                            dir.display(),
                            text,
                            e
                        ));
                    }
                    free_sid(sid);
                }
                Err(e) => errors.push(e),
            }
        }
        *swept += 1;
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // 读不了的目录如实跳过：里面就算有残留也不再往下猜。
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_dir() {
            continue;
        }
        // 重解析点（联接/符号链接）不跟：不走出产品根，也不进环。
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            continue;
        }
        sweep_dir_down(&entry.path(), sids, swept, errors);
    }
}
