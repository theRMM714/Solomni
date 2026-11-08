//! 目的：基线的纯逻辑——"沙箱里的这份内容出自哪条提交"的显式类型与提交前的按路径校核。
//! 管：基线引用 `BaseRef`、拒绝原因 `CommitRefusal`、`stale_paths` 校核。
//! 不管：基线怎么落盘（在 `detail/fs_workstore` 的 refs 后面）；三方比较与合并（在 `workstore`）。
//! 联动：`service.rs` 的 `work_commit_auto`；`api.rs` 对外重导出。

use std::collections::BTreeMap;
use std::fmt;

/// 目的：一条基线——精确的提交号；"主线现在是什么"不是基线，是要跟它比较的对象。
// 接线前（PR2 写后自动提交）bin 目标里有不到使用者，接上即删这行。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BaseRef {
    pub oid: u64,
}

/// 目的：一条路径的基线与主线当前头不一致。
// 接线前（PR2 写后自动提交）bin 目标里有不到使用者，接上即删这行。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StalePath {
    pub path: String,
    /// 目的：这条路径记录的基线；`None` = 从未拉取过。
    pub base: Option<u64>,
}

/// 目的：自动提交被拒绝的原因（自动路径不降级，只允许拒绝）。
// 接线前（PR2 写后自动提交）bin 目标里有不到使用者，接上即删这行。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitRefusal {
    /// 目的：待提交路径的基线不是主线当前头——先 `work_pull` 再写。
    Stale {
        main: BaseRef,
        paths: Vec<StalePath>,
    },
    /// 目的：其它拒绝（没有可提交的改动、落盘失败……）；带原文，调用方只上报。
    Broken(String),
}

impl fmt::Display for CommitRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommitRefusal::Stale { main, paths } => {
                let list = paths
                    .iter()
                    .map(|p| match p.base {
                        Some(base) => format!("{}（基线 {}）", p.path, base),
                        None => format!("{}（从未拉取）", p.path),
                    })
                    .collect::<Vec<_>>()
                    .join("、");
                write!(
                    f,
                    "基线过期：主线已是 {}，这些路径不是按它写的：{}——先 work_pull 再写。",
                    main.oid, list
                )
            }
            CommitRefusal::Broken(message) => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for CommitRefusal {}

// 接线前（PR2 写后自动提交）bin 目标里有不到使用者，接上即删这行。
#[cfg_attr(not(test), allow(dead_code))]
/// 目的：按路径核校基线——待提交的每条路径，记录的基线都必须等于主线当前头。
/// 约束：主线里没有、且从没拉取过的路径按新文件放行；主线里有却从没拉取过的路径拒绝
///   （否则等于拿没读过的旧内容覆盖上游）。
pub fn stale_paths(
    main: BaseRef,
    bases: &BTreeMap<String, u64>,
    paths: &[String],
    in_main: impl Fn(&str) -> bool,
) -> Vec<StalePath> {
    let mut stale = Vec::new();
    for path in paths {
        match bases.get(path) {
            Some(oid) if *oid == main.oid => {}
            Some(oid) => stale.push(StalePath {
                path: path.clone(),
                base: Some(*oid),
            }),
            None if in_main(path) => stale.push(StalePath {
                path: path.clone(),
                base: None,
            }),
            None => {}
        }
    }
    stale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bases(rows: &[(&str, u64)]) -> BTreeMap<String, u64> {
        rows.iter()
            .map(|(path, oid)| ((*path).to_string(), *oid))
            .collect()
    }

    #[test]
    fn stale_paths_accepts_only_current_head_baselines() {
        let main = BaseRef { oid: 9 };
        let recorded = bases(&[("a.md", 9), ("b.md", 7)]);
        let paths = vec!["a.md".to_string(), "b.md".to_string(), "c.md".to_string()];
        let stale = stale_paths(main, &recorded, &paths, |path| path == "never.md");
        // 基线等于主线头 → 放行；不等于 → 点名；主线里没有也从没拉过的新文件 → 放行
        assert_eq!(
            stale,
            vec![StalePath {
                path: "b.md".to_string(),
                base: Some(7),
            }]
        );
    }

    #[test]
    fn stale_paths_rejects_never_pulled_path_that_exists_upstream() {
        let main = BaseRef { oid: 9 };
        let stale = stale_paths(main, &BTreeMap::new(), &["a.md".to_string()], |path| {
            path == "a.md"
        });
        assert_eq!(
            stale,
            vec![StalePath {
                path: "a.md".to_string(),
                base: None,
            }]
        );
        // 文案要说清主线头与"先 pull 再写"
        let refusal = CommitRefusal::Stale { main, paths: stale };
        let text = refusal.to_string();
        assert!(text.contains("9") && text.contains("从未拉取") && text.contains("work_pull"));
    }
}
