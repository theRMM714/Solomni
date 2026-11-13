//! 目的：输入框的行编辑：缓冲区、光标与历史。
//! 管：按键到编辑动作的映射、光标移动、历史回溯、提交时记历史。
//! 不管：键盘读取与绘制（`cli/term.rs`）、命令含义（`cli.rs`）。
//! 联动：`cli/term.rs` 把终端事件翻成 `Key`。

use std::cell::RefCell;

/// 目的：一次按键的语义：终端只负责把事件翻成它。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Char(char),
    Paste(String),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    Enter,
    Tab,
    Escape,
    KillLine,
    Interrupt,
}

/// 目的：一键之后该做什么：继续编辑、提交、清空输入，或收摊（Ctrl+C）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Edit,
    Submit,
    Cancel,
    Quit,
}

thread_local! {
    static HISTORY: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// 目的：单行输入缓冲：文字 + 光标 + 上翻历史；历史跨提示符共用。
pub(crate) struct Editor {
    text: Vec<char>,
    at: usize,
    seen: Option<usize>,
}

impl Editor {
    /// 目的：一个空输入框（历史不随之清空）。
    pub(crate) fn new() -> Editor {
        Editor {
            text: Vec::new(),
            at: 0,
            seen: None,
        }
    }

    /// 目的：当前整行文字。
    pub(crate) fn line(&self) -> String {
        self.text.iter().collect()
    }

    /// 目的：光标之前的文字（画光标列要用）。
    pub(crate) fn head(&self) -> String {
        self.text[..self.at].iter().collect()
    }

    /// 目的：清空输入框（光标回到行首）。
    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.at = 0;
        self.seen = None;
    }

    /// 目的：提交：把这一行记进历史并清空输入框。
    pub(crate) fn commit(&mut self) -> String {
        let line = self.line();
        if !line.trim().is_empty() {
            HISTORY.with(|h| {
                let mut h = h.borrow_mut();
                if h.last() != Some(&line) {
                    h.push(line.clone());
                }
                if h.len() > 100 {
                    h.remove(0);
                }
            });
        }
        self.clear();
        line
    }

    /// 目的：处理一次按键；返回下一步动作（编辑 / 提交 / 清空 / 收摊）。
    pub(crate) fn key(&mut self, key: Key) -> Step {
        match key {
            Key::Char(c) => {
                self.text.insert(self.at, c);
                self.at += 1;
                self.seen = None;
                Step::Edit
            }
            Key::Paste(s) => {
                for c in s.chars() {
                    let c = if c == '\n' || c == '\r' { ' ' } else { c };
                    self.text.insert(self.at, c);
                    self.at += 1;
                }
                self.seen = None;
                Step::Edit
            }
            Key::Backspace => {
                if self.at > 0 {
                    self.at -= 1;
                    self.text.remove(self.at);
                    self.seen = None;
                }
                Step::Edit
            }
            Key::Delete => {
                if self.at < self.text.len() {
                    self.text.remove(self.at);
                    self.seen = None;
                }
                Step::Edit
            }
            Key::Left => {
                self.at = self.at.saturating_sub(1);
                Step::Edit
            }
            Key::Right => {
                if self.at < self.text.len() {
                    self.at += 1;
                }
                Step::Edit
            }
            Key::Home => {
                self.at = 0;
                Step::Edit
            }
            Key::End => {
                self.at = self.text.len();
                Step::Edit
            }
            Key::Up => {
                let prev = HISTORY.with(|h| {
                    let h = h.borrow();
                    if h.is_empty() {
                        return None;
                    }
                    let at = match self.seen {
                        None => h.len() - 1,
                        Some(0) => 0,
                        Some(i) => i - 1,
                    };
                    Some((at, h[at].clone()))
                });
                if let Some((at, line)) = prev {
                    self.seen = Some(at);
                    self.set(&line);
                }
                Step::Edit
            }
            Key::Down => {
                let next = self.seen.and_then(|i| {
                    HISTORY.with(|h| {
                        let h = h.borrow();
                        if i + 1 < h.len() {
                            Some((i + 1, h[i + 1].clone()))
                        } else {
                            Some((usize::MAX, String::new()))
                        }
                    })
                });
                if let Some((at, line)) = next {
                    if at == usize::MAX {
                        self.seen = None;
                        self.set("");
                    } else {
                        self.seen = Some(at);
                        self.set(&line);
                    }
                }
                Step::Edit
            }
            Key::Enter => Step::Submit,
            Key::Tab => Step::Edit,
            Key::Escape => Step::Cancel,
            Key::KillLine => {
                self.clear();
                Step::Edit
            }
            Key::Interrupt => Step::Quit,
        }
    }

    /// 目的：按补全候选改这一行：唯一候选补全并留一个空格，多候选先补公共前缀。
    /// 参数：`cands` 是已按输入筛过的候选（命令名与别名）。
    /// 返回：候选个数（调用方据此决定要不要进一步提示）。
    pub(crate) fn complete(&mut self, cands: &[String]) -> usize {
        if cands.is_empty() {
            return 0;
        }
        if cands.len() == 1 {
            let mut line = cands[0].clone();
            line.push(' ');
            self.set(&line);
            return 1;
        }
        let mut common = cands[0].clone();
        for next in &cands[1..] {
            let keep = common
                .chars()
                .zip(next.chars())
                .take_while(|(a, b)| a == b)
                .count();
            common = common.chars().take(keep).collect();
        }
        if common.chars().count() > self.text.len() {
            self.set(&common);
        }
        cands.len()
    }

    fn set(&mut self, line: &str) {
        self.text = line.chars().collect();
        self.at = self.text.len();
    }
}

/// 目的：测试用的历史清零：每个用例从空历史开始。
#[cfg(test)]
pub(crate) fn clear_history() {
    HISTORY.with(|h| h.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_line(ed: &mut Editor, text: &str) {
        for c in text.chars() {
            ed.key(Key::Char(c));
        }
    }

    /// 历史只留最近一百条；上下翻到头都停住，越过最新一条回到空行。
    #[test]
    fn history_keeps_the_latest_hundred_lines() {
        clear_history();
        let mut ed = Editor::new();
        for i in 0..101 {
            type_line(&mut ed, &format!("l{i}"));
            ed.commit();
        }
        assert_eq!(
            super::HISTORY.with(|h| h.borrow().len()),
            100,
            "超出的一百条丢掉最旧"
        );
        ed.key(Key::Up);
        assert_eq!(ed.line(), "l100", "从最新一条开始");
        ed.key(Key::Up);
        assert_eq!(ed.line(), "l99");
        for _ in 0..150 {
            ed.key(Key::Up);
        }
        assert_eq!(ed.line(), "l1", "最旧就到第二条");
        ed.key(Key::Up);
        assert_eq!(ed.line(), "l1", "到头再上翻不动");
        ed.key(Key::Down);
        assert_eq!(ed.line(), "l2");
        for _ in 0..100 {
            ed.key(Key::Down);
        }
        assert_eq!(ed.line(), "", "翻过最新一条是空行");
    }

    /// 没上翻过时 Down 不动；紧挨着的重复行只记一次。
    #[test]
    fn down_without_a_selection_and_duplicate_lines_are_no_ops() {
        clear_history();
        let mut ed = Editor::new();
        ed.key(Key::Down);
        assert_eq!(ed.line(), "", "没有历史游标时 Down 不动");
        for line in ["alpha", "alpha", "beta"] {
            type_line(&mut ed, line);
            ed.commit();
        }
        assert_eq!(
            super::HISTORY.with(|h| h.borrow().len()),
            2,
            "重复行只记一次"
        );
        ed.key(Key::Up);
        assert_eq!(ed.line(), "beta");
        ed.key(Key::Up);
        assert_eq!(ed.line(), "alpha");
    }

    /// 光标移动与删除在行首行尾都停住，不 panic 也不越过边界。
    #[test]
    fn cursor_moves_stop_at_the_line_edges() {
        clear_history();
        let mut ed = Editor::new();
        assert_eq!(ed.key(Key::Left), Step::Edit);
        assert_eq!(ed.key(Key::Backspace), Step::Edit);
        assert_eq!(ed.key(Key::Delete), Step::Edit, "空行 Delete 不 panic");
        assert_eq!(ed.key(Key::Right), Step::Edit, "空行右移不动");
        assert_eq!(ed.line(), "");
        type_line(&mut ed, "abc");
        ed.key(Key::Home);
        assert_eq!(ed.head(), "", "Home 回到行首");
        ed.key(Key::Delete);
        assert_eq!(ed.line(), "bc", "Delete 删光标处的字符");
        ed.key(Key::End);
        assert_eq!(ed.head(), "bc", "End 到行尾");
        ed.key(Key::Right);
        assert_eq!(ed.head(), "bc", "行尾再右移不动");
        ed.key(Key::Left);
        assert_eq!(ed.head(), "b");
        ed.key(Key::Backspace);
        assert_eq!(ed.line(), "c", "Backspace 删光标前的字符");
        ed.key(Key::Home);
        ed.key(Key::Backspace);
        assert_eq!(ed.line(), "c", "行首退格不动");
    }

    /// 粘贴把换行折成空格，并在光标处插入中文。
    #[test]
    fn paste_folds_newlines_and_inserts_at_the_cursor() {
        clear_history();
        let mut ed = Editor::new();
        type_line(&mut ed, "尾");
        ed.key(Key::Home);
        assert_eq!(ed.key(Key::Paste("中文\r\n第二行".to_string())), Step::Edit);
        assert_eq!(ed.line(), "中文  第二行尾");
    }

    /// 控制键各自映射到动作；空白行原样返回且不记历史。
    #[test]
    fn control_keys_map_to_steps_and_blank_lines_stay_out_of_history() {
        clear_history();
        let mut ed = Editor::new();
        type_line(&mut ed, "abc");
        assert_eq!(ed.key(Key::Tab), Step::Edit);
        assert_eq!(ed.line(), "abc", "Tab 不改行");
        assert_eq!(ed.key(Key::Enter), Step::Submit);
        assert_eq!(ed.key(Key::Escape), Step::Cancel);
        assert_eq!(ed.key(Key::Interrupt), Step::Quit);
        assert_eq!(ed.key(Key::KillLine), Step::Edit);
        assert_eq!(ed.line(), "", "KillLine 清空整行");
        type_line(&mut ed, "   ");
        assert_eq!(ed.commit(), "   ", "空白行原样返回");
        ed.key(Key::Up);
        assert_eq!(ed.line(), "", "空白行不进历史");
    }

    /// 补全：唯一候选补全并留空格，多候选只补公共前缀且不倒退。
    #[test]
    fn completion_takes_the_only_candidate_or_the_common_prefix() {
        clear_history();
        let mut ed = Editor::new();
        type_line(&mut ed, "si");
        let cands = vec!["single".to_string(), "sincere".to_string()];
        assert_eq!(ed.complete(&cands), 2, "多候选返回个数");
        assert_eq!(ed.line(), "sin", "补到公共前缀");
        assert_eq!(ed.complete(&cands), 2);
        assert_eq!(ed.line(), "sin", "公共前缀不再变长就不动");
        assert_eq!(ed.complete(&["help".to_string()]), 1);
        assert_eq!(ed.line(), "help ", "唯一候选补全并留一个空格");
        assert_eq!(ed.complete(&[]), 0, "没有候选不动");
        assert_eq!(ed.line(), "help ");
    }
}
