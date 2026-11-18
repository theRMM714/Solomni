//! 目的：写完授权后的核对与在场判断——原有权限项不得丢、我们的 ACE 必须生效，失败就还原。
//! 管：grant_verified / restore_after_failed_write / has_matching_ace 与 has_* 家族 / inheritable_flags_ok。
//! 不管：怎么读 DACL（scan.rs）、怎么写入（grant.rs）、哪些落点要写（target.rs）。
//! 联动：grant_verified 是 grant_one 的上层入口；windows/record 的撤权核对用 has_explicit_any_ace_for。

use super::super::*;
use super::ace::ACCESS_ALLOWED_ACE_TYPE;
use super::scan::{is_sid_text, permission_entries};
use std::path::Path;
use windows_sys::Win32::Security::{CONTAINER_INHERIT_ACE, OBJECT_INHERIT_ACE, PSID};

// 测试注入：强制 grant_verified 在写之前失败，覆盖“写都没成功”的失败路径。
// 约束：线程局部，避免与并行跑的真机探针互相踩。
#[cfg(test)]
thread_local! {
    pub(crate) static FORCE_GRANT_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 目的：写一条授权并做写后核对（原有权限项不得丢失、我们的 ACE 必须生效）。
/// 参数：sid 是授权对象，path 是目标，rights 是权限位，recursive 与 inherit 同 grant_one。
/// 返回：写入且核对通过时 Ok(())。
/// 错误：读写 DACL 失败、原有权限项丢失、我们的 ACE 未生效，都如实返回；调用方据此回滚。
pub(crate) fn grant_verified(
    sid: PSID,
    path: &Path,
    rights: u32,
    recursive: bool,
    inherit: bool,
) -> Result<(), String> {
    #[cfg(test)]
    if FORCE_GRANT_FAILURE.with(|f| f.get()) {
        return Err("测试注入：写 DACL 失败（没有写入）".to_string());
    }
    let before_sd = sd_bytes(path).map_err(|e| format!("读取写入前安全描述符失败：{}", e))?;
    let before = acl_scan(path).map_err(|e| format!("读取写入前 DACL 失败：{}", e))?;
    // 写失败时先比一次：真没变就什么都不做；变了（递归写可能已落了一部分）就还原。
    if let Err(e) = grant_one(sid, path, rights, recursive, inherit) {
        let changed = sd_bytes(path).map(|now| now != before_sd).unwrap_or(true);
        return Err(if changed {
            restore_after_failed_write(path, &before_sd, e, recursive)
        } else {
            e
        });
    }
    let after = match acl_scan(path) {
        Ok(after) => after,
        Err(e) => {
            return Err(restore_after_failed_write(
                path,
                &before_sd,
                format!("写后读回 DACL 失败：{}", e),
                recursive,
            ));
        }
    };
    let lost = lost_entries(&permission_entries(&before), &permission_entries(&after));
    if !lost.is_empty() {
        // 认不出的 ACE 进不了比对：有就说清，别让"缺 N 项"看起来像全部账目。
        let unparsed = if after.unparsed > 0 {
            format!("；另有 {} 条认不出的 ACE 未参与比对", after.unparsed)
        } else {
            String::new()
        };
        return Err(restore_after_failed_write(
            path,
            &before_sd,
            format!(
                "写后核对发现原有权限项丢失（{}，缺 {} 项{}）",
                path.display(),
                lost.len(),
                unparsed
            ),
            recursive,
        ));
    }
    if !has_ace_for(sid, path, rights) {
        return Err(restore_after_failed_write(
            path,
            &before_sd,
            format!(
                "写后校验失败（{}）：给定权限的 ACE 未生效，本机环境不允许",
                path.display()
            ),
            recursive,
        ));
    }
    Ok(())
}

/// 目的：写后核对失败时把对象还原到写入前；还原失败如实说明并把台账留给收尾。
fn restore_after_failed_write(
    path: &Path,
    before_sd: &[u8],
    primary: String,
    recursive: bool,
) -> String {
    match restore_sd(path, before_sd) {
        Ok(()) if recursive => format!(
            "{}（根对象已还原；递归子项若有残留，台账保留，可用 --fence-clean 收尾）",
            primary
        ),
        Ok(()) => format!("{}（已还原到写入前的 DACL）", primary),
        Err(e) => format!(
            "{}；还原失败：{}（台账保留，可用 --fence-clean 收尾）",
            primary, e
        ),
    }
}

/// 目的：该对象上有没有给这个 SID 的**任何**允许 ACE（不看权限位）。
///   用途：残留检查——撤权后哪怕只留一位（真机残留过一条只有 SYNCHRONIZE 的 (OI)(CI) ACE，
///   整棵子树因此对受限进程不可读）也算没撤干净；
///   has_ace_for 回答"够不够用"，这条回答"在不在场"。
/// 约束：只看标准允许 ACE（对象 ACE 是给子对象的，不属于"本对象上的残留"）。
pub(crate) fn has_any_ace_for(sid: PSID, path: &Path) -> bool {
    let want = sid_to_string(sid);
    if !is_sid_text(&want) {
        return false;
    }
    acl_scan(path)
        .map(|scan| {
            scan.entries.iter().any(|v| {
                v.ace_type == ACCESS_ALLOWED_ACE_TYPE
                    && v.flags & INHERIT_ONLY_ACE == 0
                    && v.sid == want
            })
        })
        .unwrap_or(false)
}

/// 目的：该对象上是不是**自己带着**给这个 SID 的允许 ACE（不含从祖先继承来的），不看权限位。
///   用途：撤权与退役的残留核对——继承来的那条属于祖先的授权，不是本对象撤得掉的那条
///   （产品根有机器级 `(OI)(CI)` 授权时，根下每个目录都继承到一条同主体的投影）；
///   不看权限位是因为真机残留过只有 SYNCHRONIZE 的 ACE，那也算没撤干净。
/// 约束：只看标准允许 ACE（对象 ACE 是给子对象的，不属于"本对象上的残留"）。
pub(crate) fn has_explicit_any_ace_for(sid: PSID, path: &Path) -> bool {
    let want = sid_to_string(sid);
    if !is_sid_text(&want) {
        return false;
    }
    acl_scan(path)
        .map(|scan| {
            scan.entries.iter().any(|v| {
                v.ace_type == ACCESS_ALLOWED_ACE_TYPE
                    && v.flags & INHERIT_ONLY_ACE == 0
                    && v.flags & INHERITED_ACE == 0
                    && v.sid == want
            })
        })
        .unwrap_or(false)
}

/// 目的：该对象上是不是已经有给这个 SID 的允许 ACE，**且权限位覆盖得住**。
///   用途：基线授权只以递归方式写过一次，所以根上已有"够用"的 ACE 就跳过整棵树——否则每来一个 agent 都要重走几万文件。
///   只看"有没有该 SID 的 ACE"不够：解释器目录会继承只有 SYNCHRONIZE 的 ALL APPLICATION PACKAGES ACE，
///   基线因此被整条跳过，容器里连解释器都读不到（工具报 python is not recognized）。
/// 约束：只看标准允许 ACE（对象 ACE 是给子对象的，不构成"本对象上已经够用"）。
pub(crate) fn has_ace_for(sid: PSID, path: &Path, rights: u32) -> bool {
    has_matching_ace(sid, path, rights, true)
}

/// 目的：ACE 匹配的公共判定：标准允许 ACE、不是只对子对象生效、权限位覆盖得住；
///   `inherited_ok=false` 时只认对象自己带的 ACE（从祖先继承来的不算）。
fn has_matching_ace(sid: PSID, path: &Path, rights: u32, inherited_ok: bool) -> bool {
    let want = sid_to_string(sid);
    if !is_sid_text(&want) {
        return false;
    }
    acl_scan(path)
        .map(|scan| {
            scan.entries.iter().any(|v| {
                v.ace_type == ACCESS_ALLOWED_ACE_TYPE
                    && v.flags & INHERIT_ONLY_ACE == 0
                    && (inherited_ok || v.flags & INHERITED_ACE == 0)
                    && rights_covered(v.mask, rights)
                    && v.sid == want
            })
        })
        .unwrap_or(false)
}

/// 目的：该对象上是不是**自己带着**给这个 SID 的允许 ACE（不含从祖先继承来的），且权限位覆盖得住。
///   用途：验证「我们写过 / 我们撤掉了」这类归属判断——继承来的 ACE 属于祖先那条授权，不是本对象可撤的那条；
///   产品根一旦有机器级 `(OI)(CI)` 授权，子目录的继承 ACE 会让「已撤干净」的断言假失败。
#[cfg(test)]
pub(crate) fn has_explicit_ace_for(sid: PSID, path: &Path, rights: u32) -> bool {
    has_matching_ace(sid, path, rights, false)
}

/// 目的：该对象上是不是已经有给这个 SID、**会向下继承**的允许 ACE，且权限位覆盖得住。
///   用途：机器级的「根上写一次 (OI)(CI)」这一步——每会话祖先链写的那条不可继承 STAT ACE 不算数，
///   它不会传给根下的子目录，子目录仍需逐条写；只有带 (OI)/(CI) 的那条才让后续检查全部命中。
/// 约束：只看标准允许 ACE（对象 ACE 是给子对象的，不属于本对象上的可继承授权）。
pub(crate) fn has_inheritable_ace_for(sid: PSID, path: &Path, rights: u32) -> bool {
    let want = sid_to_string(sid);
    if !is_sid_text(&want) {
        return false;
    }
    acl_scan(path)
        .map(|scan| {
            scan.entries.iter().any(|v| {
                v.ace_type == ACCESS_ALLOWED_ACE_TYPE
                    && inheritable_flags_ok(v.flags)
                    && rights_covered(v.mask, rights)
                    && v.sid == want
            })
        })
        .unwrap_or(false)
}

/// 目的：一条 ACE 的继承位是否足够覆盖「根下既有与新建子项」：必须同时带 (OI) 与 (CI)，且不是只对子对象生效。
///   写侧（`write_ace` 的 inherit 分支）总是两条都设；只带一条的旧 ACE 不算就绪，否则 O(1) 会静默不成立。
pub(super) fn inheritable_flags_ok(flags: u8) -> bool {
    let both = (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8;
    flags & INHERIT_ONLY_ACE == 0 && flags & both == both
}
