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
