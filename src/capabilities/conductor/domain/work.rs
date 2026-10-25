//! 目的：共享区版本化工作区六个核心自有工具（`work_pull` / `work_commit` / `work_status` /
//!   `work_versions` / `work_resolve` / `work_lost`）的纯逻辑——工具名、入参解析与回执拼装。
//! 管：工具名常量与 `is_work_tool`、取参（`str_list` / `str_arg`）、各工具回执的拼装。
//! 不管：真正的拉取 / 提交 / 状态 / 三方视图 / 解决 / 备份恢复（走 `conductor::service::work_tools`
//!   → `workspace::api::Workspace`）；冲突、状态与备份判定（在 `workspace`）。
//! 联动：由本能力的 `service/` 调用；工具名与 `systools/tools.yaml` 同一份名单。

use crate::capabilities::workspace::api::{
    ChangeKind, CommitReport, LostReport, PullReport, ResolveAction, ResolveEntry, ResolveReport,
    RestoreLostReport, StatusReport, VersionView, VERSION_TEXT_LIMIT,
};

pub const PULL: &str = "work_pull";
pub const COMMIT: &str = "work_commit";
pub const STATUS: &str = "work_status";
pub const VERSIONS: &str = "work_versions";
pub const RESOLVE: &str = "work_resolve";
pub const LOST: &str = "work_lost";

/// 目的：判定这个名字是不是共享区版本化的工具面（角色表 executor / solo 引用它们）。
pub fn is_work_tool(name: &str) -> bool {
    matches!(name, PULL | COMMIT | STATUS | VERSIONS | RESOLVE | LOST)
}

/// 回执里最多逐个列出的路径数（超出只说数量，不把整棵共享区灌进上下文）。
const MAX_LISTED: usize = 50;

fn paths(list: &[String]) -> String {
    if list.is_empty() {
        return "无".to_string();
    }
    let mut out = list
        .iter()
        .take(MAX_LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    if list.len() > MAX_LISTED {
        out.push_str(&format!("…（共 {} 条）", list.len()));
    }
    out
}

/// 目的：把 work_pull 的结果说成人能读的几行。
/// 返回：写入 / 跳过 / 冲突 / 主副本没有 各若干行（路径截断规则见 `MAX_LISTED`）。
pub fn render_pull(r: &PullReport) -> String {
    let mut out = format!(
        "已对准主副本提交 {}。写入沙箱 {} 个，已是最新 {} 个，无变化 {} 个，跳过（沙箱已有同名文件）{} 个，冲突 {} 个，主副本没有 {} 个。",
        r.commit,
        r.updated.len(),
        r.already.len(),
        r.no_change.len(),
        r.occupied.len(),
        r.conflicts.len(),
        r.missing.len()
    );
    if !r.updated.is_empty() {
        out.push_str(&format!("\n写入：{}", paths(&r.updated)));
    }
    if !r.occupied.is_empty() {
        out.push_str(&format!(
            "\n跳过（沙箱里已有未被跟踪的同名文件，未覆盖）：{}——先用 work_versions 看主副本与本地，\n合并后用 work_resolve（take=local）登记；放弃本地改动用 take=upstream。",
            paths(&r.occupied)
        ));
    }
    if !r.conflicts.is_empty() {
        let mut lines = Vec::new();
        for c in r.conflicts.iter().take(MAX_LISTED) {
            lines.push(format!("{}（{}）", c.path, c.kind.reason()));
        }
        if r.conflicts.len() > MAX_LISTED {
            lines.push(format!("…（共 {} 条）", r.conflicts.len()));
        }
        out.push_str(&format!(
            "\n冲突（未覆盖）：{}——冲突不会因为把两边内容合进沙箱就消失：\n先用 work_versions 看基线 / 主副本 / 沙箱三方；文本可人工合并后用 work_resolve（take=local）声明已解决，图片 / 视频 / 二进制不能合并，只能选边（take=upstream 取主副本，take=local 保留本地）。",
            lines.join("、")
        ));
    }
    if !r.missing.is_empty() {
        out.push_str(&format!(
            "\n主副本没有这些路径：{}——可能上游已删除；用 work_versions 看本地与基线，\n再用 work_resolve 选择 take=upstream（删本地）或 take=local（保留为新增）。",
            paths(&r.missing)
        ));
    }
    out
}

/// 目的：把 work_commit 的结果说成人能读的几行。
/// 返回：提交 id 与新增 / 修改 / 删除 各若干行。
pub fn render_commit(r: &CommitReport) -> String {
    let mut adds = Vec::new();
    let mut mods = Vec::new();
    let mut dels = Vec::new();
    for c in &r.changes {
        match c.kind {
            ChangeKind::Add => adds.push(c.path.clone()),
            ChangeKind::Modify => mods.push(c.path.clone()),
            ChangeKind::Delete => dels.push(c.path.clone()),
        }
    }
    let mut out = format!(
        "提交 {} 已落盘：新增 {}，修改 {}，删除 {}。",
        r.id,
        adds.len(),
        mods.len(),
        dels.len()
    );
    if !adds.is_empty() {
        out.push_str(&format!("\n新增：{}", paths(&adds)));
    }
    if !mods.is_empty() {
        out.push_str(&format!("\n修改：{}", paths(&mods)));
    }
    if !dels.is_empty() {
        out.push_str(&format!("\n删除（只删共享区；沙箱副本未动）：{}", paths(&dels)));
        out.push_str("\n沙箱里的同名文件如果还在，用 delete 工具清理；工作副本的删除不会自动跟着提交。");
    }
    out
}

/// 目的：把 work_status 的结果说成人能读的几行。
/// 返回：主副本当前提交与路径、你的工作副本状态、最近提交。
pub fn render_status(r: &StatusReport) -> String {
    let mut out = String::new();
    match &r.head {
        Some(h) => out.push_str(&format!(
            "主副本当前提交 {}（{}，{}）：{}",
            h.id, h.author, h.time, h.message
        )),
        None => out.push_str("主副本还没有任何提交"),
    }
    if !r.tree.is_empty() {
        out.push_str(&format!("\n主副本路径：{}", paths(&r.tree)));
        if r.tree_truncated {
            out.push_str("…（已截断，只列了一部分）");
        }
    }
    if !r.entries.is_empty() {
        out.push_str("\n你的工作副本状态：");
        for e in &r.entries {
            out.push_str(&format!("\n- {}：{}", e.path, e.state.as_str()));
        }
    }
    if !r.missing.is_empty() {
        out.push_str(&format!(
            "\n这些路径在基线、主副本与沙箱里都没有：{}",
            paths(&r.missing)
        ));
    }
    if !r.local_only.is_empty() {
        out.push_str(&format!(
            "\n沙箱里未跟踪（还没拉取、也没提交）的文件：{}",
            paths(&r.local_only)
        ));
        if r.local_only_truncated {
            out.push_str("…（已截断，只列了一部分）");
        }
    }
    if !r.recent.is_empty() {
        out.push_str("\n最近提交：");
        for c in &r.recent {
            out.push_str(&format!("\n- {} {}：{}", c.id, c.author, c.message));
        }
    }
    out
}

/// 目的：把 work_versions 的三方视图说成人能读的几行。
/// 返回：基线 / 主副本 / 沙箱的指纹、大小与文本预览；二进制只给指纹与大小。
pub fn render_versions(v: &VersionView) -> String {
    let mut out = format!("路径：{}", v.path);
    match &v.media_type {
        Some(t) => out.push_str(&format!("（媒体类型：{}，按扩展名提示）", t)),
        None => out.push_str("（媒体类型：未识别）"),
    }
    if let Some(id) = v.head_commit {
        out.push_str(&format!(
            "\n主副本提交：{}（take=local 时把这个 id 作为 head 传回）",
            id
        ));
    }
    let mut incomplete: Vec<&str> = Vec::new();
    for (label, item) in [("基线", &v.base), ("主副本", &v.head), ("沙箱", &v.local)] {
        if let Some(f) = item {
            if f.truncated || f.too_large {
                incomplete.push(label);
            }
        }
    }
    if !incomplete.is_empty() {
        out.push_str(&format!(
            "\n注意：{}的内容预览不完整/超限，不能只按这段内容合并；请用 read 读沙箱文件与共享主副本同一相对路径的全文，再合并。",
            incomplete.join("、")
        ));
    }
    for (label, item) in [("基线", &v.base), ("主副本", &v.head), ("沙箱", &v.local)] {
        match item {
            None => out.push_str(&format!(
                "\n{}：{}",
                label,
                if label == "基线" {
                    "未拉取"
                } else {
                    "不存在"
                }
            )),
            Some(f) => {
                let kind = if f.too_large {
                    "超过单文件上限"
                } else if f.text.is_some() {
                    "文本"
                } else {
                    "二进制/非 UTF-8"
                };
                out.push_str(&format!(
                    "\n{}：{}，hash={}，大小={} 字节",
                    label, kind, f.hash, f.size
                ));
                if f.too_large {
                    out.push_str("（只给指纹、大小与类型，不预览）");
                }
                if let Some(text) = &f.text {
                    let kb = VERSION_TEXT_LIMIT / 1024;
                    if f.truncated {
                        out.push_str(&format!(
                            "\n--- {}内容（只显示前 {} KiB，共 {} 字节）---\n{}",
                            label, kb, f.size, text
                        ));
                    } else {
                        out.push_str(&format!("\n--- {}内容 ---\n{}", label, text));
                    }
                }
            }
        }
    }
    out.push_str(&format!(
        "\n提示：文本预览最多 {} KiB；图片 / 视频 / 其它二进制只给指纹、大小与类型，不冒充文本；冲突只能显式选边：文本可人工合并后用 take=local（带 head=<上面主副本提交 id>），二进制 / 媒体只能 take=upstream 或 take=local，不做自动合并。",
        VERSION_TEXT_LIMIT / 1024
    ));
    out
}

/// 目的：把 work_resolve 的结果说成人能读的几行。
/// 返回：按动作分组的路径，以及下一步。
pub fn render_resolve(r: &ResolveReport) -> String {
    let mut out = format!(
        "已按 take={} 解决 {} 条路径。",
        r.take.as_str(),
        r.entries.len()
    );
    if let Some(id) = r.head {
        out.push_str(&format!("\n基于主副本提交：{}", id));
    }
    if !r.warnings.is_empty() {
        out.push_str("\n注意：");
        for w in r.warnings.iter().take(MAX_LISTED) {
            out.push_str(&format!("\n- {}：{}", w.path, w.kind.as_str()));
        }
        if r.warnings.len() > MAX_LISTED {
            out.push_str(&format!("\n…（共 {} 条）", r.warnings.len()));
        }
    }
    for action in [
        ResolveAction::WroteUpstream,
        ResolveAction::RemovedLocal,
        ResolveAction::Marked,
        ResolveAction::Cleared,
        ResolveAction::NoChange,
    ] {
        let list: Vec<String> = r
            .entries
            .iter()
            .filter(|e| e.action == action)
            .map(|e| e.path.clone())
            .collect();
        if !list.is_empty() {
            out.push_str(&format!("\n{}：{}", action.as_str(), paths(&list)));
        }
    }
    let backups: Vec<&ResolveEntry> = r.entries.iter().filter(|e| e.backup.is_some()).collect();
    if !backups.is_empty() {
        out.push_str("\n已备份被覆盖/删除的本地版本（可用 work_lost 找回）：");
        for e in backups.iter().take(MAX_LISTED) {
            out.push_str(&format!(
                "\n- {} → {}",
                e.path,
                e.backup.as_deref().unwrap_or("")
            ));
        }
        if backups.len() > MAX_LISTED {
            out.push_str(&format!("\n…（共 {} 条）", backups.len()));
        }
    }
    if !r.missing.is_empty() {
        out.push_str(&format!(
            "\n基线、主副本与沙箱都没有：{}",
            paths(&r.missing)
        ));
    }
    out.push_str(
        "\n下一步：take=local 保留的是沙箱当前内容（文本合并结果或二进制选边结果），提交会以本地覆盖主副本当前版；请确认已包含上游改动。take=upstream 已把沙箱对齐主副本；被覆盖/删除的本地版本在 lost/ 下，可用 work_lost 找回。",
    );
    out
}

/// 目的：把 work_lost 的清单说成人能读的几行：备份路径、指纹、当前沙箱指纹与恢复方式。
pub fn render_lost(r: &LostReport) -> String {
    if r.entries.is_empty() {
        return "没有找到被覆盖的本地版本。只有 work_resolve take=upstream 覆盖/删除本地文件时才会产生备份。"
            .to_string();
    }
    let mut out = format!("被覆盖的本地版本（agent {}）：共 {} 份", r.agent, r.total);
    if r.truncated {
        out.push_str(&format!("（只列前 {} 份）", r.entries.len()));
    }
    for e in r.entries.iter().take(MAX_LISTED) {
        out.push_str(&format!(
            "\n- {}：备份 hash={}，大小={} 字节；当前沙箱={}",
            e.path,
            e.hash,
            e.size,
            match &e.current_hash {
                Some(hash) => format!("hash={}", hash),
                None => "不存在".to_string(),
            }
        ));
    }
    if r.entries.len() > MAX_LISTED {
        out.push_str(&format!("\n…（共 {} 条）", r.entries.len()));
    }
    out.push_str(
        "\n恢复：work_lost path=<路径> restore=true expect_current=<当前沙箱 hash；不存在传空串>；恢复会覆盖当前沙箱同路径，先确认新内容已另存/提交。",
    );
    out
}

/// 目的：把 work_restore_lost 的结果说成人能读的几行。
pub fn render_restore_lost(r: &RestoreLostReport) -> String {
    format!(
        "已恢复 {}：备份 hash={}，大小={} 字节；覆盖前沙箱={}。备份仍保留在 lost/ 下，可再次恢复。恢复后请 work_versions 看三方，合并后再提交。",
        r.path,
        r.hash,
        r.size,
        match &r.previous {
            Some(hash) => format!("hash={}", hash),
            None => "不存在".to_string(),
        }
    )
}

/// 目的：从 args 里取一个字符串数组（缺省 = 空）。
/// 错误：不是数组、或数组里有非字符串时如实报错。
pub fn str_list(args: &serde_json::Value, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::new();
            for it in items {
                match it.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => return Err(format!("{} 的每一项都必须是字符串", key)),
                }
            }
            Ok(out)
        }
        Some(_) => Err(format!("{} 必须是一个字符串数组", key)),
    }
}

/// 目的：从 args 里取一个字符串（缺省 = `None`）。
pub fn str_arg(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::workspace::api::FileVersion;

    fn version(hash: &str, size: u64, text: Option<&str>, too_large: bool) -> FileVersion {
        FileVersion {
            hash: hash.to_string(),
            size,
            text: text.map(str::to_string),
            truncated: false,
            too_large,
        }
    }

    /// 目的：回执按媒体类型说人话；二进制不给“内容”段，也不假装文本。
    #[test]
    fn versions_receipt_reports_media_type_and_never_previews_binary() {
        let v = VersionView {
            path: "clip.mp4".to_string(),
            head_commit: None,
            media_type: Some("video/mp4".to_string()),
            base: None,
            head: Some(version("h", 9, None, false)),
            local: None,
        };
        let text = render_versions(&v);
        assert!(text.contains("video/mp4"), "{}", text);
        assert!(text.contains("二进制/非 UTF-8"), "{}", text);
        assert!(
            !text.contains("--- 主副本内容"),
            "二进制不能给内容段：{}",
            text
        );
    }

    /// 目的：超过上限只给元数据，回执明确说“不预览”。
    #[test]
    fn versions_receipt_marks_oversize_metadata_only() {
        let v = VersionView {
            path: "big.bin".to_string(),
            head_commit: None,
            media_type: None,
            base: None,
            head: Some(version("h", 70_000_000, None, true)),
            local: None,
        };
        let text = render_versions(&v);
        assert!(text.contains("超过单文件上限"), "{}", text);
        assert!(text.contains("不预览"), "{}", text);
        assert!(text.contains("不能只按"), "{}", text);
        assert!(!text.contains("--- 主副本内容"), "{}", text);
    }

    /// 目的：文本预览仍按上限显示，并在截断时说明。
    #[test]
    fn versions_receipt_keeps_text_preview_with_limit() {
        let mut f = version("h", (VERSION_TEXT_LIMIT + 10) as u64, Some("开头"), false);
        f.truncated = true;
        let v = VersionView {
            path: "note.md".to_string(),
            head_commit: Some(7),
            media_type: Some("text/markdown".to_string()),
            base: None,
            head: Some(f),
            local: None,
        };
        let text = render_versions(&v);
        assert!(text.contains("--- 主副本内容"), "{}", text);
        assert!(text.contains("只显示前"), "{}", text);
        assert!(text.contains("主副本提交：7"), "{}", text);
        assert!(text.contains("不能只按"), "{}", text);
        assert!(text.contains("read"), "{}", text);
    }

    /// 目的：resolve 回执要说清基于哪个主副本、哪些路径有风险、哪些本地版本已备份。
    #[test]
    fn resolve_receipt_shows_head_warnings_and_backups() {
        use crate::capabilities::workspace::api::{
            ResolveReport, ResolveTake, ResolveWarning, ResolveWarningKind,
        };
        let r = ResolveReport {
            take: ResolveTake::Local,
            head: Some(5),
            entries: vec![
                ResolveEntry {
                    path: "note.md".to_string(),
                    action: ResolveAction::Marked,
                    backup: None,
                },
                ResolveEntry {
                    path: "cover.png".to_string(),
                    action: ResolveAction::WroteUpstream,
                    backup: Some("lost/a/cover.png".to_string()),
                },
            ],
            missing: Vec::new(),
            warnings: vec![ResolveWarning {
                path: "other.md".to_string(),
                kind: ResolveWarningKind::NoBase,
            }],
        };
        let text = render_resolve(&r);
        assert!(text.contains("基于主副本提交：5"), "{}", text);
        assert!(
            text.contains("other.md") && text.contains("从未拉取"),
            "{}",
            text
        );
        assert!(text.contains("lost/a/cover.png"), "{}", text);
        assert!(text.contains("work_lost"), "{}", text);
    }

    /// 目的：work_lost / restore 回执给出备份指纹、当前沙箱指纹与恢复结果。
    #[test]
    fn lost_receipts_show_backup_and_current_hashes() {
        use crate::capabilities::workspace::api::LostEntry;
        let r = LostReport {
            agent: "a".to_string(),
            total: 1,
            entries: vec![LostEntry {
                path: "note.md".to_string(),
                hash: "hb".to_string(),
                size: 3,
                current_hash: Some("hc".to_string()),
            }],
            truncated: false,
        };
        let text = render_lost(&r);
        assert!(text.contains("note.md") && text.contains("hb") && text.contains("hc"));
        assert!(text.contains("work_lost"), "{}", text);
        let rr = RestoreLostReport {
            path: "note.md".to_string(),
            hash: "hb".to_string(),
            size: 3,
            previous: Some("hc".to_string()),
        };
        let text = render_restore_lost(&rr);
        assert!(text.contains("已恢复") && text.contains("hb") && text.contains("hc"));
    }
}
