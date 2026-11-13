//! 目的：终端列宽：把字符串换算成终端格子数（东亚宽字符占两格）。
//! 管：字符宽度判定、按列截断、按列补空格。
//! 不管：颜色、布局与输入语义。
//! 联动：`cli/term.rs` 画框与光标定位；`cli/input.rs` 光标列。

/// 目的：单个字符占几格：组合符与零宽字符 0 格，东亚宽字符 2 格，其余 1 格。
pub(super) fn width(c: char) -> usize {
    let v = c as u32;
    if v == 0 {
        return 0;
    }
    if v < 0x20 || (0x7F..0xA0).contains(&v) {
        return 0;
    }
    if (0x0300..=0x036F).contains(&v)
        || (0x200B..=0x200F).contains(&v)
        || (0x20D0..=0x20FF).contains(&v)
        || (0xFE00..=0xFE0F).contains(&v)
        || v == 0xFEFF
        || (0x1F3FB..=0x1F3FF).contains(&v)
    {
        return 0;
    }
    if (0x1100..=0x115F).contains(&v)
        || (0x2E80..=0x303E).contains(&v)
        || (0x3041..=0x33FF).contains(&v)
        || (0x3400..=0x4DBF).contains(&v)
        || (0x4E00..=0x9FFF).contains(&v)
        || (0xA000..=0xA4CF).contains(&v)
        || (0xA960..=0xA97F).contains(&v)
        || (0xAC00..=0xD7A3).contains(&v)
        || (0xF900..=0xFAFF).contains(&v)
        || (0xFE10..=0xFE19).contains(&v)
        || (0xFE30..=0xFE6F).contains(&v)
        || (0xFF00..=0xFF60).contains(&v)
        || (0xFFE0..=0xFFE6).contains(&v)
        || (0x1F300..=0x1F64F).contains(&v)
        || (0x1F900..=0x1F9FF).contains(&v)
        || (0x20000..=0x3FFFD).contains(&v)
    {
        return 2;
    }
    1
}

/// 目的：整个字符串占几格。
pub(crate) fn cols(s: &str) -> usize {
    s.chars().map(width).sum()
}

/// 目的：按列截断（不切断宽字符）；超出上限时补省略号。
pub(crate) fn cut(s: &str, max: usize) -> String {
    if cols(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = width(c);
        if used + w > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}
/// 目的：按列折行（不切断宽字符）；空行原样保留，供对话框正文排版。
pub(crate) fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut out = Vec::new();
    for raw in text.lines() {
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut row = String::new();
        let mut used = 0;
        for c in raw.chars() {
            let cw = width(c);
            if used + cw > max && !row.is_empty() {
                out.push(row);
                row = String::new();
                used = 0;
            }
            row.push(c);
            used += cw;
        }
        if !row.is_empty() {
            out.push(row);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 控制符与组合符不占格，东亚宽字符占两格，其余一格。
    #[test]
    fn width_classifies_zero_wide_and_narrow_chars() {
        assert_eq!(width('\0'), 0, "NUL 不占格");
        assert_eq!(width('\u{1}'), 0, "控制字符不占格");
        assert_eq!(width('\u{7f}'), 0, "DEL 不占格");
        assert_eq!(width('\u{9f}'), 0, "C1 控制字符不占格");
        for c in [
            '\u{300}',
            '\u{200b}',
            '\u{20d0}',
            '\u{fe0f}',
            '\u{feff}',
            '\u{1f3fb}',
        ] {
            assert_eq!(width(c), 0, "组合符与零宽字符不占格：{c:?}");
        }
        for c in [
            '\u{1100}',
            '\u{2e80}',
            '\u{3041}',
            '\u{3400}',
            '中',
            '\u{a000}',
            '\u{a960}',
            '\u{ac00}',
            '\u{f900}',
            '\u{fe10}',
            '\u{fe30}',
            '\u{ff21}',
            '\u{ffe0}',
            '😀',
            '\u{1f900}',
            '\u{20000}',
        ] {
            assert_eq!(width(c), 2, "东亚宽字符占两格：{c:?}");
        }
        assert_eq!(width('a'), 1, "半角字符占一格");
        assert_eq!(width('é'), 1, "带音标的拉丁字母仍是一格");
    }

    /// 混排按格相加：宽字符两格、零宽字符不占。
    #[test]
    fn cols_sums_mixed_widths() {
        assert_eq!(cols(""), 0);
        assert_eq!(cols("abc"), 3);
        assert_eq!(cols("中a"), 3);
        assert_eq!(cols("中文a\u{301}"), 5);
        assert_eq!(cols("\u{1}中"), 2);
    }

    /// 截断：没超宽与正好等宽都原样返回；超宽按整字符截并补省略号。
    #[test]
    fn cut_keeps_text_within_the_column_limit() {
        assert_eq!(cut("abc", 10), "abc");
        assert_eq!(cut("abc", 3), "abc", "正好等宽不截");
        assert_eq!(cut("中文", 4), "中文", "宽字符正好等宽不截");
        assert_eq!(cut("中文abc", 5), "中文…");
        assert_eq!(cut("中文", 3), "中…", "不留半格");
        assert_eq!(cut("", 0), "");
        assert_eq!(cut("abc", 0), "…", "上限为零也给省略号");
    }

    /// 折行：按列折且不劈开宽字符；上限为零按一列算。
    #[test]
    fn wrap_respects_the_column_limit() {
        assert_eq!(wrap("abcdef", 0), vec!["a", "b", "c", "d", "e", "f"]);
        assert_eq!(wrap("中", 1), vec!["中"], "一格放不下宽字符也整字成行");
        assert_eq!(wrap("中文中文", 2), vec!["中", "文", "中", "文"]);
        assert_eq!(wrap("abcde", 3), vec!["abc", "de"]);
        assert_eq!(wrap("a\n\nb", 4), vec!["a", "", "b"]);
        assert_eq!(wrap("", 10), vec![""]);
    }
}
