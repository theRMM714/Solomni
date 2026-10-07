//! **授权记录与撤销**：ACL 改动**在进程退出后仍留在盘上**，所以要把"给哪个容器授了哪些路径"落一份记录，
//! 下次启动按记录撤销、并清扫残留的 AppContainer 档案——Windows 独有的一整类工作。

use crate::capabilities::tools::api::FenceSpec;
use std::collections::BTreeSet;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile;
use windows_sys::Win32::Security::PSID;

/// 文件对象（SetNamedSecurityInfoW / GetNamedSecurityInfoW 的对象类型）。
use super::*;
/// 授权台账：记下"我们给谁、在哪些路径上写了权限"，`--fence-clean` 按它精确回收。
/// 位置：产品私有区 `.home/fence-grants.json`（数据不出工作区）。
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct GrantRecord {
    /// 目的：我们创建过的容器 profile 名（清理时按名删除）。
    #[serde(default)]
    pub(crate) profiles: BTreeSet<String>,
    /// 目的：根外路径的授权摘要（SID, 路径, 权限位）；收尾按它精确撤销。
    #[serde(default)]
    pub(crate) grants: Vec<(String, String, u32)>,
    /// 目的：产品根内路径的原始安全描述符（路径, self-relative 字节）；收尾整体还原。
    #[serde(default)]
    pub(crate) snapshots: Vec<(String, Vec<u8>)>,
}

pub(crate) fn record_path(home: &Path) -> PathBuf {
    home.join("fence-grants.json")
}

pub(crate) fn load_record(home: &Path) -> GrantRecord {
    std::fs::read_to_string(record_path(home))
        .ok()
        .and_then(|t| serde_json::from_str::<GrantRecord>(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn save_record(home: &Path, rec: &GrantRecord) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| format!("建私有区失败：{}", e))?;
    let text = serde_json::to_string_pretty(rec).map_err(|e| e.to_string())?;
    std::fs::write(record_path(home), text).map_err(|e| e.to_string())
}

/// 记下"我们建过这个容器 profile"（与 `prepare_fence` 共用同一份台账），供 `--fence-clean` 精确回收。
pub(crate) fn record_profile(home: &Path, name: &str) {
    let mut rec = load_record(home);
    if rec.profiles.insert(name.to_string()) {
        if let Err(e) = save_record(home, &rec) {
            eprintln!(
                "[围栏] 容器 profile 台账落盘失败（影响 --fence-clean 的精确回收）：{}",
                e
            );
        }
    }
}

/// 本程序建过的容器 profile 名（Windows 把包目录名转小写，按前缀不区分大小写筛）。
/// profile 可能来自没有台账的路径（探针、夹具的台账被删、旧版本），所以按名字前缀扫。
fn our_profile_names() -> Result<Vec<String>, String> {
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

/// 扫掉本程序建过的整族容器 profile：台账只记"我们知道写过什么"，而 profile 可能来自没有台账的路径。
/// 名字前缀是本程序独有的，所以按它扫；`DeleteAppContainerProfile` 连该容器的存储一起删。返回扫掉的个数。
pub fn sweep_profiles() -> Result<usize, String> {
    let mut deleted = 0usize;
    for name in our_profile_names()? {
        if delete_profile(&name) {
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// 删掉一个具名的容器 profile（连该容器的存储一起删）。整族清扫与测试的定向清理共用。
pub(crate) fn delete_profile(name: &str) -> bool {
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    (unsafe { DeleteAppContainerProfile(wide.as_ptr()) }) >= 0
}

/// 孤儿授权清扫：在产品根内找台账之外、我们写过的显式 ACE 并连树撤掉。
/// 台账是精确回收的依据，但账会断（夹具/临时 home 被删、进程被杀、旧版本没记账）——
/// 断了账不代表没有残留（tests/ 上留过一条旧版授出去的显式 ACE，整棵子树因此对受限进程不可读）。
/// 反向兜底两条：本程序建过的容器 profile 名派生 SID；以及任何显式、非继承、SID 形如
/// S-1-15-2-* 的允许 ACE（排除 ALL APPLICATION PACKAGES 与 ALL RESTRICTED 两个基线）——
/// profile 已删或从没建过的残留也能回收。自产品根向下在**最上层**命中处连树撤掉（授权只会
/// 以某个目录为根整树写下去，树下同名 SID 的 ACE 都是它的传播产物）。只扫产品根内：根外落点
/// （解释器目录、根外只读根）仍只由台账管。
/// **必须在 sweep_profiles 之前调用**：profile 删了就派生不出 SID 了。
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
/// 读不到 DACL 或撤权失败都记进 errors 并继续扫别处，最后如实返回 Err（清不掉不能当清干净）。
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

/// 目的：把容器 profile 名先记进台账落盘（守门进程真正建 profile 之前，名字先有主）。
/// 错误：台账落盘失败时返回原因。
pub(crate) fn journal_add_profile(
    home: &Path,
    rec: &mut GrantRecord,
    name: &str,
) -> Result<(), String> {
    if !rec.profiles.insert(name.to_string()) {
        return Ok(());
    }
    if let Err(e) = save_record(home, rec) {
        rec.profiles.remove(name);
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(())
}

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
            .map(|c| super::super::strip_verbatim_prefix(&c))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    let path_owned = canon(path);
    let root_owned = canon(root);
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

/// 目的：把一条根外授权先写进台账落盘；返回本次是否新增了该条目。
/// 错误：台账落盘失败时返回原因（落盘失败就不得动 ACL）。
pub(crate) fn journal_add_grant(
    home: &Path,
    rec: &mut GrantRecord,
    sid: &str,
    path: &Path,
    rights: u32,
) -> Result<bool, String> {
    let entry = (sid.to_string(), path.to_string_lossy().into_owned(), rights);
    if rec.grants.contains(&entry) {
        return Ok(false);
    }
    rec.grants.push(entry);
    if let Err(e) = save_record(home, rec) {
        rec.grants.pop();
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(true)
}

/// 目的：把一条根内路径的原始安全描述符先写进台账落盘；返回本次是否新增了该路径的快照。
/// 错误：台账落盘失败时返回原因（落盘失败就不得动 ACL）。
pub(crate) fn journal_add_snapshot(
    home: &Path,
    rec: &mut GrantRecord,
    path: &Path,
    bytes: Vec<u8>,
) -> Result<bool, String> {
    let key = path.to_string_lossy().into_owned();
    if rec.snapshots.iter().any(|(p, _)| p == &key) {
        return Ok(false);
    }
    rec.snapshots.push((key, bytes));
    if let Err(e) = save_record(home, rec) {
        rec.snapshots.pop();
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(true)
}

/// 目的：按“先落台账、再写 ACL、写后核对、失败回滚”完成一条授权。
/// 参数：rec 是本次准备的内存台账；sid_text 是 SID 字符串；target 是落点；root 是产品根。
/// 错误：台账落盘、写后核对或回滚失败时返回原因（回滚失败会一并写进错误）。
pub(crate) fn grant_one_journaled(
    home: &Path,
    rec: &mut GrantRecord,
    sid: PSID,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
) -> Result<(), String> {
    let (path, rights, recursive, inherit) = (target.0.as_path(), target.1, target.2, target.3);
    let key = path.to_string_lossy().into_owned();
    if inside_root(path, root) {
        let bytes = sd_bytes(path).map_err(|e| format!("读原始安全描述符失败：{}", e))?;
        let added = journal_add_snapshot(home, rec, path, bytes)?;
        return match grant_verified(sid, path, rights, recursive, inherit) {
            Ok(()) => Ok(()),
            Err(e) => {
                let rollback = rec
                    .snapshots
                    .iter()
                    .find(|(p, _)| p == &key)
                    .map(|(_, b)| restore_sd(path, b))
                    .unwrap_or_else(|| Err("台账里没有该路径的快照".to_string()));
                let journal = if added {
                    rec.snapshots.retain(|(p, _)| p != &key);
                    save_record(home, rec).map_err(|x| format!("台账更新失败：{}", x))
                } else {
                    Ok(())
                };
                Err(combine(e, rollback.err(), journal.err()))
            }
        };
    }
    let added = journal_add_grant(home, rec, sid_text, path, rights)?;
    match grant_verified(sid, path, rights, recursive, inherit) {
        Ok(()) => Ok(()),
        Err(e) => {
            let rollback = revoke_one(sid, path, recursive)
                .map_err(|x| format!("撤销已写入的 ACE 失败：{}", x));
            let journal = if added {
                rec.grants.retain(|(s, p, _)| !(s == sid_text && p == &key));
                save_record(home, rec).map_err(|x| format!("台账更新失败：{}", x))
            } else {
                Ok(())
            };
            Err(combine(e, rollback.err(), journal.err()))
        }
    }
}

/// 目的：把主错误与回滚、台账更新两条次生错误拼成一条如实的失败原因。
fn combine(primary: String, rollback: Option<String>, journal: Option<String>) -> String {
    let mut out = primary;
    if let Some(r) = rollback {
        out.push_str("；回滚失败：");
        out.push_str(&r);
    }
    if let Some(j) = journal {
        out.push_str("；台账更新失败：");
        out.push_str(&j);
    }
    out
}

/// SID → 字符串（写台账用）。
pub(crate) fn sid_to_string(sid: PSID) -> String {
    let mut out: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 || out.is_null() {
        return "(未知 SID)".to_string();
    }
    let mut buf: Vec<u16> = Vec::new();
    unsafe {
        let mut i = 0isize;
        loop {
            let c = *out.offset(i);
            if c == 0 {
                break;
            }
            buf.push(c);
            i += 1;
        }
        LocalFree(out as *mut c_void);
    }
    String::from_utf16_lossy(&buf)
}

/// 字符串 → SID（清理时按台账里的字符串还原）。
pub(crate) fn sid_from_string(text: &str) -> Result<PSID, String> {
    let w: Vec<u16> = std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sid: PSID = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut sid) } == 0 || sid.is_null() {
        return Err(format!("SID 不合法：{}", text));
    }
    Ok(sid)
}

/// 精确回收：按台账把我们写过的 ACE 逐条撤掉，并删掉我们建过的容器 profile。
/// 返回给用户看的一句话（清理了几条、删了几个 profile）。
pub fn clean(home: &Path) -> Result<String, String> {
    let rec = load_record(home);
    if rec.grants.is_empty() && rec.profiles.is_empty() && rec.snapshots.is_empty() {
        return Ok("没有台账：本程序没在本机写过权限项".to_string());
    }
    let mut errors: Vec<String> = Vec::new();
    let mut restored = 0usize;
    // 根内路径整体还原原始安全描述符：被写坏的 DACL 只有这一条路能修回来。
    for (path, bytes) in &rec.snapshots {
        let p = PathBuf::from(path);
        if std::fs::symlink_metadata(&p).is_err() {
            continue;
        }
        match restore_sd(&p, bytes) {
            Ok(()) => restored += 1,
            Err(e) => errors.push(e),
        }
    }
    let mut removed = 0usize;
    for (sid_text, path, _rights) in &rec.grants {
        let sid = match sid_from_string(sid_text) {
            Ok(s) => s,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let p = PathBuf::from(path);
        if std::fs::symlink_metadata(&p).is_ok() {
            match revoke_one(sid, &p, true) {
                Ok(()) => removed += 1,
                Err(e) => errors.push(e),
            }
        }
        free_sid(sid);
    }
    let mut deleted = 0usize;
    for name in &rec.profiles {
        let n: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let hr = unsafe { DeleteAppContainerProfile(n.as_ptr()) };
        if hr >= 0 {
            deleted += 1;
        }
    }
    if !errors.is_empty() {
        // 还有没清掉的：台账保留，供下次重试（删了就等于把待办也删了）。
        return Err(format!(
            "台账回收未完成：{}（已还原 {} 处、已撤销 {} 条、已删 {} 个 profile；台账保留供重试）",
            errors.join("；"),
            restored,
            removed,
            deleted
        ));
    }
    // 全部清干净了才删台账；删不掉本身就是没收干净，如实报错。
    match std::fs::remove_file(record_path(home)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("权限项已清干净，但删除授权台账失败：{}", e)),
    }
    Ok(format!(
        "已还原 {} 处原始权限、撤销 {} 条授权、删除 {} 个容器 profile",
        restored, removed, deleted
    ))
}

/// 目的：撤销一次会话的授权（会话删除时经 FenceHost 端口调用）。
/// 参数：spec 是该会话各席的围栏范围；home 是产品私有区（台账落点，也是产品根的锚）。
/// 错误：撤权、还原或台账更新失败时如实返回；失败条目留在台账里，供下次重试。
pub fn release_fence(spec: &FenceSpec, home: &Path) -> Result<(), String> {
    let sid = container_sid(&container_name(spec))?;
    let sid_text = sid_to_string(sid);
    let mut rec = load_record(home);
    // 没有台账就别凭空造一份：释放只在"确实写过授权"时才动台账文件。
    let had_record = record_path(home).exists();
    // 撤权要覆盖**同一次授权写下的全部条目**：叶子（读写根 / 只读根 / 工作目录）**与它们的父目录**。
    // 落点清单与 prepare_fence 共用 grant_targets——两处各写一份迟早会漏掉某一类。
    let mut paths: Vec<PathBuf> = grant_targets(spec)
        .into_iter()
        .map(|(p, _, _, _)| p)
        .collect();
    // 台账里这个 SID 写过的路径也要覆盖：配置改过后，落点清单可能已经算不出它们。
    for (s, p, _) in &rec.grants {
        if s == &sid_text {
            paths.push(PathBuf::from(p));
        }
    }
    paths.sort();
    paths.dedup();
    let mut errors: Vec<String> = Vec::new();
    for p in &paths {
        if p.as_os_str().is_empty() || std::fs::symlink_metadata(p).is_err() {
            continue;
        }
        let key = p.to_string_lossy().into_owned();
        // 还有别的容器 SID 在同一个落点上时，只精确撤自己那一条；快照留给最后一个释放者整体还原。
        let has_other = rec
            .grants
            .iter()
            .any(|(s, q, _)| s != &sid_text && q == &key);
        let snapshot = rec.snapshots.iter().find(|(q, _)| q == &key).cloned();
        if let (Some((_, bytes)), false) = (snapshot, has_other) {
            match restore_sd(p, &bytes) {
                Ok(()) => {
                    rec.snapshots.retain(|(q, _)| q != &key);
                    rec.grants
                        .retain(|(s, q, _)| !(s == &sid_text && q == &key));
                }
                Err(e) => errors.push(e),
            }
            continue;
        }
        match revoke_one(sid, p, true) {
            Ok(()) => {
                rec.grants
                    .retain(|(s, q, _)| !(s == &sid_text && q == &key));
            }
            Err(e) => errors.push(e),
        }
    }
    free_sid(sid);
    if had_record {
        if let Err(e) = save_record(home, &rec) {
            errors.push(format!("授权台账更新失败：{}", e));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}
