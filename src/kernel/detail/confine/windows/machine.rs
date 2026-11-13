//! 目的：机器级一次性授权——把「每次新会话都要重写」的祖先链只读属性与可信基座只读授权一次授给
//!   所有应用程序包（ALL APPLICATION PACKAGES），后续围栏准备就只剩 O(深度) 的 ACE 读。
//! 管：落点清单（祖先链 / 产品根 / .tools / runtimes）、只读判定、逐条授权与分步计时、失败如实上报。
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
///   祖先链（产品根之上）：只授读属性、不递归不继承——每会话的 deep 祖先链会查它，授一次就够。
///   产品根：读属性 + (OI)(CI) 继承——根下已有与将来新建的目录/文件都自动带这条 ACE，
///     于是每会话的祖先链、模块父目录等 STAT 检查全部命中，不再写 DACL。
///   可信基座（.tools / runtimes）：递归只读，授给共享包组；新装进这两个目录的文件也继承。
/// 约束：盘根不写（写盘根 = 整盘继承重算，真机灾难）。
pub(crate) fn machine_plan(root: &Path) -> Vec<MachineStep> {
    let mut out = Vec::new();
    let mut parent = root.parent();
    while let Some(anc) = parent {
        // 盘卷根不写：那是整盘继承重算，且默认就对容器可读。
        if anc.parent().is_none() {
            break;
        }
        out.push(MachineStep {
            label: "根祖先读属性",
            path: anc.to_path_buf(),
            rights: RIGHTS_STAT,
            recursive: false,
            inherit: false,
        });
        parent = anc.parent();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：落点清单覆盖祖先链（不写盘根）+ 产品根继承 + 可信基座；形状与 prepare_fence 的消费方式一致。
    #[test]
    fn plan_covers_ancestors_root_and_trusted_bases() {
        let base = std::env::temp_dir().join("solomni-machine-plan");
        let root = base.join("product");
        let plan = machine_plan(&root);
        let root_step = plan.iter().find(|s| s.path == root).expect("有产品根落点");
        assert_eq!(root_step.rights, RIGHTS_STAT);
        assert!(root_step.inherit && !root_step.recursive);
        // 祖先链至少包含 base（root 的父目录），且都不在盘卷根上。
        assert!(
            plan.iter()
                .any(|s| s.path == base && s.rights == RIGHTS_STAT)
        );
        assert!(plan.iter().all(|s| s.path.parent().is_some()));
    }
}
