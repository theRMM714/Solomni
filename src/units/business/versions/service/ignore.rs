//! 目的：`work_ignore` 用例——读写本工作的 `.workignore`（工作区过滤的唯一名单）。
//! 管：op 分派（list / add / remove）、规则校验、行数与字节上限、改完的原子写回。
//! 不管：规则语义与匹配（`workbench` 的 `domain/filter.rs`）；工具参数与权限收口（`tools` 的工作区工具执行者）。
//! 联动：内容面（导入 / 未跟踪清单 / 提交展开）与检索面（grep）读同一份文件；`.gitignore` 不在这里改。

use super::*;
use crate::units::business::versions::api::IgnoreReport;

/// `.workignore` 的三种 op。
const LIST: &str = "list";
const ADD: &str = "add";
const REMOVE: &str = "remove";

impl VersionService {
    /// 目的：读 / 改本工作的过滤名单；`add` 去重、`remove` 精确删行，改完立即按规则语法校验。
    /// 约束：只动版本库目录里的 `.workignore`（不碰共享区内容与 `.gitignore`）；改完的整份规则必须能编译，
    ///   语法不合法就整次拒绝，不留半份坏规则。
    pub(super) fn work_ignore(
        &self,
        work: &str,
        op: &str,
        patterns: &[String],
    ) -> Result<IgnoreReport, String> {
        let (_, store) = self.work_roots(work)?;
        let path = store.join(filter::WORKIGNORE);
        let mut lines: Vec<String> = self
            .work_rules(&store)?
            .lines()
            .map(|line| line.to_string())
            .collect();

        match op {
            LIST => {
                if !patterns.is_empty() {
                    return Err("list 不收 patterns（要改规则用 add / remove）".to_string());
                }
            }
            ADD => {
                if patterns.is_empty() {
                    return Err("add 要给至少一条规则（patterns）".to_string());
                }
                for p in patterns {
                    filter::check_rule(p)?;
                    let rule = p.trim();
                    if lines.iter().any(|line| line.trim() == rule) {
                        continue;
                    }
                    lines.push(rule.to_string());
                }
            }
            REMOVE => {
                if patterns.is_empty() {
                    return Err("remove 要给至少一条规则（patterns）".to_string());
                }
                for p in patterns {
                    let rule = p.trim();
                    if rule.is_empty() {
                        return Err("remove 的规则不能是空行".to_string());
                    }
                    lines.retain(|line| line.trim() != rule);
                }
            }
            other => {
                return Err(format!("op 只认 list / add / remove，收到：{}", other));
            }
        }

        if op != LIST {
            let text = join_rules(&lines);
            if text.len() > filter::MAX_RULE_BYTES {
                return Err(format!(
                    "过滤名单太长（{} 字节，上限 {}）：规则只放「不算工作区内容」的短名单",
                    text.len(),
                    filter::MAX_RULE_BYTES
                ));
            }
            if lines.len() > filter::MAX_RULE_LINES {
                return Err(format!(
                    "过滤名单行数太多（{} 行，上限 {}）",
                    lines.len(),
                    filter::MAX_RULE_LINES
                ));
            }
            // 语法校验：坏一行就整次拒绝——grep 与导入都按这份规则跑，不能留半份坏规则。
            filter::Rules::compile(&store, &text)?;
            // 同一个工作的规则写入串行：进程内闸 + 版本库跨进程锁，避免两个 agent 互相覆盖。
            let gate = self.gate(work);
            let _guard = gate.lock().unwrap_or_else(|e| e.into_inner());
            let _lock = self.store.lock(&store)?;
            self.store.write_text(&path, &text)?;
        }

        Ok(IgnoreReport { rules: lines })
    }
}

/// 目的：把规则行拼回文件文本：非空即以换行收尾（与文本编辑器写出来的一致）。
fn join_rules(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}
