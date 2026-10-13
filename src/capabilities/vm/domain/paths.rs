//! 目的：宿主路径 → guest 路径的桥（纯逻辑）——按 guest 方言给拼法，同方言走恒等。
//! 管：把宿主路径编成 guest 能用的路径（跨方言时编进镜像根；单射，可逆的编码规则集中在这里）。
//! 不管：反向解回（有消费点再加）、谁在什么时候用它（挂载 / 命令装配 / 提示词各自调用）、文件是否真存在。
//! 联动：GuestImage 的 os 决定 guest 方言；mounts_from 与命令装配共用同一座桥。

use std::path::{Component, Path};

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
