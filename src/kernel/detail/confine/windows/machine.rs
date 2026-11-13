//! 目的：机器级一次性授权——把「每次新会话都要重写」的祖先链只读属性与可信基座只读授权一次授给
//!   所有应用程序包（ALL APPLICATION PACKAGES），后续围栏准备就只剩 O(深度) 的 ACE 读。
//! 管：落点清单（祖先链 / 产品根 / .tools / runtimes）、只读判定、逐条授权与分步计时、失败如实上报，
//!   以及外部工作区的显式一次性只读授权/撤权（ensure_workspace_grant / revoke_machine_grants）。
//! 不管：每会话的容器数据授权（windows.rs 的 prepare_fence）、ACE 读写（acl.rs）、台账（record/）。
//! 联动：entry::setup::fence 与启动首次授权共用 machine_ensure；状态查询与 --setup=status 共用 machine_missing。

use super::*;
use std::time::Instant;

/// 目的：一个机器级落点：路径 + 权限位 + 递归/继承 + 给用户看的这一环叫什么。
/// 约束：形状与 GrantTarget 对齐；RIGHTS_STAT 只授读属性（容器判存在性），RIGHTS_RO 递归只读（工具链与运行包）。
pub(crate) struct MachineStep {
    pub(crate) label: &'static str,
    pub(crate) path: PathBuf,
    pub(crate) rights: u32,
    pub(crate) recursive: bool,
    pub(crate) inherit: bool,
}

/// 目的：机器级授权的全部落点。
///   产品根：读属性 + (OI)(CI) 继承——根下已有与将来新建的目录/文件都自动带这条 ACE，
///     于是每会话的祖先链、模块父目录等 STAT 检查全部命中，不再写 DACL。
///   可信基座（.tools / runtimes）：递归只读，授给共享包组；新装进这两个目录的文件也继承。
/// 约束：产品根之外的祖先链一级都不写（视图盘把产品根映射成盘符，容器只认映射根这一级）；
///   盘根更不写（写盘根 = 整盘继承重算，真机灾难）。
pub(crate) fn machine_plan(root: &Path) -> Vec<MachineStep> {
    let mut out = Vec::new();
    out.push(MachineStep {
        label: "产品根读属性（继承）",
        path: root.to_path_buf(),
        rights: RIGHTS_STAT,
        recursive: false,
        inherit: true,
    });
    for (label, dir) in [
        ("工具链只读（递归）", root.join(".tools")),
        ("运行包只读（递归）", root.join("runtimes")),
    ] {
        // 目录不存在就跳过：装好之后下一次 ensure 会补上（重复执行只写缺的那几条）。
        if dir.is_dir() {
            out.push(MachineStep {
                label,
                path: dir,
                rights: RIGHTS_RO,
                recursive: true,
                inherit: true,
            });
        }
    }
    out
}

/// 目的：这一条落点是否已足够。
/// 约束：要继承的落点（产品根 / 可信基座）必须已有**可继承**的 ACE——每会话祖先链留下的不可继承
///   STAT ACE 不算数，否则根下的新目录继承不到，per-session 仍要逐条写（PR3 的 O(1) 就不成立）。
fn step_ready(base: PSID, step: &MachineStep) -> bool {
    if step.inherit {
        has_inheritable_ace_for(base, &step.path, step.rights)
    } else {
        has_ace_for(base, &step.path, step.rights)
    }
}

/// 目的：还有哪些机器级落点没授上（空 = 已就绪）。只读：每条一次 DACL 读，O(深度 + 2)。
/// 返回：缺失落点的人话描述（给 --setup=status 与启动对账用）。
pub(crate) fn machine_missing(root: &Path) -> Result<Vec<String>, String> {
    let base = baseline_sid()?;
    let mut missing = Vec::new();
    for step in machine_plan(root) {
        if !step.path.exists() {
            continue;
        }
        if !step_ready(base, &step) {
            missing.push(format!("{}（{}）", step.label, step.path.display()));
        }
    }
    free_sid(base);
    Ok(missing)
}

/// 目的：把机器级落点一次性授好（幂等：已够用的只报「已就绪」，不重复改 DACL）。
/// 参数：out = 每条落点一行进度（含耗时与是否真写了 ACL）。
/// 错误：任一条授权失败时返回全部失败原因（其余落点继续尝试；台账保证可回滚/可重跑）。
pub(crate) fn machine_ensure(
    root: &Path,
    home: &Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if root.parent().is_none() {
        return Err("产品根是盘卷根：拒绝在盘根写继承 ACE（改用产品目录作根）".to_string());
    }
    // 台账锁只罩「读—改—写」事务（见 record/store.rs 的 with_ledger）：逐条授权各自开事务，
    // 不把整段 ensure 罩在一把锁里，否则并发首启会在慢 ACL 写期间互相等到 60 秒锁上限。
    let base = baseline_sid()?;
    let base_text = sid_to_string(base);
    let mut errors: Vec<String> = Vec::new();
    let mut written = 0usize;
    let plan = machine_plan(root);
    for step in &plan {
        let label = step.label;
        let shown = step.path.display().to_string();
        if !step.path.exists() {
            out(&format!(
                "{}：{} 不存在，跳过（装好后重跑即可）",
                label, shown
            ));
            continue;
        }
        let started = Instant::now();
        if step_ready(base, step) {
            out(&format!(
                "{}：{} 已就绪（{} ms）",
                label,
                shown,
                started.elapsed().as_millis()
            ));
            continue;
        }
        let target = GrantTarget {
            path: step.path.clone(),
            rights: step.rights,
            recursive: step.recursive,
            inherit: step.inherit,
            part: FencePart::Parent,
        };
        match grant_machine_journaled(home, base, &base_text, &target, root) {
            Ok(true) => {
                written += 1;
                out(&format!(
                    "{}：{} 已写入（{} ms）",
                    label,
                    shown,
                    started.elapsed().as_millis()
                ));
            }
            Ok(false) => out(&format!(
                "{}：{} 已就绪（{} ms）",
                label,
                shown,
                started.elapsed().as_millis()
            )),
            Err(e) => {
                out(&format!(
                    "{}：{} 失败（{} ms）：{}",
                    label,
                    shown,
                    started.elapsed().as_millis(),
                    e
                ));
                errors.push(format!("{}（{}）：{}", label, shown, e));
            }
        }
    }
    free_sid(base);
    out(&format!(
        "机器级授权：本次新写 {} 条，共 {} 个落点",
        written,
        plan.len()
    ));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

// 契约出口：prep.rs 的 ensure_workspace_grant 转发它；装配层接上调用点之前显式放行。
#[allow(dead_code)]
/// 目的：给一个外部工作区授**机器级一次性只读**（外部工作区绑定等显式动作先付一次成本）。
///   授给基线组、递归、可继承——之后每会话的围栏准备只剩一次 DACL 读，不再重付整树成本。
/// 参数：home = 产品私有区（台账）；root = 工作区目录。
/// 返回：本次是否真写了 ACL（written / reused）与动作耗时。
/// 错误：盘卷根、目录不存在、台账或 ACL 写失败都如实返回（台账先落、失败不静默）。
pub(crate) fn ensure_workspace_grant(
    home: &Path,
    root: &Path,
) -> Result<MachineGrantReport, String> {
    if root.parent().is_none() {
        return Err("工作区是盘卷根：拒绝对盘根写继承 ACE（换一个具体目录）".to_string());
    }
    if !root.is_dir() {
        return Err(format!("工作区目录不存在：{}", root.display()));
    }
    let base = baseline_sid()?;
    let base_text = sid_to_string(base);
    let started = Instant::now();
    let target = GrantTarget {
        path: root.to_path_buf(),
        rights: RIGHTS_RO,
        recursive: true,
        inherit: true,
        part: FencePart::AuthorizedRead,
    };
    let product = product_root(home);
    let result = grant_machine_journaled(home, base, &base_text, &target, &product);
    free_sid(base);
    match result {
        Ok(true) => Ok(MachineGrantReport {
            written: 1,
            reused: 0,
            ms: started.elapsed().as_millis() as u64,
        }),
        Ok(false) => Ok(MachineGrantReport {
            written: 0,
            reused: 1,
            ms: started.elapsed().as_millis() as u64,
        }),
        Err(e) => Err(e),
    }
}

// 契约出口：prep.rs 的 revoke_machine_grants 转发它；装配层接上调用点之前显式放行。
#[allow(dead_code)]
/// 目的：撤掉某个工作区的**机器级只读**授权：盘上 ACE（含子对象继承副本）+ 台账条目一起销。
///   显式动作可以慢：递归撤权 O(子树)，先如实提示再动手。
/// 返回：销掉的台账条目数；台账里没有这个路径的机器级条目就直接 0——不从父目录继承来的 ACE 不在本命令职责里。
/// 错误：路径不在了、ACE 撤不掉、台账更新失败都如实返回；台账落盘失败时文件保持原样，重跑可再撤。
pub(crate) fn revoke_machine_grants(home: &Path, path: &Path) -> Result<usize, String> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err(format!("这个路径现在不在了：{}", path.display()));
    }
    // 先看台账：单参撤权只撤我们发过的那条机器级授权，别去碰从父目录继承来的 ACE
    //（那些不是本程序的授权，递归撤它们既慢又会改掉用户目录原有的语义）。
    let recorded = {
        let _lock = lock_ledger(home)?;
        load_record(home)
            .grants
            .iter()
            .any(|g| g.scope == GrantScope::Machine && same_path(&g.path, path))
    };
    if !recorded {
        return Ok(0);
    }
    let base = baseline_sid()?;
    if has_any_ace_for(base, path) {
        eprintln!(
            "[围栏] 正在撤销机器级只读授权（递归子树，可能要几十秒）：{}",
            path.display()
        );
        if let Err(e) = revoke_one(base, path, true) {
            free_sid(base);
            return Err(format!("撤销未完成（{}）：{}", path.display(), e));
        }
    }
    free_sid(base);
    // 台账只删「机器级 + 同一路径」的条目：路径逐分量比、大小写不敏感（台账里存的是历史字符串）。
    let _lock = lock_ledger(home)?;
    let mut rec = load_record(home);
    let before = rec.grants.len();
    rec.grants
        .retain(|g| !(g.scope == GrantScope::Machine && same_path(&g.path, path)));
    let removed = before - rec.grants.len();
    if removed == 0 {
        return Ok(0);
    }
    // 该路径上再没有任何授权引用时，快照也一并销掉（没有可管理的 ACE 了）。
    let still_granted = rec.grants.iter().any(|g| same_path(&g.path, path));
    if !still_granted {
        rec.snapshots.retain(|s| !same_path(&s.path, path));
    }
    if let Err(e) = save_record(home, &rec) {
        return Err(format!("已撤掉 ACE，但台账更新失败：{}", e));
    }
    rec.drop_if_empty(home);
    Ok(removed)
}

/// 目的：台账里记的路径字符串与目标路径是不是同一个（能规范化就按规范化路径比，逐分量、大小写不敏感）。
#[allow(dead_code)]
fn same_path(recorded: &str, target: &Path) -> bool {
    let canon = |p: &Path| {
        std::fs::canonicalize(p)
            .map(|c| crate::kernel::domain::path::native(&c))
            .ok()
    };
    let (left, right) = match (canon(Path::new(recorded)), canon(target)) {
        (Some(a), Some(b)) => (a, b),
        _ => (PathBuf::from(recorded), target.to_path_buf()),
    };
    let mut left = left.components();
    let mut right = right.components();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) => {
                if !a
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：落点清单只覆盖产品根继承 + 可信基座；产品根之外的祖先链一级都不写（视图盘负责）。
    #[test]
    fn plan_covers_root_and_trusted_bases_only() {
        let base = std::env::temp_dir().join("solomni-machine-plan");
        let root = base.join("product");
        let plan = machine_plan(&root);
        let root_step = plan.iter().find(|s| s.path == root).expect("有产品根落点");
        assert_eq!(root_step.rights, RIGHTS_STAT);
        assert!(root_step.inherit && !root_step.recursive);
        // 根外祖先不出现：root 的上级目录（含 base）一条都不在清单里。
        assert!(
            plan.iter()
                .all(|s| s.path == root || s.path.starts_with(&root))
        );
        assert!(!plan.iter().any(|s| s.path == base));
    }
}
