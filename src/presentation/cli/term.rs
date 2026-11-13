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

const HINT: &str = "Enter 发送 · Tab 补全 · Esc 清空 · Ctrl+U 清行 · ↑↓ 历史 · Ctrl+C 退出";

const FRAME_LINES: u16 = 5;

type Completer = fn(&str) -> Vec<String>;

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static COMPLETER: RefCell<Option<Completer>> = const { RefCell::new(None) };
    #[cfg(test)]
    static CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

struct Ui {
    paint: Paint,
    tty: bool,
    cols: usize,
    status: String,
    ascii: bool,
    anim: bool,
}

/// 目的：初始化终端会话；返回是否交互（否 = 老式逐行输入，输出保持机器可解析）。
/// 参数：`plain` 强制纯文本；`anim` 为假时生成态只画静态帧。
pub(super) fn init(plain: bool, anim: bool) -> bool {
    let tty = !plain && io::stdout().is_terminal() && io::stdin().is_terminal();
    let paint = Paint::new(Level::detect(plain || !tty));
    let ascii = std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false);
    UI.with(|cell| {
        *cell.borrow_mut() = Some(Ui {
            paint,
            tty,
            cols: term_cols(),
            status: String::new(),
            ascii,
            anim,
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
    emit(text);
}

/// 目的：登记 Tab 补全的候选来源：命令表由 `cli.rs` 注入，终端层不认识任何命令。
pub(super) fn set_completer(f: Completer) {
    COMPLETER.with(|cell| *cell.borrow_mut() = Some(f));
}

/// 目的：是否交互终端：只有交互模式才用富渲染与活状态行，非 TTY 保持逐行原样。
pub(super) fn tty() -> bool {
    current().1
}

/// 目的：当前终端列数（对话框与折行按它排版）；读不到时按 80 列。
pub(super) fn cols() -> usize {
    current().2
}

/// 目的：终端是不是 `TERM=dumb`：是则所有画框字形退回 ASCII。
fn ascii() -> bool {
    UI.with(|cell| cell.borrow().as_ref().map(|ui| ui.ascii).unwrap_or(false))
}

/// 目的：生成态是否允许动画（`--no-anim` 与降级时为假，spinner 停在静态帧）。
pub(super) fn anim() -> bool {
    UI.with(|cell| cell.borrow().as_ref().map(|ui| ui.anim).unwrap_or(false))
}

/// 目的：把画框与状态字形退回纯 ASCII：`TERM=dumb` 下整屏仍然可读。
fn fold(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘' => '+',
            '─' | '━' => '-',
            '│' | '┃' => '|',
            '●' | '◆' | '◈' => '*',
            '✓' => 'v',
            '✗' | '✘' => 'x',
            '›' => '>',
            '↑' => '^',
            '↓' => 'v',
            '→' => '>',
            '←' => '<',
            '⏺' => 'o',
            '·' => '-',
            '█' => '#',
            '⠋' | '⠙' | '⠹' | '⠸' | '⠼' | '⠴' | '⠦' | '⠧' | '⠇' | '⠏' | '⠿' => {
                '|'
            }
            _ => c,
        })
        .collect()
}

/// 目的：按当前终端能力写出一行：`TERM=dumb` 时先折成 ASCII。
fn emit(text: &str) {
    if ascii() {
        say(&fold(text));
    } else {
        say(text);
    }
}

/// 目的：往转录区写一行；生产路径与 `println!` 等价，测试里由 `capture` 收进内存。
pub(super) fn say(text: &str) {
    #[cfg(test)]
    if CAPTURE.with(|cell| cell.borrow().is_some()) {
        CAPTURE.with(|cell| {
            if let Some(lines) = cell.borrow_mut().as_mut() {
                lines.push(text.to_string());
            }
        });
        return;
    }
    println!("{}", text);
}

/// 目的：仅测试：把 `say` 的输出收起来，返回闭包结果与逐行文本。
#[cfg(test)]
pub(super) fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    CAPTURE.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
    let value = f();
    let lines = CAPTURE.with(|cell| cell.borrow_mut().take().unwrap_or_default());
    (value, lines)
}

/// 目的：仅测试：装一套确定的终端能力，免得断言随运行环境漂移。
#[cfg(test)]
pub(super) fn test_ui(tty: bool, cols: usize, ascii: bool, anim: bool, level: Level) {
    UI.with(|cell| {
        *cell.borrow_mut() = Some(Ui {
            paint: Paint::new(level),
            tty,
            cols,
            status: String::new(),
            ascii,
            anim,
        });
    });
}

/// 目的：仅测试：卸掉终端能力，回到未初始化状态。
#[cfg(test)]
pub(super) fn reset_ui() {
    UI.with(|cell| *cell.borrow_mut() = None);
}

/// 目的：仅测试：读回状态行文字，验证 `status` 写进了同一份帧状态。
#[cfg(test)]
pub(super) fn status_snapshot() -> String {
    status_text()
}

/// 目的：一行原地位刷新的活状态（生成中的 spinner 与耗时）：不占转录区，事件落地前由调用方清掉。
pub(super) fn live(text: &str) {
    let (paint, tty, cols) = current();
    if !tty {
        return;
    }
    let text = if ascii() {
        fold(text)
    } else {
        text.to_string()
    };
    let body = paint.paint(Ink::Dim, &width::cut(&text, cols.saturating_sub(1)));
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
    let dumb = ascii();
    say("");
    if tty && dumb {
        emit(&paint.bold(Ink::Ice, "SOLOMNI"));
    } else if tty && cols >= 46 {
        for (row, art) in WORDMARK.iter().enumerate() {
            emit(&paint.rgb(theme::wordmark_rgb(row), art));
        }
    } else if tty {
        emit(&paint.bold(Ink::Ice, "SOLOMNI"));
    }
    emit(&paint.paint(Ink::Dim, tagline));
    if !facts.is_empty() {
        let sep = paint.paint(Ink::Line, "  ·  ");
        emit(&facts.join(&sep));
    }
    if tty {
        let note = if dumb {
            Some("配色：ASCII 档（TERM=dumb：无颜色、无画框字符）")
        } else {
            match paint.level() {
                Level::True => None,
                Level::Ansi256 => Some("配色：256 色档（真彩终端会显示冰青渐变）"),
                Level::Ansi16 => Some("配色：16 色档（真彩终端会显示冰青渐变）"),
                Level::None => Some("配色：无色档（NO_COLOR 或 --plain 时如此）"),
            }
        };
        if let Some(note) = note {
            emit(&paint.paint(Ink::Dim, note));
        }
    }
    say("");
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
                Some(Key::Tab) => {
                    complete(&mut editor);
                    Step::Edit
                }
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
        emit(&echo);
    }
    answer
}

fn complete(editor: &mut Editor) {
    let line = editor.line();
    if line.contains(' ') {
        return;
    }
    let cands = COMPLETER.with(|cell| match &*cell.borrow() {
        Some(f) => f(&line),
        None => Vec::new(),
    });
    editor.complete(&cands);
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
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::Esc => Some(Key::Escape),
        _ => None,
    }
}

struct Frame<'a> {
    paint: &'a Paint,
    cols: usize,
    label: &'a str,
    status: &'a str,
    ascii: bool,
    drawn: bool,
}

impl<'a> Frame<'a> {
    fn new(paint: &'a Paint, cols: usize, label: &'a str, status: &'a str) -> Frame<'a> {
        Frame {
            paint,
            cols,
            label,
            status,
            ascii: ascii(),
            drawn: false,
        }
    }

    fn write_line(&self, out: &mut impl Write, line: String) {
        let line = if self.ascii { fold(&line) } else { line };
        let _ = writeln!(out, "{}", line);
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
        self.write_line(out, top);
        self.write_line(
            out,
            format!(
                "{} {}{}{}{}",
                paint.paint(Ink::Line, "│"),
                paint.paint(Ink::Ice, "› "),
                paint.paint(Ink::Fg, &text),
                " ".repeat(padn),
                paint.paint(Ink::Line, " │")
            ),
        );
        self.write_line(
            out,
            format!(
                "{}{}",
                paint.paint(Ink::Line, "╰"),
                paint.paint(
                    Ink::Line,
                    &format!("{}╯", "─".repeat(total.saturating_sub(2)))
                )
            ),
        );
        self.write_line(out, paint.paint(Ink::Dim, &width::cut(HINT, total)));
        let status = paint.paint(Ink::Dim, &width::cut(self.status, total));
        let status = if self.ascii { fold(&status) } else { status };
        let _ = write!(out, "{}", status);
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
    fn ascii_fold_replaces_box_and_status_glyphs() {
        let folded = fold("╭─╮│✓✗●·›⏺");
        assert_eq!(folded, "+-+|vx*->o");
        assert!(folded.is_ascii());
    }

    #[test]
    fn ascii_frame_stays_ascii() {
        let paint = Paint::new(Level::None);
        let mut frame = Frame {
            paint: &paint,
            cols: 40,
            label: "转录中心",
            status: "● 就绪 · 模型 deepseek-flash",
            ascii: true,
            drawn: false,
        };
        let editor = Editor::new();
        let mut buf: Vec<u8> = Vec::new();
        frame.show(&mut buf, &editor);
        let text = String::from_utf8(buf).expect("utf8");
        for glyph in ["╭", "─", "│", "›", "●", "·", "↑", "↓"] {
            assert!(!text.contains(glyph), "TERM=dumb 下不出现画框字形：{glyph}");
        }
        let lines: Vec<&str> = text.split('\n').take(5).collect();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with("+-"), "上边框退回 +-");
        assert!(lines[1].contains("> "), "输入提示退回 >");
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

    #[test]
    fn say_prints_without_capture() {
        say("直接打印的一行");
        assert!(capture(|| ()).1.is_empty(), "capture 关闭后不收集旧行");
    }

    #[test]
    fn status_ink_and_line_use_current_ui() {
        test_ui(true, 80, false, true, Level::True);
        status("● 就绪 · 模型 m");
        assert!(status_snapshot().contains("就绪"));
        assert!(ink(Ink::Ice, "冰青").contains("冰青"));
        assert!(bold(Ink::Fg2, "加粗").contains("加粗"));
        assert!(tty());
        assert_eq!(cols(), 80);
        assert!(anim());
        let (_, lines) = capture(|| line("转录一行"));
        assert_eq!(lines, vec!["转录一行".to_string()]);
        reset_ui();
        assert!(!tty());
        assert_eq!(cols(), 80);
        assert!(!anim());
        assert!(status_snapshot().is_empty());
    }

    #[test]
    fn init_falls_back_to_plain_outside_a_terminal() {
        assert!(!init(true, false));
        assert!(!tty());
        assert!(!init(false, true));
        assert!(!tty(), "测试进程的 stdout 不是终端，没有交互模式");
        assert!(anim());
    }

    #[test]
    fn banner_draws_wordmark_only_on_wide_color_terminals() {
        let facts = vec!["工具链 cargo".to_string(), "模型 m".to_string()];
        test_ui(true, 80, false, true, Level::True);
        let (_, lines) = capture(|| banner("转录中心 v0.1.0", &facts));
        assert_eq!(
            lines.iter().filter(|l| l.contains('█')).count(),
            7,
            "七行像素字标"
        );
        assert!(lines.iter().any(|l| l.contains("转录中心")));
        assert!(lines.iter().any(|l| l.contains("工具链 cargo")));
        assert!(
            !lines.iter().any(|l| l.contains("配色：")),
            "真彩档不提示降级"
        );

        test_ui(true, 80, true, false, Level::None);
        let (_, lines) = capture(|| banner("副行", &[]));
        assert!(lines.iter().any(|l| l.contains("SOLOMNI")));
        assert!(lines.iter().any(|l| l.contains("配色：ASCII 档")));
        assert!(
            !lines.iter().any(|l| l.contains("  ·  ")),
            "事实为空时不写分隔"
        );

        test_ui(true, 40, false, true, Level::Ansi256);
        let (_, lines) = capture(|| banner("副行", &[]));
        assert!(lines.iter().any(|l| l.contains("SOLOMNI")));
        assert!(lines.iter().any(|l| l.contains("配色：256 色档")));

        test_ui(true, 40, false, true, Level::Ansi16);
        let (_, lines) = capture(|| banner("副行", &[]));
        assert!(lines.iter().any(|l| l.contains("配色：16 色档")));

        test_ui(true, 40, false, true, Level::None);
        let (_, lines) = capture(|| banner("副行", &[]));
        assert!(lines.iter().any(|l| l.contains("配色：无色档")));

        test_ui(false, 80, false, true, Level::None);
        let (_, lines) = capture(|| banner("副行", &[]));
        assert!(!lines.iter().any(|l| l.contains('█')), "非 TTY 不画字标");
        assert!(!lines.iter().any(|l| l.contains("配色：")));
        reset_ui();
    }

    #[test]
    fn live_lines_are_skipped_off_terminal_and_folded_on_ascii() {
        live("没有终端时什么也不写");
        live_clear();
        test_ui(true, 24, true, false, Level::None);
        live("生成中 · 12s");
        live_clear();
        test_ui(true, 24, false, true, Level::True);
        live("生成中 · 12s");
        live_clear();
        assert_eq!(fold("⠹ ●"), "| *", "ASCII 档下 spinner 与状态点也能读");
        reset_ui();
    }

    #[test]
    fn translate_covers_every_key() {
        let ctrl = KeyModifiers::CONTROL;
        let cases = [
            (KeyCode::Char('c'), ctrl, Some(Key::Interrupt)),
            (KeyCode::Char('u'), ctrl, Some(Key::KillLine)),
            (KeyCode::Char('a'), ctrl, Some(Key::Home)),
            (KeyCode::Char('e'), ctrl, Some(Key::End)),
            (KeyCode::Char('x'), KeyModifiers::NONE, Some(Key::Char('x'))),
            (KeyCode::Backspace, KeyModifiers::NONE, Some(Key::Backspace)),
            (KeyCode::Delete, KeyModifiers::NONE, Some(Key::Delete)),
            (KeyCode::Left, KeyModifiers::NONE, Some(Key::Left)),
            (KeyCode::Right, KeyModifiers::NONE, Some(Key::Right)),
            (KeyCode::Home, KeyModifiers::NONE, Some(Key::Home)),
            (KeyCode::End, KeyModifiers::NONE, Some(Key::End)),
            (KeyCode::Up, KeyModifiers::NONE, Some(Key::Up)),
            (KeyCode::Down, KeyModifiers::NONE, Some(Key::Down)),
            (KeyCode::Enter, KeyModifiers::NONE, Some(Key::Enter)),
            (KeyCode::Tab, KeyModifiers::NONE, Some(Key::Tab)),
            (KeyCode::Esc, KeyModifiers::NONE, Some(Key::Escape)),
            (KeyCode::F(1), KeyModifiers::NONE, None),
        ];
        for (code, mods, want) in cases {
            assert_eq!(translate(KeyEvent::new(code, mods)), want, "{code:?}");
        }
    }

    #[test]
    fn tab_completion_only_when_a_word_is_being_typed() {
        let mut editor = Editor::new();
        for c in "he".chars() {
            editor.key(Key::Char(c));
        }
        COMPLETER.with(|cell| {
            *cell.borrow_mut() = Some(|line: &str| {
                ["help", "hello"]
                    .iter()
                    .filter(|c| c.starts_with(line))
                    .map(|c| c.to_string())
                    .collect()
            })
        });
        complete(&mut editor);
        assert_eq!(editor.line(), "hel", "公共前缀补到只剩一个分叉点");

        let mut spaced = Editor::new();
        for c in "help me".chars() {
            spaced.key(Key::Char(c));
        }
        complete(&mut spaced);
        assert_eq!(spaced.line(), "help me");

        COMPLETER.with(|cell| *cell.borrow_mut() = None);
        let mut blank = Editor::new();
        complete(&mut blank);
        assert!(blank.line().is_empty());
    }

    #[test]
    fn repeated_show_erases_the_previous_frame() {
        let paint = Paint::new(Level::None);
        let mut frame = Frame::new(&paint, 60, "转录中心", "● 就绪");
        let editor = Editor::new();
        let mut buf: Vec<u8> = Vec::new();
        frame.show(&mut buf, &editor);
        let first = buf.len();
        frame.show(&mut buf, &editor);
        let text = String::from_utf8(buf).expect("utf8");
        assert!(text[first..].contains("\u{1b}[1A"), "第二次先上移擦掉旧帧");
        assert_eq!(
            text.matches("Enter 发送").count(),
            2,
            "两帧都完整写过（旧帧先被擦掉再重画）"
        );
    }

    #[test]
    fn panic_hook_restores_the_terminal() {
        let previous = std::panic::take_hook();
        let result = std::panic::catch_unwind(|| {
            install_panic_restore();
            panic!("测试用：触发恢复钩子");
        });
        std::panic::set_hook(previous);
        assert!(result.is_err());
    }
}
