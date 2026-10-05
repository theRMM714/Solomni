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
    /// 我们创建过的容器 profile 名（清理时按名删除）。
    #[serde(default)]
    profiles: BTreeSet<String>,
    /// (SID, 路径, 权限位) —— 逐条对应写下去的 ACE。
    #[serde(default)]
    grants: Vec<(String, String, u32)>,
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

/// 孤儿授权清扫：按容器 SID 族在产品根内找我们写过的显式 ACE 并连树撤掉。
/// 台账是精确回收的依据，但账会断（夹具/临时 home 被删、进程被杀、旧版本没记账）——
/// 断了账不代表没有残留（fence.leftover-grant-hides-parent：`tests/` 上留过一条旧版
/// 授出去的显式 ACE，整棵子树因此在受限进程里不可读）。这里反向兜底：本程序建过的
/// 容器 profile 名 → 派生 SID → 自产品根向下找带该 SID 显式 ACE 的目录，在**最上层**
/// 命中处连树撤掉（授权只会以某个目录为根整树写下去，树下同名 SID 的 ACE 都是它的
/// 传播产物）。只扫产品根内：根外落点（解释器目录、根外只读根）仍只由台账管。
/// **必须在 `sweep_profiles` 之前调用**：profile 删了就派生不出 SID 了。
pub fn sweep_orphan_aces(root: &Path) -> Result<usize, String> {
    let mut sids: Vec<PSID> = Vec::new();
    for name in our_profile_names()? {
        match container_sid(&name) {
            Ok(s) => sids.push(s),
            Err(e) => eprintln!("[围栏] {}：该容器的孤儿扫描跳过", e),
        }
    }
    if sids.is_empty() {
        return Ok(0);
    }
    let mut swept = 0usize;
    sweep_dir_down(root, &sids, &mut swept);
    for s in &sids {
        free_sid(*s);
    }
    Ok(swept)
}

/// 自上而下扫一个目录：最上层命中就整树撤、不再下钻；没命中才继续走子目录。
fn sweep_dir_down(dir: &Path, sids: &[PSID], swept: &mut usize) {
    if sids.iter().any(|s| has_any_ace_for(*s, dir)) {
        for s in sids {
            if let Err(e) = revoke_one(*s, dir, true) {
                eprintln!("[围栏] 孤儿撤权未完成（{}）：{}", dir.display(), e);
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
        sweep_dir_down(&entry.path(), sids, swept);
    }
}

pub(crate) fn record_grants(
    home: &Path,
    container: &str,
    written: &[(String, PathBuf, u32)],
) -> Result<(), String> {
    let mut rec = load_record(home);
    rec.profiles.insert(container.to_string());
    for (sid, path, rights) in written {
        let entry = (sid.clone(), path.to_string_lossy().into_owned(), *rights);
        if !rec.grants.contains(&entry) {
            rec.grants.push(entry);
        }
    }
    save_record(home, &rec)
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
    if rec.grants.is_empty() && rec.profiles.is_empty() {
        return Ok("没有台账：本程序没在本机写过权限项".to_string());
    }
    let mut removed = 0usize;
    for (sid_text, path, _rights) in &rec.grants {
        let sid = match sid_from_string(sid_text) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[围栏] {}", e);
                continue;
            }
        };
        let p = PathBuf::from(path);
        if p.exists() {
            match revoke_one(sid, &p, false) {
                Ok(()) => removed += 1,
                Err(e) => eprintln!("[围栏] 撤销未完成：{}", e),
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
    let _ = std::fs::remove_file(record_path(home));
    Ok(format!(
        "已撤销 {} 条授权、删除 {} 个容器 profile",
        removed, deleted
    ))
}

/// 撤销一次会话的授权（会话删除时经 FenceHost 端口调用）。
/// home 为 None 时只撤权限、不动台账（无台的调用方少见；正常路径都给 home）。
pub fn release_fence_home(spec: &FenceSpec, home: Option<&Path>) -> Result<(), String> {
    let sid = container_sid(&container_name(spec))?;
    let mut result = Ok(());
    // 撤权要覆盖**同一次授权写下的全部条目**：叶子（读写根 / 只读根 / 工作目录）**与它们的父目录**。
    // 落点清单与 prepare_fence 共用 grant_targets——两处各写一份迟早会漏掉某一类。
    let mut paths: Vec<PathBuf> = grant_targets(spec)
        .into_iter()
        .map(|(p, _, _, _)| p)
        .collect();
    paths.sort();
    paths.dedup();
    // 台账比对用字符串：下面 paths 会被消费掉。
    let path_texts: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    for p in paths {
        if p.as_os_str().is_empty() {
            continue;
        }
        if let Err(e) = revoke_one(sid, &p, true) {
            eprintln!("[围栏] 撤销未完成：{}", e);
            if result.is_ok() {
                result = Err(e);
            }
        }
    }
    let sid_text = sid_to_string(sid);
    free_sid(sid);
    if let Some(h) = home {
        // 台账跟着会话一起清：撤掉的条目不留残账（账目等于"本机现在还有我们写的哪些权限"）。
        let mut rec = load_record(h);
        rec.grants
            .retain(|(s, p, _)| s != &sid_text || !path_texts.iter().any(|x| x == p));
        if let Err(e) = save_record(h, &rec) {
            eprintln!("[围栏] 台账更新失败：{}", e);
        }
    }
    result
}

/// 兼容旧调用点：不带台账的撤销。
pub fn release_fence(spec: &FenceSpec) -> Result<(), String> {
    release_fence_home(spec, None)
}
