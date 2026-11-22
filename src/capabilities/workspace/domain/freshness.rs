//! 目的：判断读过的那些行在新版本里还在不在原样——按**内容**对齐，不看行号。
//! 管：行窗口判定的纯算法。
//!   把读过的那段行当成一个连续"行窗口"（新旧两版都按行切开），在新内容里找同样的窗口：
//!   找到 = 原样还在，找不到 = 那段内容被改过或被删掉。
//!   不看时间戳、不看遍历顺序；插一行导致的行号移位不算过期。
//! 不管：读事实的形状归 readfact.rs；过期结论不进转录，发送视图那次派生才算数。
//! 联动：被 workspace 服务的 judge_reads 调用（它取新旧两份正文，把内容比较翻成 ReadVerdict）。

/// 目的：`[from, to]` 这几行（1 起）在新旧两版之间是否被改过。
/// 参数：`old` = 读的那一刻的全文；`new` = 现在的全文；`from`/`to` = 读到的行区间（含）。
/// 返回：true = 读过的行里有内容变了（这条读取已过期）；false = 读过的行原样还在。
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

/// 目的：一条读取相对**当前**版本的判决。
/// 约束：SameVersion = 版本指纹相同；Shifted = 版本变了但读过的那段内容还在（行号可能移位）；
///   Changed = 那段内容确实变了。两个消费者对 Shifted 刻意取不同答案：占位符只要内容还在就
///   不换（模型看到的旧内容仍然可信），写门禁只认 SameVersion（整份覆盖不能抹掉没看见的改动）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadVerdict {
    /// 读的就是当前版本的指纹。
    SameVersion,
    /// 版本变了，但读过的那段内容原样还在新版本里。
    Shifted,
    /// 读过的那段内容被改过或删掉了。
    Changed,
}

impl ReadVerdict {
    /// 目的：占位符策略：只有内容真的变了才算过期（行号移位不算）。
    pub fn is_stale(self) -> bool {
        matches!(self, ReadVerdict::Changed)
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadVerdict, lines_changed};

    /// 目的：判决的三态：版本相同 / 行号移位但内容还在 / 内容变了。
    #[test]
    fn verdicts_separate_shifted_from_changed() {
        let old = "a\nb\nc\n";
        let shifted = "zero\na\nb\nc\n";
        let changed = "a\nb\nC\n";
        assert!(!lines_changed(old, shifted, 1, 3));
        assert!(!lines_changed(old, shifted, 4, 4));
        assert!(lines_changed(old, changed, 3, 3));
        assert!(!ReadVerdict::SameVersion.is_stale());
        assert!(!ReadVerdict::Shifted.is_stale());
        assert!(ReadVerdict::Changed.is_stale());
    }

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
