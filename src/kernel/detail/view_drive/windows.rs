//! 目的：产品根的视图盘——把产品根映射成一个盘符（如 Z:），容器 cwd 走视图写法，
//!   于是容器只需要产品根这一级的读属性，不必再向产品根之外的祖先链写 ACL。
//! 管：盘符分配与回收（DefineDosDeviceW / QueryDosDeviceW）、记录文件 .home/view-drive.json 的
//!   清扫与多实例复用、进程级 active 视图、以及给沙箱用的路径编解码（PathCodec）。
//! 不管：ACL 授权（confine/windows）、装配层在何时调用（入口层组合根与守门进程）。
//! 联动：confine/windows.rs 的 prepare_fence 用 active_shell_view(spec) 判断祖先链边界，
//!   container.rs 用它决定 CreateProcessW 的 cwd 与程序名写法；ports::PathCodec 的实现在这里。
//!   记录文件 .home/view-drive.json 是 JSON 数组：[{"letter":"Z:","root":"D:\\…","users":[123]}]。
//!   每个进程启动时分配（同 root 复用字母、登记自己的 pid），退出时注销；启动清扫死 pid、
//!   目标不存在或不匹配的条目，让残留的盘符映射不会越积越多。盘符映射是登录会话级的
//!   （同用户的所有进程都看得见），因此守门进程只调 attach_view_drive 读记录、校验映射，不重复登记 pid。

use crate::kernel::contracts::fence::FenceSpec;
use crate::kernel::ports::PathCodec;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 目的：一个生效中的视图盘：盘符 + 它指向的真实产品根。
/// 约束：letter 形如 "Z:"（含冒号、不含反斜杠）；root 是规范化后的真实绝对路径。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewDrive {
    letter: String,
    root: PathBuf,
}

impl ViewDrive {
    /// 目的：盘符（"Z:" 形式）。
    ///   契约方法——装配层与诊断按需取用；bin crate 内暂时没有调用点，故放行 dead_code。
    #[allow(dead_code)]
    pub fn letter(&self) -> &str {
        &self.letter
    }

    /// 目的：视图盘指向的真实产品根。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 目的：真实路径转视图写法；不在产品根下（或不是绝对路径）给 None。
    /// 约束：比较逐分量、大小写不敏感（Windows 路径语义）；产品根本身译成 "Z:\\"。
    pub fn view_of(&self, host: &Path) -> Option<PathBuf> {
        if !host.is_absolute() {
            return None;
        }
        let rel = relative_under(&self.root, host)?;
        let mut text = format!("{}\\", self.letter);
        let rel_text = rel.to_string_lossy();
        if !rel_text.is_empty() {
            text.push_str(&rel_text);
        }
        Some(PathBuf::from(text))
    }

    /// 目的：视图写法转真实路径；不是本视图盘的写法（盘符不同、或不是绝对路径）给 None。
    pub fn host_of(&self, view: &Path) -> Option<PathBuf> {
        let mut comps = view.components();
        match comps.next() {
            Some(Component::Prefix(prefix)) => {
                if !prefix
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&self.letter)
                {
                    return None;
                }
            }
            _ => return None,
        }
        if !matches!(comps.next(), Some(Component::RootDir)) {
            return None;
        }
        let mut out = self.root.clone();
        for c in comps {
            match c {
                Component::Normal(n) => out.push(n),
                Component::CurDir => {}
                _ => return None,
            }
        }
        Some(out)
    }

    /// 目的：测试用构造：只造一个读写路径的视图对象，不碰盘符映射。
    #[cfg(test)]
    pub(crate) fn for_test(letter: &str, root: &Path) -> ViewDrive {
        ViewDrive {
            letter: letter.to_string(),
            root: root.to_path_buf(),
        }
    }

    /// 目的：这个路径是不是本视图盘的写法（只看盘符，不要求路径存在）。
    pub fn is_view(&self, path: &Path) -> bool {
        match path.components().next() {
            Some(Component::Prefix(prefix)) => prefix
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&self.letter),
            _ => false,
        }
    }
}

/// 目的：进程级生效的视图盘（ensure_view_drive / attach_view_drive 设置，释放时清）。
static ACTIVE: Mutex<Option<ViewDrive>> = Mutex::new(None);

/// 目的：当前进程可用的视图盘；没有就是 None（调用方据此回退到真实路径）。
pub fn active_view() -> Option<ViewDrive> {
    ACTIVE.lock().ok().and_then(|g| g.clone())
}

/// 目的：这次围栏调用该不该用视图写法——只有 cwd 落在私有沙箱（shell 这类）才算。
///   模块工具的 cwd 是模块根，沿用宿主 cwd 与宿主程序名：把它也译成视图写法，程序名里带分隔符的
///   相对路径会被算成跨卷绝对路径，容器里反而调不起来；而它本就不需要视图盘（祖先链只对
///   cwd == private 的深叶路径写）。
/// 参数：spec —— 本次围栏的路径面。
/// 返回：视图盘生效且 spec.cwd 就是 spec.private 时给盘；否则 None（调用方回退真实路径）。
pub(crate) fn active_shell_view(spec: &FenceSpec) -> Option<ViewDrive> {
    if spec.private.as_os_str().is_empty() || spec.cwd != spec.private {
        return None;
    }
    active_view()
}

fn set_active(view: &ViewDrive) {
    if let Ok(mut g) = ACTIVE.lock() {
        *g = Some(view.clone());
    }
}

fn clear_active(letter: &str) {
    if let Ok(mut g) = ACTIVE.lock()
        && g.as_ref()
            .is_some_and(|v| v.letter.eq_ignore_ascii_case(letter))
    {
        *g = None;
    }
}

/// 目的：按当前视图盘把宿主路径译成视图写法；没视图盘或是相对路径就给 None。
///   契约出口——围栏内一律用 active_shell_view 判断，bin crate 内暂时没有调用点。
#[allow(dead_code)]
pub fn view_of(host: &Path) -> Option<PathBuf> {
    active_view().and_then(|v| v.view_of(host))
}

/// 目的：按当前视图盘把视图写法译回真实路径；不是视图写法就给 None。
///   契约出口——工具参数里的视图写法由装配层在需要时译回；bin crate 内暂时没有调用点。
#[allow(dead_code)]
pub fn host_of(view: &Path) -> Option<PathBuf> {
    active_view().and_then(|v| v.host_of(view))
}

/// 目的：给沙箱用的路径编解码器：写视图写法、读回真实路径。
pub fn view_codec() -> Option<Arc<dyn PathCodec>> {
    active_view().map(|v| Arc::new(ViewCodec(v)) as Arc<dyn PathCodec>)
}

/// PathCodec 的视图盘实现：to_view 译成盘符写法，to_host 把视图写法译回真实路径，
/// 收到的已经是真实路径就原样通过（编解码器要能同时吃两种写法）。
#[derive(Debug)]
struct ViewCodec(ViewDrive);

impl PathCodec for ViewCodec {
    fn to_host(&self, raw: &str) -> Option<PathBuf> {
        let path = Path::new(raw);
        if self.0.is_view(path) {
            self.0.host_of(path)
        } else if path.is_absolute() {
            Some(path.to_path_buf())
        } else {
            None
        }
    }

    fn to_view(&self, host: &Path) -> String {
        self.0
            .view_of(host)
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|| host.to_string_lossy().into_owned())
    }
}

/// 目的：记录文件里的一条：盘符、真实根、还在用它的进程。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ViewEntry {
    letter: String,
    root: String,
    #[serde(default)]
    users: Vec<u32>,
}

fn record_path(home: &Path) -> PathBuf {
    home.join("view-drive.json")
}

fn lock_path(home: &Path) -> PathBuf {
    home.join("view-drive.lock")
}

/// 目的：视图盘记录的跨进程写事务锁（与台账同一套路：进程内互斥 + 文件锁、60 秒有界等待）。
struct ViewGuard {
    _proc: std::sync::MutexGuard<'static, ()>,
    _file: std::fs::File,
}

static VIEW_MUTEX: Mutex<()> = Mutex::new(());

fn lock_view(home: &Path) -> Result<ViewGuard, String> {
    let proc = VIEW_MUTEX
        .lock()
        .map_err(|_| "视图盘记录进程内锁已被毒化".to_string())?;
    std::fs::create_dir_all(home).map_err(|e| format!("建私有区失败：{}", e))?;
    let path = lock_path(home);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("打开视图盘记录锁失败（{}）：{}", path.display(), e))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match file.try_lock() {
            Ok(()) => {
                return Ok(ViewGuard {
                    _proc: proc,
                    _file: file,
                });
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    return Err(
                        "视图盘记录被另一个进程占用，等待超时（稍后重试或重启应用）".to_string()
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("视图盘记录加锁失败：{}", e));
            }
        }
    }
}

fn load_entries(home: &Path) -> Vec<ViewEntry> {
    std::fs::read_to_string(record_path(home))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<ViewEntry>>(&t).ok())
        .unwrap_or_default()
}

fn save_entries(home: &Path, entries: &[ViewEntry]) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| format!("建私有区失败：{}", e))?;
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    std::fs::write(record_path(home), text).map_err(|e| e.to_string())
}

/// 目的：清扫后留下的条目，以及要撤掉的盘符映射（盘符 + 记录的根，撤映射时用它做精确匹配）。
struct Sweep {
    kept: Vec<ViewEntry>,
    remove: Vec<ViewEntry>,
}

/// 目的：纯逻辑清扫——死 pid 摘掉、映射不在或指向别处就丢条目、目标根已不存在就撤映射、没用户的撤映射。
/// 参数：alive 判 pid 活性；query 查盘符当前映射目标（None = 没映射）；exists 判记录的目标根还在不在。
/// 约束：映射指向别处（别的程序占了同一字母）时只丢条目、不撤别人的映射。
fn sweep_entries(
    entries: Vec<ViewEntry>,
    alive: impl Fn(u32) -> bool,
    query: impl Fn(&str) -> Option<PathBuf>,
    exists: impl Fn(&Path) -> bool,
) -> Sweep {
    let mut kept = Vec::new();
    let mut remove = Vec::new();
    for mut entry in entries {
        let mapped = query(&entry.letter);
        let same = mapped
            .as_deref()
            .is_some_and(|target| same_path(target, Path::new(&entry.root)));
        if !same {
            continue;
        }
        if !exists(Path::new(&entry.root)) {
            remove.push(entry);
            continue;
        }
        entry.users.retain(|pid| alive(*pid));
        if entry.users.is_empty() {
            remove.push(entry);
        } else {
            kept.push(entry);
        }
    }
    Sweep { kept, remove }
}

/// 视图盘可用的盘符顺序：从 Z: 往 C:（跳过 A:/B: 软盘传统字母）。
const LETTERS: [&str; 24] = [
    "Z:", "Y:", "X:", "W:", "V:", "U:", "T:", "S:", "R:", "Q:", "P:", "O:", "N:", "M:", "L:", "K:",
    "J:", "I:", "H:", "G:", "F:", "E:", "D:", "C:",
];

/// 目的：清扫时兜底撤掉**记录里没有**的孤儿映射——崩溃 / 强杀绕过 release，映射还在、记录已空。
/// 参数：root = 本产品根（home 的父级）；kept = 清扫后仍在用的条目；query = 查盘符当前的映射目标。
/// 返回：要撤的（盘符, 映射目标）。
/// 约束：只撤目标**正好是本产品根**的字母（映射只指向根，不指向根内子目录）；别的程序占的字母、
///   别的产品实例的根一律不碰，产品根本身是盘根时也不会误撤别人的映射；记录里还有活持有者的字母也不碰。
fn orphan_mappings(
    root: &Path,
    kept: &[ViewEntry],
    query: impl Fn(&str) -> Option<PathBuf>,
) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for letter in LETTERS {
        if kept.iter().any(|e| e.letter.eq_ignore_ascii_case(letter)) {
            continue;
        }
        let Some(target) = query(letter) else {
            continue;
        };
        if same_path(&target, root) {
            out.push((letter.to_string(), target));
        }
    }
    out
}

/// 目的：从 Z: 往 C: 找一个没人用的盘符（跳过 A:/B: 软盘传统字母）。
fn free_letter(occupied: impl Fn(&str) -> bool) -> Option<String> {
    LETTERS
        .into_iter()
        .find(|letter| !occupied(letter))
        .map(str::to_string)
}

/// 目的：规范化产品根：必须是绝对路径且存在，去掉 Windows 扩展前缀（\\\\?\\），统一成分量比较。
fn normalize_root(root: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(root)
        .map_err(|e| format!("产品根不可用（{}）：{}", root.display(), e))?;
    let text = canonical.to_string_lossy();
    let stripped = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let path = PathBuf::from(stripped);
    if !path.is_absolute() {
        return Err(format!("产品根不是绝对路径：{}", path.display()));
    }
    Ok(path)
}

/// 目的：逐分量比较两个绝对路径（Windows 大小写不敏感）。
fn same_path(a: &Path, b: &Path) -> bool {
    let ac: Vec<Component> = a.components().collect();
    let bc: Vec<Component> = b.components().collect();
    ac.len() == bc.len()
        && ac.iter().zip(bc.iter()).all(|(x, y)| {
            x.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&y.as_os_str().to_string_lossy())
        })
}

/// 目的：path 在 root 之下时的相对部分（逐分量、大小写不敏感）；不在其下给 None。
fn relative_under(root: &Path, path: &Path) -> Option<PathBuf> {
    let rc: Vec<Component> = root.components().collect();
    let pc: Vec<Component> = path.components().collect();
    if rc.is_empty() || pc.len() < rc.len() {
        return None;
    }
    for (r, p) in rc.iter().zip(pc.iter()) {
        if !r
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&p.as_os_str().to_string_lossy())
        {
            return None;
        }
    }
    let mut out = PathBuf::new();
    for c in &pc[rc.len()..] {
        out.push(c.as_os_str());
    }
    Some(out)
}

mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, GetLastError};
    use windows_sys::Win32::Storage::FileSystem::{
        DDD_EXACT_MATCH_ON_REMOVE, DDD_REMOVE_DEFINITION, DefineDosDeviceW, QueryDosDeviceW,
    };

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// 目的：盘符当前的映射目标（真实路径）；没有映射或查不到给 None。
    fn query_letter(letter: &str) -> Option<PathBuf> {
        let name = wide(letter);
        let mut buf = [0u16; 1024];
        let len = unsafe { QueryDosDeviceW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        if len == 0 {
            return None;
        }
        // 值是 MULTI_SZ：多个目标串以 NUL 分隔、最后再加一个 NUL；返回长度可能把结尾 NUL 算进来，
        // 所以按第一个 NUL 截断，别把 NUL 带进路径（带进去就永远比不中产品根）。
        let stored = &buf[..(len as usize).min(buf.len())];
        let end = stored.iter().position(|c| *c == 0).unwrap_or(stored.len());
        let raw = String::from_utf16_lossy(&stored[..end]);
        let stripped = raw.strip_prefix(r"\??\").unwrap_or(&raw);
        let text = stripped
            .strip_prefix(r"UNC\")
            .map(|rest| format!(r"\\{}", rest))
            .unwrap_or_else(|| stripped.to_string());
        if text.is_empty() {
            None
        } else {
            Some(PathBuf::from(text))
        }
    }

    /// 目的：真实产品根转 DefineDosDeviceW 的设备目标写法（\??\D:\…；UNC 加 UNC\ 前缀）。
    fn device_target(root: &Path) -> String {
        let text = root.to_string_lossy().replace('/', r"\");
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return format!(r"\??\{}", rest);
        }
        if let Some(rest) = text.strip_prefix(r"\\") {
            return format!(r"\??\UNC\{}", rest);
        }
        format!(r"\??\{}", text)
    }

    /// 目的：建映射（不校验是否被占用；调用方先查空）。
    fn define_mapping(letter: &str, root: &Path) -> Result<(), String> {
        let name = wide(letter);
        let target_text = device_target(root);
        let target = wide(&target_text);
        if unsafe { DefineDosDeviceW(0, name.as_ptr(), target.as_ptr()) } == 0 {
            let err = std::io::Error::last_os_error();
            return Err(format!(
                "盘符 {} 映射到 {} 失败：{}",
                letter,
                root.display(),
                err
            ));
        }
        Ok(())
    }

    /// 目的：撤映射；精确匹配目标，避免误撤别的程序刚建在同一字母上的映射。
    /// 约束：映射已经不在（竞态）按成功处理。
    fn remove_mapping(letter: &str, root: &Path) -> Result<(), String> {
        let name = wide(letter);
        let target_text = device_target(root);
        let target = wide(&target_text);
        let flags = DDD_REMOVE_DEFINITION | DDD_EXACT_MATCH_ON_REMOVE;
        if unsafe { DefineDosDeviceW(flags, name.as_ptr(), target.as_ptr()) } == 0 {
            let code = unsafe { GetLastError() };
            if code == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            return Err(format!(
                "撤盘符 {}（{}）失败：错误码 {}",
                letter,
                root.display(),
                code
            ));
        }
        Ok(())
    }

    fn pid_alive(pid: u32) -> bool {
        pid != 0 && crate::kernel::detail::confine::process_alive(pid)
    }

    /// 目的：清扫记录并尽力撤掉孤儿映射，返回留下的条目与被撤掉的盘符。
    /// 约束：调用方必须已持有 lock_view；撤不掉的条目以空 users 保留，留给下次清扫。
    fn sweep_locked(home: &Path) -> (Vec<ViewEntry>, Vec<String>) {
        let entries = load_entries(home);
        let sweep = sweep_entries(entries, pid_alive, query_letter, |path| path.is_dir());
        let mut kept = sweep.kept;
        let mut removed = Vec::new();
        for entry in sweep.remove {
            match remove_mapping(&entry.letter, Path::new(&entry.root)) {
                Ok(()) => removed.push(entry.letter),
                Err(e) => {
                    // 撤不掉就留下空归属条目，让下次启动清扫再试——不能凭空丢掉"这里有个映射"的事实。
                    eprintln!("[视图盘] {}", e);
                    kept.push(ViewEntry {
                        letter: entry.letter,
                        root: entry.root,
                        users: Vec::new(),
                    });
                }
            }
        }
        // 兜底：记录里没有的孤儿映射（崩溃 / 强杀绕过 release，映射还在、记录已空）。
        // 只撤目标正好是本产品根的字母——别的程序占的字母、别的实例的根、根内子目录一律不碰。
        if let Some(root) = home.parent().filter(|p| !p.as_os_str().is_empty()) {
            for (letter, target) in orphan_mappings(root, &kept, query_letter) {
                match remove_mapping(&letter, &target) {
                    Ok(()) => removed.push(letter),
                    Err(e) => eprintln!("[视图盘] {}", e),
                }
            }
        }
        (kept, removed)
    }

    /// 目的：只清扫记录、不分配盘符：摘死 pid 与目标不存在的条目、撤孤儿映射、落盘。
    pub(super) fn sweep(home: &Path) {
        let Ok(_lock) = lock_view(home) else {
            return;
        };
        let (kept, removed) = sweep_locked(home);
        if let Err(e) = save_entries(home, &kept) {
            eprintln!("[视图盘] 记录落盘失败：{}", e);
        }
        for letter in removed {
            clear_active(&letter);
        }
    }

    /// 目的：分配/复用视图盘：清扫记录、同根复用、找空字母建映射；成功设进程级 active。
    pub(super) fn ensure(root: &Path, home: &Path) -> Result<ViewDrive, String> {
        let root = normalize_root(root)?;
        let _lock = lock_view(home)?;
        let (mut kept, _removed) = sweep_locked(home);
        let pid = std::process::id();
        if let Some(pos) = kept
            .iter()
            .position(|e| same_path(Path::new(&e.root), &root))
        {
            let entry = &mut kept[pos];
            if !entry.users.contains(&pid) {
                entry.users.push(pid);
            }
            let letter = entry.letter.clone();
            save_entries(home, &kept)?;
            let view = ViewDrive { letter, root };
            set_active(&view);
            return Ok(view);
        }
        let Some(letter) = free_letter(|candidate| query_letter(candidate).is_some()) else {
            return Err("没有空闲盘符可作视图盘（请先释放被占用的盘符映射）".to_string());
        };
        define_mapping(&letter, &root)?;
        if !query_letter(&letter).is_some_and(|target| same_path(&target, &root)) {
            let mapped = query_letter(&letter);
            let _ = remove_mapping(&letter, &root);
            return Err(format!(
                "盘符 {} 建好后校验不上（映射目标 {:?} 与产品根 {} 不一致）",
                letter,
                mapped.map(|p| p.display().to_string()),
                root.display()
            ));
        }
        let mut entries = kept;
        entries.push(ViewEntry {
            letter: letter.clone(),
            root: root.to_string_lossy().into_owned(),
            users: vec![pid],
        });
        if let Err(e) = save_entries(home, &entries) {
            let _ = remove_mapping(&letter, &root);
            return Err(e);
        }
        let view = ViewDrive { letter, root };
        set_active(&view);
        Ok(view)
    }

    /// 目的：守门进程只挂现成视图盘：读记录、要求映射仍指向记录的产品根，不登记 pid。
    /// 约束：优先选 home 所属产品根的条目；找不到就退而取第一条映射有效的条目。
    pub(super) fn attach(home: &Path) -> Option<ViewDrive> {
        let entries = load_entries(home);
        let preferred = home.parent().filter(|p| p.is_dir()).map(Path::to_path_buf);
        let mut fallback: Option<ViewDrive> = None;
        for entry in entries {
            let Some(mapped) = query_letter(&entry.letter) else {
                continue;
            };
            let root = PathBuf::from(&entry.root);
            if !same_path(&mapped, &root) || !root.is_dir() {
                continue;
            }
            let view = ViewDrive {
                letter: entry.letter,
                root,
            };
            if let Some(pref) = &preferred
                && same_path(pref, view.root())
            {
                set_active(&view);
                return Some(view);
            }
            if fallback.is_none() {
                fallback = Some(view);
            }
        }
        if let Some(view) = &fallback {
            set_active(view);
        }
        fallback
    }

    /// 目的：注销本进程并回收没人用的映射（撤不掉就留下空归属条目给下次启动清扫）。
    pub(super) fn release(home: &Path) {
        let Ok(_lock) = lock_view(home) else {
            return;
        };
        let mut entries = load_entries(home);
        let pid = std::process::id();
        let mut changed = false;
        let mut removed_letters: Vec<String> = Vec::new();
        for entry in &mut entries {
            let Some(pos) = entry.users.iter().position(|p| *p == pid) else {
                continue;
            };
            entry.users.remove(pos);
            changed = true;
            if !entry.users.is_empty() {
                continue;
            }
            let mapping = query_letter(&entry.letter);
            let same = mapping
                .as_deref()
                .is_some_and(|target| same_path(target, Path::new(&entry.root)));
            if same && let Err(e) = remove_mapping(&entry.letter, Path::new(&entry.root)) {
                eprintln!("[视图盘] {}", e);
                continue;
            }
            removed_letters.push(entry.letter.clone());
        }
        entries.retain(|e| {
            !removed_letters
                .iter()
                .any(|letter| letter.eq_ignore_ascii_case(&e.letter))
        });
        if changed && let Err(e) = save_entries(home, &entries) {
            eprintln!("[视图盘] 记录落盘失败：{}", e);
        }
        for letter in removed_letters {
            clear_active(&letter);
        }
    }
}

/// 目的：只清扫视图盘记录、不分配盘符：摘死 pid 与目标不存在的条目、撤孤儿映射、落盘。
/// 约束：不登记本进程、不改 active。
pub fn sweep_view_drive(home: &Path) {
    imp::sweep(home)
}

/// 目的：分配（或复用）产品根的视图盘并设为进程级 active。
pub fn ensure_view_drive(root: &Path, home: &Path) -> Result<ViewDrive, String> {
    imp::ensure(root, home)
}

/// 目的：守门进程挂现成视图盘（读记录 + 校验映射），不分配、不登记 pid。
pub fn attach_view_drive(home: &Path) -> Option<ViewDrive> {
    imp::attach(home)
}

/// 目的：注销本进程的登记；该盘没有别的存活进程时撤映射并清条目。
pub fn release_view_drive(home: &Path) {
    imp::release(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(letter: &str, root: &str, users: &[u32]) -> ViewEntry {
        ViewEntry {
            letter: letter.to_string(),
            root: root.to_string(),
            users: users.to_vec(),
        }
    }

    #[test]
    fn view_paths_round_trip_and_respect_the_root() {
        // 用运行期临时目录构造产品根：源码里不写任何机器绝对路径。
        let root = std::env::temp_dir().join("solomni-view-root");
        let drive = ViewDrive {
            letter: "Z:".to_string(),
            root: root.clone(),
        };
        let host = root.join("session").join("test").join("test");
        let view = drive.view_of(&host).expect("根下路径要能译成视图写法");
        assert_eq!(view, PathBuf::from(r"Z:\session\test\test"));
        assert_eq!(drive.host_of(&view), Some(host.clone()));
        assert!(drive.is_view(&view));
        assert!(!drive.is_view(&host));
        // 根外、相对路径都不译。
        assert!(drive.view_of(&root.with_file_name("other")).is_none());
        assert!(drive.view_of(Path::new("session")).is_none());
        // 产品根本身 = 盘符根。
        assert_eq!(drive.view_of(drive.root()), Some(PathBuf::from(r"Z:\")));
        // 别的盘符不是这个视图。
        assert!(drive.host_of(Path::new(r"Y:\x")).is_none());
        // 大小写不敏感。
        let upper = PathBuf::from(root.to_string_lossy().to_uppercase()).join("a");
        assert_eq!(drive.view_of(&upper), Some(PathBuf::from(r"Z:\a")));
    }

    #[test]
    fn sweep_drops_dead_pids_and_stale_targets_but_keeps_live_ones() {
        let entries = vec![
            entry("Z:", r"D:\root", &[1, 2]),
            entry("Y:", r"D:\dead", &[3]),
            entry("X:", r"D:\stale", &[4]),
        ];
        let query = |letter: &str| match letter {
            "Z:" => Some(PathBuf::from(r"D:\root")),
            "Y:" => Some(PathBuf::from(r"D:\dead")),
            "X:" => Some(PathBuf::from(r"D:\stale-other")),
            _ => None,
        };
        let alive = |pid: u32| pid == 2;
        let exists = |path: &Path| path != Path::new(r"D:\dead");
        let sweep = sweep_entries(entries, alive, query, exists);
        assert_eq!(sweep.kept.len(), 1);
        assert_eq!(sweep.kept[0].users, vec![2], "死 pid 摘掉、活归属留着");
        // Y 根不存在要撤映射；X 的映射指向别处只丢条目、不撤别人的映射（不在 remove 里）。
        assert_eq!(sweep.remove.len(), 1);
        assert!(sweep.remove.iter().any(|e| e.letter == "Y:"));
        assert!(!sweep.remove.iter().any(|e| e.letter == "X:"));
    }

    #[test]
    fn sweep_keeps_entries_with_no_users_for_a_retry() {
        let entries = vec![entry("Z:", r"D:\root", &[])];
        let sweep = sweep_entries(
            entries,
            |_| true,
            |_| Some(PathBuf::from(r"D:\root")),
            |_| true,
        );
        assert_eq!(sweep.remove.len(), 1, "空归属要交给调用方撤映射");
    }

    #[test]
    fn orphan_mappings_only_reclaim_targets_under_the_product_root() {
        let query = |letter: &str| match letter {
            "Z:" => Some(PathBuf::from(r"D:\root")),
            "Y:" => Some(PathBuf::from(r"D:\root\sub")),
            "X:" => Some(PathBuf::from(r"C:\Temp")),
            "W:" => Some(PathBuf::from(r"D:\root2")),
            _ => None,
        };
        let orphans = orphan_mappings(Path::new(r"D:\root"), &[], query);
        assert_eq!(
            orphans,
            vec![("Z:".to_string(), PathBuf::from(r"D:\root"))],
            "只撤正好指向本产品根的映射：根内子目录 Y:、C:\\Temp、同前缀的 D:\\root2 都不算"
        );
        // 记录里还有活持有者的字母是在用的视图盘，不撤。
        let kept = vec![entry("Z:", r"D:\root", &[7])];
        assert!(orphan_mappings(Path::new(r"D:\root"), &kept, query).is_empty());
    }

    #[test]
    fn free_letter_skips_occupied_and_floppy_letters() {
        let occupied = |letter: &str| letter == "Z:" || letter == "Y:";
        assert_eq!(free_letter(occupied).as_deref(), Some("X:"));
        assert_eq!(free_letter(|_| true), None);
        assert!(free_letter(|_| false).is_some_and(|l| l == "Z:"));
    }
}
