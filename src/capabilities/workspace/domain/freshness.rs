//! 目的：判断读过的那些行在新版本里还在不在原样——按**内容**对齐，不看行号。
//! 管：行窗口判定的纯算法。
//!   把读过的那段行当成一个连续"行窗口"（新旧两版都按行切开），在新内容里找同样的窗口：
//!   找到 = 原样还在，找不到 = 那段内容被改过或被删掉。
//!   不看时间戳、不看遍历顺序；插一行导致的行号移位不算过期。
//! 不管：读事实的形状归 readfact.rs；过期结论不进转录，发送视图那次派生才算数。
//! 联动：被 workspace 服务的 work_stale_reads 调用（它负责取新旧两份正文）。

/// 目的：`[from, to]` 这几行（1 起）在新旧两版之间是否被改过。
/// 参数：`old` = 读的那一刻的全文；`new` = 现在的全文；`from`/`to` = 读到的行区间（含）。
/// 返回：true = 读过的行里有内容变了（这条读取已过期）；false = 读过的行原样还在。
// 接线前（PR2 的 2D 发送视图）bin 目标里还没有使用者，接上即删这行属性。
#[cfg_attr(not(test), allow(dead_code))]
pub fn lines_changed(old: &str, new: &str, from: u64, to: u64) -> bool {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let from = from.max(1) as usize;
    let to = (to.max(from as u64) as usize).min(a.len());
    // 读的时候什么都没读到（空文件 / 越过末行）：没有内容需要保护。
    if from > a.len() {
        return false;
    }
    let read = &a[from - 1..to];
    if read.is_empty() {
        return false;
    }
    !b.windows(read.len()).any(|window| window == read)
}

#[cfg(test)]
mod tests {
    use super::lines_changed;

    #[test]
    fn header_insertion_keeps_every_read_fresh() {
        let old = "a\nb\nc\n";
        let new = "zero\na\nb\nc\n";
        assert!(!lines_changed(old, new, 1, 3));
    }

    #[test]
    fn changed_line_only_invalidates_the_reads_that_cover_it() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nb\nC\nd\n";
        assert!(
            !lines_changed(old, new, 1, 2),
            "第 3 行变了，读 1-2 不受影响"
        );
        assert!(lines_changed(old, new, 3, 4), "读 3-4 覆盖了变掉的那行");
        assert!(lines_changed(old, new, 1, 4));
    }

    #[test]
    fn insertion_and_edit_at_once_only_invalidates_the_edited_range() {
        let old = "a\nb\nc\n";
        let new = "zero\na\nb\nC\n";
        assert!(!lines_changed(old, new, 1, 2), "读过的那两行原样还在");
        assert!(lines_changed(old, new, 3, 3), "第 3 行被改掉了");
    }

    #[test]
    fn deletion_of_read_lines_is_stale_and_untouched_prefix_is_not() {
        let old = "a\nb\nc\n";
        let new = "a\nc\n";
        assert!(!lines_changed(old, new, 1, 1));
        assert!(lines_changed(old, new, 2, 3));
    }

    #[test]
    fn identical_content_is_never_stale() {
        assert!(!lines_changed("a\nb\n", "a\nb\n", 1, 2));
    }
}
