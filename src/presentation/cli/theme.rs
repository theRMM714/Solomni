//! 目的：终端配色与颜色档位：语义 token 是唯一取色入口。
//! 管：档位判定（真彩 / 256 / 16 / 无色）、token 取值、上色拼接。
//! 不管：布局、输入与事件含义。
//! 联动：`cli/term.rs` 取色；`docs/presentation/cli/README.md`

use std::env;

/// 目的：颜色能力档位：决定一个 token 用 24 位、256 色还是 16 色写出。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Level {
    True,
    Ansi256,
    Ansi16,
    None,
}

/// 目的：语义 token：界面只认含义，不认具体色号。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Ink {
    Fg,
    Fg2,
    Dim,
    Line,
    Ice,
    Ice2,
}

impl Level {
    /// 目的：按环境判定档位：`--plain` 与 `NO_COLOR` 一律无色，`TERM=dumb` 同样无色。
    pub(super) fn detect(plain: bool) -> Level {
        if plain || env::var_os("NO_COLOR").is_some() {
            return Level::None;
        }
        let term = env::var("TERM").unwrap_or_default();
        if term == "dumb" {
            return Level::None;
        }
        let colorterm = env::var("COLORTERM").unwrap_or_default();
        let truecolor = colorterm.eq_ignore_ascii_case("truecolor")
            || colorterm.eq_ignore_ascii_case("24bit")
            || term.contains("truecolor")
            || term.contains("direct");
        if truecolor || env::var_os("WT_SESSION").is_some() || env::var_os("WEZTERM_PANE").is_some()
        {
            return Level::True;
        }
        if term.contains("256color") || env::var_os("TERM_PROGRAM").is_some() {
            return Level::Ansi256;
        }
        Level::Ansi16
    }
}

impl Ink {
    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Ink::Fg => (0xDD, 0xEB, 0xEE),
            Ink::Fg2 => (0xAF, 0xC3, 0xC9),
            Ink::Dim => (0x6F, 0x82, 0x89),
            Ink::Line => (0x2A, 0x38, 0x3E),
            Ink::Ice => (0x7F, 0xE7, 0xE0),
            Ink::Ice2 => (0x4F, 0xC9, 0xC6),
        }
    }

    fn c256(self) -> u8 {
        match self {
            Ink::Fg => 255,
            Ink::Fg2 => 250,
            Ink::Dim => 244,
            Ink::Line => 237,
            Ink::Ice => 116,
            Ink::Ice2 => 80,
        }
    }

    fn c16(self) -> u8 {
        match self {
            Ink::Fg | Ink::Fg2 => 97,
            Ink::Dim | Ink::Line => 90,
            Ink::Ice | Ink::Ice2 => 96,
        }
    }
}

/// 目的：256 色立方里最接近的一格：给不能查表的真彩色用。
fn cube(rgb: (u8, u8, u8)) -> u8 {
    let q = |v: u8| -> u8 { ((v as f32 / 255.0) * 5.0).round() as u8 };
    16 + 36 * q(rgb.0) + 6 * q(rgb.1) + q(rgb.2)
}

/// 目的：取色器：档位固定，所有上色与还原都从这里走。
#[derive(Clone, Copy)]
pub(super) struct Paint {
    level: Level,
}

impl Paint {
    /// 目的：建一个档位固定的取色器。
    pub(super) fn new(level: Level) -> Paint {
        Paint { level }
    }

    /// 目的：当前档位（启动页与降级文案要如实说）。
    pub(super) fn level(&self) -> Level {
        self.level
    }

    /// 目的：还原所有属性；无色档位返回空串，调用方无需分支。
    pub(super) fn reset(&self) -> &'static str {
        match self.level {
            Level::None => "",
            _ => "\x1b[0m",
        }
    }

    fn sgr(&self, ink: Ink, bold: bool) -> String {
        match self.level {
            Level::None => String::new(),
            Level::True => {
                let (r, g, b) = ink.rgb();
                let attr = if bold { "1;" } else { "" };
                format!("\x1b[{}38;2;{};{};{}m", attr, r, g, b)
            }
            Level::Ansi256 => {
                let b = if bold { "1;" } else { "" };
                format!("\x1b[{}38;5;{}m", b, ink.c256())
            }
            Level::Ansi16 => {
                let b = if bold { "1;" } else { "" };
                format!("\x1b[{}{}m", b, ink.c16())
            }
        }
    }

    /// 目的：给一段文字上色；无色档位原样返回。
    pub(super) fn paint(&self, ink: Ink, text: &str) -> String {
        if self.level == Level::None {
            return text.to_string();
        }
        format!("{}{}{}", self.sgr(ink, false), text, self.reset())
    }

    /// 目的：加粗上色（标题、字标这类要压过正文的文字）。
    pub(super) fn bold(&self, ink: Ink, text: &str) -> String {
        if self.level == Level::None {
            return text.to_string();
        }
        format!("{}{}{}", self.sgr(ink, true), text, self.reset())
    }

    /// 目的：按 24 位色上色；256 / 16 色档位各自降到最近的一档。
    pub(super) fn rgb(&self, rgb: (u8, u8, u8), text: &str) -> String {
        match self.level {
            Level::None => text.to_string(),
            Level::True => format!(
                "\x1b[38;2;{};{};{}m{}{}",
                rgb.0,
                rgb.1,
                rgb.2,
                text,
                self.reset()
            ),
            Level::Ansi256 => format!("\x1b[38;5;{}m{}{}", cube(rgb), text, self.reset()),
            Level::Ansi16 => {
                let lum = 299 * rgb.0 as u32 + 587 * rgb.1 as u32 + 114 * rgb.2 as u32;
                let code = if lum > 128 * 1000 { 96 } else { 36 };
                format!("\x1b[{}m{}{}", code, text, self.reset())
            }
        }
    }
}

/// 目的：字标每一行的取色：从近白冰青一路压到深青，形成纵向渐变。
pub(super) fn wordmark_rgb(row: usize) -> (u8, u8, u8) {
    const STEPS: [(u8, u8, u8); 7] = [
        (0xC9, 0xFF, 0xFB),
        (0xA9, 0xF6, 0xF1),
        (0x8D, 0xF0, 0xE8),
        (0x71, 0xE2, 0xDC),
        (0x55, 0xCF, 0xCB),
        (0x41, 0xB8, 0xB5),
        (0x2E, 0x9F, 0xA2),
    ];
    STEPS[row.min(STEPS.len() - 1)]
}
