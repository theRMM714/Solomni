//! 目的：工具结果的**单次预算**——所有工具回执在送回模型前统一 head+tail 截断。
//! 管：字符上限与截断标记文案；read 的续读 offset、shell 的状态行 / 末行都保留在尾部。
//! 不管：各工具自己的分页与截断（在 tools 的 systool / kernel 的 receipt）；上下文压缩（在 session）。
//! 联动：由 tools 的 service 在所有执行路径返回前调用；细则见 docs/tools/tools-and-roles.md 二。

/// 目的：单次工具结果回执的字符上限（约 4k tokens）。
/// 约束：超出按 head+tail 截断并给出续读提示。
pub const MAX_RESULT_CHARS: usize = 16_000;

/// 目的：把一次工具回执收进统一预算；未超限原样返回。
/// 约束：保留开头约 1/4、结尾约 3/4（尾部常含 read 的续读 offset / shell 的状态行与末行）。
pub fn cap_result(text: String) -> String {
    let total = text.chars().count();
    if total <= MAX_RESULT_CHARS {
        return text;
    }
    let head_budget = MAX_RESULT_CHARS / 4;
    let tail_budget = MAX_RESULT_CHARS - head_budget;
    let head: String = text.chars().take(head_budget).collect();
    let tail: String = {
        let mut chars: Vec<char> = text.chars().rev().take(tail_budget).collect();
        chars.reverse();
        chars.into_iter().collect()
    };
    format!(
        "{head}
[结果截断] 本次回执共 {total} 字符，这里保留开头 {head_budget} / 结尾 {tail_budget}；需要更多请缩小查询范围或分段读取（read 可用 offset/limit 接着读）。
{tail}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_results_are_untouched() {
        let s = "短回执".to_string();
        assert_eq!(cap_result(s.clone()), s);
    }

    #[test]
    fn long_results_keep_head_and_tail_with_a_marker() {
        let head = "开头".repeat(100);
        let tail = "结尾".repeat(100);
        let middle = "中间".repeat(MAX_RESULT_CHARS);
        let out = cap_result(format!("{head}{middle}{tail}"));
        assert!(
            out.chars().count() < MAX_RESULT_CHARS + 200,
            "预算要真的生效（截断标记本身也要算进去）"
        );
        assert!(out.starts_with("开头"), "保留开头");
        assert!(out.ends_with("结尾"), "保留结尾");
        assert!(out.contains("[结果截断]"), "截断要如实标注");
        assert!(out.contains("共 "), "要报完整长度");
        assert!(out.contains("offset/limit"), "要给续读办法");
    }
}
