//! 目的：终端 I/O：能力判定、启动页、固定输入框与行读取。
//! 管：TTY 判定、raw mode 开关、帧绘制与擦除、事件到按键的翻译、非交互回退。
//! 不管：配色取值（`theme`）、编辑语义（`input`）、命令含义（`cli.rs`）。
//! 联动：`cli.rs` 取用；`cli/view.rs` 提供启动页文案；`docs/presentation/cli/README.md`

use super::input::{Editor, Key, Step};
use super::theme::{self, Ink, Level, Paint};
use super::width;
use crossterm::cursor::{MoveDown, MoveToColumn, MoveUp, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::ResetColor;
use crossterm::terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode};
use std::cell::RefCell;
use std::io::{self, IsTerminal, Write};

const WORDMARK: [&str; 7] = [
    " ███   ███  █      ███  █   █ █   █ █████",
    "█   █ █   █ █     █   █ ██ ██ ██  █   █  ",
    "█     █   █ █     █   █ █ █ █ █ █ █   █  ",
    " ███  █   █ █     █   █ █   █ █  ██   █  ",
    "    █ █   █ █     █   █ █   █ █   █   █  ",
    "█   █ █   █ █     █   █ █   █ █   █   █  ",
    " ███   ███  █████  ███  █   █ █   █ █████",
];

const HINT: &str = "Enter 发送 · Esc 清空 · Ctrl+U 清行 · ↑↓ 历史 · Ctrl+C 退出";

const FRAME_LINES: u16 = 5;

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

struct Ui {
    paint: Paint,
    tty: bool,
    cols: usize,
    status: String,
}

/// 目的：初始化终端会话；返回是否交互（否 = 老式逐行输入，输出保持机器可解析）。
pub(super) fn init(plain: bool) -> bool {
    let tty = !plain && io::stdout().is_terminal() && io::stdin().is_terminal();
    let paint = Paint::new(Level::detect(plain || !tty));
    UI.with(|cell| {
        *cell.borrow_mut() = Some(Ui {
            paint,
            tty,
            cols: term_cols(),
            status: String::new(),
        });
    });
    if tty {
        install_panic_restore();
    }
    tty
}

fn term_cols() -> usize {
    terminal::size().map(|(c, _)| c as usize).unwrap_or(80)
}

fn current() -> (Paint, bool, usize) {
    UI.with(|cell| match cell.borrow().as_ref() {
        Some(ui) => (ui.paint, ui.tty, ui.cols),
        None => (Paint::new(Level::None), false, 80),
    })
}

/// 目的：换状态行文字（启动事实在进入主循环前定下来，之后每次重绘都带同一份）。
pub(super) fn status(text: &str) {
    UI.with(|cell| {
        if let Some(ui) = cell.borrow_mut().as_mut() {
            ui.status = text.to_string();
        }
    });
}

fn status_text() -> String {
    UI.with(|cell| match cell.borrow().as_ref() {
        Some(ui) => ui.status.clone(),
        None => String::new(),
    })
}

/// 目的：给一段文字上色（供启动页事实与状态行拼接）。
pub(super) fn ink(ink: Ink, text: &str) -> String {
    current().0.paint(ink, text)
}

/// 目的：加粗上色（标题与关键字段）。
pub(super) fn bold(ink: Ink, text: &str) -> String {
    current().0.bold(ink, text)
}

/// 目的：往转录区打一行（不留在固定帧里，随输出滚上去）。
pub(super) fn line(text: &str) {
    println!("{}", text);
}

/// 目的：是否交互终端：只有交互模式才用富渲染与活状态行，非 TTY 保持逐行原样。
pub(super) fn tty() -> bool {
    current().1
}

/// 目的：当前终端列数（对话框与折行按它排版）；读不到时按 80 列。
pub(super) fn cols() -> usize {
    current().2
}

/// 目的：一行原地位刷新的活状态（生成中的 spinner 与耗时）：不占转录区，事件落地前由调用方清掉。
pub(super) fn live(text: &str) {
    let (paint, tty, cols) = current();
    if !tty {
        return;
    }
    let body = paint.paint(Ink::Dim, &width::cut(text, cols.saturating_sub(1)));
    let mut out = io::stdout();
    let _ = write!(out, "\r\x1b[2K{}", body);
    let _ = out.flush();
}

/// 目的：清掉活状态行（事件要落到转录区之前调用；没有活状态行时无害）。
pub(super) fn live_clear() {
    let (_, tty, _) = current();
    if !tty {
        return;
    }
    let mut out = io::stdout();
    let _ = write!(out, "\r\x1b[2K");
    let _ = out.flush();
}

fn install_panic_restore() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), Show, ResetColor);
            prev(info);
        }));
    });
}

/// 目的：启动页：冰青字标 + 副行 + 事实行，一次性打进转录区（之后不再重绘）。
pub(super) fn banner(tagline: &str, facts: &[String]) {
    let (paint, tty, cols) = current();
    let mut out = io::stdout();
    let _ = writeln!(out);
    if tty && cols >= 46 {
        for (row, art) in WORDMARK.iter().enumerate() {
            let _ = writeln!(out, "{}", paint.rgb(theme::wordmark_rgb(row), art));
        }
    } else if tty {
        let _ = writeln!(out, "{}", paint.bold(Ink::Ice, "SOLOMNI"));
    }
    let _ = writeln!(out, "{}", paint.paint(Ink::Dim, tagline));
    if !facts.is_empty() {
        let sep = paint.paint(Ink::Line, "  ·  ");
        let _ = writeln!(out, "{}", facts.join(&sep));
    }
    if tty {
        let note = match paint.level() {
            Level::True => None,
            Level::Ansi256 => Some("配色：256 色档（真彩终端会显示冰青渐变）"),
            Level::Ansi16 => Some("配色：16 色档（真彩终端会显示冰青渐变）"),
            Level::None => Some("配色：无色档（NO_COLOR 或 --plain 时如此）"),
        };
        if let Some(note) = note {
            let _ = writeln!(out, "{}", paint.paint(Ink::Dim, note));
        }
    }
    let _ = writeln!(out);
}

/// 目的：读一行输入；交互模式下用固定输入框，非交互模式保持逐行回退（机器可解析）。
pub(super) fn ask(label: &str) -> Option<String> {
    let (paint, tty, cols) = current();
    if !tty {
        return plain_ask(label);
    }
    tty_ask(&paint, cols, label, &status_text())
}

fn plain_ask(label: &str) -> Option<String> {
    print!("{} ", label);
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_string()),
    }
}

fn tty_ask(paint: &Paint, cols: usize, label: &str, status: &str) -> Option<String> {
    if enable_raw_mode().is_err() {
        return plain_ask(label);
    }
    let mut out = io::stdout();
    let _ = execute!(out, Show);
    let mut frame = Frame::new(paint, cols, label, status);
    let mut editor = Editor::new();
    frame.show(&mut out, &editor);
    let answer = loop {
        let ev = match event::read() {
            Ok(e) => e,
            Err(_) => break None,
        };
        let step = match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release => match translate(key) {
                Some(key) => editor.key(key),
                None => Step::Edit,
            },
            Event::Paste(text) => editor.key(Key::Paste(text)),
            Event::Resize(w, _) => {
                frame.cols = (w as usize).max(24);
                Step::Edit
            }
            _ => continue,
        };
        match step {
            Step::Edit => frame.show(&mut out, &editor),
            Step::Cancel => {
                editor.clear();
                frame.show(&mut out, &editor);
            }
            Step::Submit => break Some(editor.commit()),
            Step::Quit => break None,
        }
    };
    frame.erase(&mut out);
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), Show, ResetColor);
    if let Some(text) = &answer {
        let echo = format!(
            "{} {}",
            paint.paint(Ink::Ice, "›"),
            paint.paint(Ink::Fg, text)
        );
        println!("{}", echo);
    }
    answer
}

fn translate(key: KeyEvent) -> Option<Key> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c') if ctrl => Some(Key::Interrupt),
        KeyCode::Char('u') if ctrl => Some(Key::KillLine),
        KeyCode::Char('a') if ctrl => Some(Key::Home),
        KeyCode::Char('e') if ctrl => Some(Key::End),
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Escape),
        _ => None,
    }
}

struct Frame<'a> {
    paint: &'a Paint,
    cols: usize,
    label: &'a str,
    status: &'a str,
    drawn: bool,
}

impl<'a> Frame<'a> {
    fn new(paint: &'a Paint, cols: usize, label: &'a str, status: &'a str) -> Frame<'a> {
        Frame {
            paint,
            cols,
            label,
            status,
            drawn: false,
        }
    }

    fn erase(&mut self, out: &mut impl Write) {
        if !self.drawn {
            return;
        }
        let _ = execute!(
            out,
            MoveUp(1),
            MoveToColumn(0),
            Clear(ClearType::FromCursorDown)
        );
        self.drawn = false;
    }

    fn show(&mut self, out: &mut impl Write, editor: &Editor) {
        self.erase(out);
        let paint = self.paint;
        let total = self.cols.saturating_sub(1).max(24);
        let inner = total.saturating_sub(2);
        let label = width::cut(self.label, inner.saturating_sub(4));
        let dash = total.saturating_sub(5 + width::cols(&label));
        let top = format!(
            "{} {}{}",
            paint.paint(Ink::Line, "╭─"),
            paint.paint(Ink::Ice2, &label),
            paint.paint(Ink::Line, &format!(" {}╮", "─".repeat(dash)))
        );
        let area = total.saturating_sub(6);
        let text = width::cut(&editor.line(), area);
        let padn = area.saturating_sub(width::cols(&text));
        let _ = writeln!(out, "{}", top);
        let _ = writeln!(
            out,
            "{} {}{}{}{}",
            paint.paint(Ink::Line, "│"),
            paint.paint(Ink::Ice, "› "),
            paint.paint(Ink::Fg, &text),
            " ".repeat(padn),
            paint.paint(Ink::Line, " │")
        );
        let _ = writeln!(
            out,
            "{}{}",
            paint.paint(Ink::Line, "╰"),
            paint.paint(
                Ink::Line,
                &format!("{}╯", "─".repeat(total.saturating_sub(2)))
            )
        );
        let _ = writeln!(out, "{}", paint.paint(Ink::Dim, &width::cut(HINT, total)));
        let _ = write!(
            out,
            "{}",
            paint.paint(Ink::Dim, &width::cut(self.status, total))
        );
        let _ = out.flush();
        self.drawn = true;
        let head = width::cols(&editor.head()).min(area);
        let _ = execute!(
            out,
            MoveUp(FRAME_LINES - 1),
            MoveToColumn(0),
            MoveToColumn((4 + head) as u16),
            MoveDown(1)
        );
        let _ = out.flush();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_keeps_one_width_for_every_line() {
        let paint = Paint::new(Level::None);
        let status = "● 就绪 · 模型 deepseek-flash · 档位 host";
        let mut frame = Frame::new(&paint, 80, "转录中心", status);
        let mut editor = Editor::new();
        for c in "把 CLI 前端重做一下".chars() {
            editor.key(Key::Char(c));
        }
        let mut buf: Vec<u8> = Vec::new();
        frame.show(&mut buf, &editor);
        let text = String::from_utf8(buf).expect("utf8");
        let lines: Vec<&str> = text.split('\n').take(5).collect();
        assert_eq!(lines.len(), 5);
        let total = 80usize.saturating_sub(1);
        assert_eq!(width::cols(lines[0]), total, "上边框与正文同宽");
        assert_eq!(width::cols(lines[2]), total, "下边框与正文同宽");
        assert!(
            lines[1].contains("把 CLI 前端重做一下"),
            "输入框里是正在编辑的这一行"
        );
        assert!(width::cols(lines[1]) >= total - 6, "输入行按宽度补齐");
        assert!(lines[3].contains("Enter 发送"), "快捷键提示行");
        assert!(width::cols(lines[3]) <= total);
        assert!(lines[4].contains("就绪"), "状态行跟着输入框走");
    }

    #[test]
    fn long_label_is_cut_not_wrapped() {
        let paint = Paint::new(Level::None);
        let mut frame = Frame::new(&paint, 40, "选哪一项（序号 / 选项 id；回车 = 先不答）", "");
        let editor = Editor::new();
        let mut buf: Vec<u8> = Vec::new();
        frame.show(&mut buf, &editor);
        let text = String::from_utf8(buf).expect("utf8");
        let top = text.split('\n').next().expect("上边框");
        assert_eq!(width::cols(top), 39, "窄终端里标签被截断，框不换行");
    }
}
