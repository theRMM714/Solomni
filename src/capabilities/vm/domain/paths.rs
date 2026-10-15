//! 目的：宿主路径 → guest 路径的桥（纯逻辑）——按 guest 方言给拼法，同方言走恒等。
//! 管：宿主路径与 guest 路径两个方向的互换（跨方言时编进镜像根；单射、可逆的编码规则集中在这里）——系统工具跑在宿主侧，靠反向把模型给的 guest 写法译回宿主路径再校验。
//! 不管：谁在什么时候用它（挂载 / 命令装配 / 提示词 / 系统工具的寻址各自调用）、文件是否真存在。
//! 联动：GuestImage 的 os 决定 guest 方言；mounts_from 与命令装配共用同一座桥。

use std::path::{Component, Path, PathBuf};

/// 目的：路径方言——决定分隔符与根的写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathDialect {
    /// POSIX 拼法（Linux / macOS）。
    Posix,
    /// Windows 拼法（盘符 + 反斜杠）。
    Windows,
}

/// 目的：一座桥——宿主路径映成 guest 路径；同方言时是恒等（宿主路径原样在 guest 里成立）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathBridge {
    identity: bool,
    guest: PathDialect,
}

/// 目的：非恒等时 guest 侧的镜像根（POSIX guest 用；Windows guest 用系统盘下的同名根）。
const POSIX_MIRROR: &str = "/solomni-host";
const WINDOWS_MIRROR: &str = "C:\\solomni-host";

/// 目的：宿主平台的路径方言（平台名就是 std::env::consts::OS 的取值）。
pub fn host_dialect(host: &str) -> PathDialect {
    if host == "windows" {
        PathDialect::Windows
    } else {
        PathDialect::Posix
    }
}

impl PathBridge {
    /// 目的：按（宿主平台、guest 方言）造桥；两边同方言 = 恒等。
    pub fn new(host: &str, guest: PathDialect) -> PathBridge {
        PathBridge {
            identity: host_dialect(host) == guest,
            guest,
        }
    }

    /// 目的：按会话环境造桥——没有 guest（本机档）时按宿主自己的方言造（恒等）。
    pub fn for_session(host: &str, guest: Option<PathDialect>) -> PathBridge {
        PathBridge::new(host, guest.unwrap_or_else(|| host_dialect(host)))
    }

    /// 目的：宿主路径 → guest 路径；恒等时原样给。
    pub fn to_guest(self, host: &Path) -> String {
        if self.identity {
            return host.to_string_lossy().into_owned();
        }
        match self.guest {
            PathDialect::Posix => posix_mirror(host),
            PathDialect::Windows => windows_mirror(host),
        }
    }
}

/// 目的：Windows 宿主路径编进 POSIX guest——盘符小写当第一段（Windows 文件名不含 /，所以天然单射）。
fn posix_mirror(host: &Path) -> String {
    let mut out = String::from(POSIX_MIRROR);
    for c in host.components() {
        match c {
            Component::Prefix(p) => {
                out.push('/');
                out.push_str(
                    &p.as_os_str()
                        .to_string_lossy()
                        .replace(':', "")
                        .to_lowercase(),
                );
            }
            Component::Normal(s) => {
                out.push('/');
                out.push_str(&s.to_string_lossy());
            }
            Component::ParentDir => out.push_str("/.."),
            Component::RootDir | Component::CurDir => {}
        }
    }
    out
}

/// 目的：POSIX 宿主路径编进 Windows guest——镜像根下逐段，非法字符转义（可逆）。
fn windows_mirror(host: &Path) -> String {
    let mut out = String::from(WINDOWS_MIRROR);
    for c in host.components() {
        match c {
            Component::Normal(s) => {
                out.push('\\');
                out.push_str(&win_escape(&s.to_string_lossy()));
            }
            Component::ParentDir => {
                out.push('\\');
                out.push_str("..");
            }
            Component::Prefix(p) => {
                out.push('\\');
                out.push_str(&win_escape(&p.as_os_str().to_string_lossy()));
            }
            Component::RootDir | Component::CurDir => {}
        }
    }
    out
}

/// 目的：guest 路径 → 宿主路径——`to_guest` 的逆；认不出（不在镜像根里、也不是宿主绝对路径）给 None。
/// 约束：只做编码逆变换与写法识别，不做安全判断（是否落在允许根内由寻址层判）。
pub fn guest_to_host(bridge: PathBridge, guest: &str) -> Option<PathBuf> {
    if bridge.identity {
        return Some(PathBuf::from(guest));
    }
    let mirrored = match bridge.guest {
        PathDialect::Posix => posix_unmirror(guest),
        PathDialect::Windows => windows_unmirror(guest),
    };
    // 宿主写法直通：系统工具在宿主侧跑，宿主绝对路径本来就成立（提示词教的是 guest 写法，但两种都认）。
    mirrored.or_else(|| {
        let p = PathBuf::from(guest);
        p.is_absolute().then_some(p)
    })
}

/// 目的：POSIX guest 的镜像路径解回 Windows 宿主路径（`/solomni-host/d/a/b` → `D:\a\b`）。
fn posix_unmirror(guest: &str) -> Option<PathBuf> {
    let rest = guest.strip_prefix(POSIX_MIRROR)?.strip_prefix('/')?;
    let mut parts = rest.split('/');
    let drive = parts.next()?;
    if drive.len() != 1 || !drive.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&drive.to_ascii_uppercase());
    out.push(':');
    for seg in parts {
        out.push('\\');
        out.push_str(seg);
    }
    Some(PathBuf::from(out))
}

/// 目的：Windows guest 的镜像路径解回 POSIX 宿主路径（`C:\solomni-host\home\u\a` → `/home/u/a`）。
fn windows_unmirror(guest: &str) -> Option<PathBuf> {
    let rest = guest.strip_prefix(WINDOWS_MIRROR)?;
    let rest = rest.trim_start_matches('\\');
    if rest.is_empty() {
        return None;
    }
    let mut out = PathBuf::new();
    for seg in rest.split('\\') {
        if seg.is_empty() {
            continue;
        }
        out.push(win_unescape(seg)?);
    }
    Some(out)
}

/// 目的：`win_escape` 的逆——只认它编过的那几个字符（其它 %XX 一律不认，避免歧义）。
fn win_unescape(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let h1 = it.next()?;
        let h2 = it.next()?;
        let byte = u8::from_str_radix(&format!("{h1}{h2}"), 16).ok()?;
        let ch = match byte {
            b'%' => '%',
            b':' => ':',
            b'*' => '*',
            b'?' => '?',
            b'"' => '"',
            b'<' => '<',
            b'>' => '>',
            b'|' => '|',
            b'\\' => '\\',
            b'/' => '/',
            _ => return None,
        };
        out.push(ch);
    }
    Some(out)
}

/// 目的：这座桥就是本会话的路径翻译器（模型侧 ↔ 宿主），系统工具的寻址按它翻译后再校验。
impl crate::kernel::ports::PathCodec for PathBridge {
    fn to_host(&self, raw: &str) -> Option<PathBuf> {
        guest_to_host(*self, raw)
    }

    fn to_view(&self, host: &Path) -> String {
        (*self).to_guest(host)
    }
}

/// 目的：把 POSIX 名字里 Windows 不认的字符转义成 %XX（% 自己先转，保证可逆）。
fn win_escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '%' => out.push_str("%25"),
            ':' => out.push_str("%3A"),
            '*' => out.push_str("%2A"),
            '?' => out.push_str("%3F"),
            '"' => out.push_str("%22"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '|' => out.push_str("%7C"),
            '\\' => out.push_str("%5C"),
            '/' => out.push_str("%2F"),
            _ => out.push(ch),
        }
    }
    out
}
