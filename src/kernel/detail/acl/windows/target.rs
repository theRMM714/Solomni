//! 目的：围栏要授权的落点清单——数据边界叶子、它们的父目录只读、可信执行基座与系统基线主体。
//! 管：GrantTarget 与 rw_part / grant_targets / is_trusted_base / system_baseline_sid。
//! 不管：ACE 怎么写（grant.rs）、写后怎么核对（verify.rs）。
//! 联动：confine 的 prepare_fence 用这份清单逐条授权（解释器目录由调用方算好、经 launch 解析）。

use super::*;
use crate::kernel::domain::layout;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows_sys::Win32::Security::PSID;

/// 目的：一个授权落点：**哪一环**（决定授不上时能不能降级）+ 路径 + 权限位 + 递归与继承。
/// 约束：把落点收成一个名字，prepare / release / 回滚与探针共用同一份形状。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GrantTarget {
    pub(crate) path: PathBuf,
    pub(crate) rights: u32,
    pub(crate) recursive: bool,
    pub(crate) inherit: bool,
    /// 目的：这一环是什么（必要 / 可选的判据只在 `FencePart::necessary`）。
    pub(crate) part: FencePart,
}

/// 目的：一个**写叶子**是哪一环：私有沙箱 / 模块目录与 cwd 之下 / 其余数据边界。
/// 约束：判据按策略侧派生的路径比（沙箱、模块目录、cwd 都不是猜出来的），三类**都是必要落点**——
///   区别只在给用户看时说不说得清缺的是哪一环。
fn write_part(plan: &FencePlan, path: &Path) -> FencePart {
    if !plan.home.as_os_str().is_empty() && path == plan.home {
        return FencePart::Sandbox;
    }
    let under =
        |root: &Path| !root.as_os_str().is_empty() && (path == root || path.starts_with(root));
    if under(&plan.cwd)
        || plan
            .grants
            .iter()
            .any(|g| g.access == Access::Read && g.reach == Reach::Tree && under(&g.path))
    {
        return FencePart::Module;
    }
    FencePart::DataBoundary
}

/// 目的：围栏要授权的全部落点：数据边界叶子（读写 / 用户授权的只读）+ **它们的父目录**（只读属性）。
///   父目录为什么要授：容器里对**中间目录**没有 FILE_READ_ATTRIBUTES 时，`exists()` / `stat()` 这类
///   常规判断会对一个**确实存在**的目录返回假。后果不是"读不到"，而是模块的"父目录不存在就先建"逻辑
///   以为整条链都不存在，一路向上建到盘卷根，撞出 `WinError 5 Access is denied: 'D:\'`
///   （真机 CI 上抓到的：Python 的 `os.makedirs(parent, exist_ok=True)`）。
///   只授"读属性"、**不递归、不继承**：容器能判断存在性，但读不到内容、列不了目录。
///   参数 root：视图盘生效时给产品根，祖先链只补到产品根（含）为止；产品根之外一级都不写——
///   cwd 走视图写法后，容器只需要映射根这一级的读属性。根外祖先缺 RA 的代价由视图盘消除，
///   而不是靠改写盘根下的大型目录的 DACL（顺整棵树重算继承，真机 193 s/条）。
///   工作目录本身不单独授权：它恒被某条只读子树或写子树覆盖（模块目录就是那条子树）。
pub(crate) fn grant_targets(plan: &FencePlan, root: Option<&Path>) -> Vec<GrantTarget> {
    let mut todo: Vec<GrantTarget> = Vec::new();
    let mut leaves: Vec<GrantTarget> = Vec::new();
    let leaf = |path: &Path, rights: u32, recursive: bool, part: FencePart| GrantTarget {
        path: path.to_path_buf(),
        rights,
        recursive,
        inherit: true,
        part,
    };
    // 私有沙箱：整棵子树可读写（工具进程的 HOME / TEMP 在这里）。
    if !plan.home.as_os_str().is_empty() {
        leaves.push(leaf(&plan.home, RIGHTS_RW, true, FencePart::Sandbox));
    }
    // 其余授予照意图逐条落实：写授予 → RIGHTS_RW，读授予 → RIGHTS_RO；
    // 落点形状 Dir 只这一层（不递归不继承），Tree 整棵子树（递归并继承）。
    for grant in &plan.grants {
        if grant.path.as_os_str().is_empty() {
            continue;
        }
        let recursive = grant.reach == Reach::Tree;
        let (rights, part) = match grant.access {
            Access::Write => (RIGHTS_RW, write_part(plan, &grant.path)),
            // 用户显式授权的主机只读根（`sandbox.policy.host_read_roots`）：只写只读 ACE，
            // **授给该 agent 自己的容器 SID**——不能授给共享组（那等于向任意容器程序开放用户数据）。
            Access::Read if !recursive => (RIGHTS_RO, FencePart::AuthorizedRead),
            // 只读子树（模块目录对 agent 只读）：必须递归可读（工具脚本在目录里）。
            Access::Read => (RIGHTS_RO, FencePart::Module),
        };
        leaves.push(leaf(&grant.path, rights, recursive, part));
    }
    // 父目录：只读属性、不递归、不继承（带 (OI)(CI) 的 ACE 会传播，撤权断链就污染整棵子树）；shell 的 cwd = home 时
    // 要逐级补祖先的 FILE_READ_ATTRIBUTES，这条链是机器级授权（授给本产品能力 SID、跨会话复用、对账不回收）。
    let deep = !plan.home.as_os_str().is_empty() && plan.cwd == plan.home;
    for one in &leaves {
        // 只有 shell 的 cwd/private 这个叶子需要祖先属性；运行包根等只给直接父目录。
        let deep_leaf = deep && (one.path == plan.cwd || one.path == plan.home);
        let mut parent = one.path.parent();
        while let Some(p) = parent {
            // 视图盘生效时根外祖先不写：已经越过产品根就停，不能碰盘根下大型目录的 DACL。
            if deep_leaf
                && let Some(root) = root
                && !(p == root || p.starts_with(root))
            {
                break;
            }
            // 盘根不写：ACL 写盘根会触发整盘继承重算（真机很慢），而盘根的读属性默认已对容器可用。
            if deep_leaf && p.parent().is_none() {
                break;
            }
            if !p.as_os_str().is_empty() {
                todo.push(GrantTarget {
                    path: p.to_path_buf(),
                    rights: RIGHTS_STAT,
                    recursive: false,
                    inherit: false,
                    part: FencePart::Parent,
                });
            }
            if !deep_leaf {
                break;
            }
            // 产品根本身也补（它就是映射根），到此为止：根外一级都不写。
            if root.is_some_and(|root| p == root) {
                break;
            }
            parent = p.parent();
        }
    }
    todo.extend(leaves);
    todo
}

/// 目的：路径是不是**可信执行基座**（产品内工具链 / 运行包）：只读二进制，不携带任何 scope 信息，
///   授给 baseline 一次即可，不参与按 reach 的容器身份与授权。
pub(crate) fn is_trusted_base(path: &Path, product_root: &Path) -> bool {
    let under = |base: &Path| path == base || path.starts_with(base);
    under(&product_root.join(".tools")) || under(&layout::runtimes(product_root))
}

/// 目的：**系统自带**只读基线的主体：ALL APPLICATION PACKAGES（S-1-15-2-1）。
///   只用来**认**系统目录（System32 等）上本来就有的那条 ACE——不是我们写的、也改不动，但它确实让容器读得到解释器。
///   我们自己写的机器级授权一律用能力 SID（container::product_capability_sid），不用这个组：
///   授给它等于把读权限下放给本机所有 AppContainer 应用。调用方负责 free_sid。
pub(crate) fn system_baseline_sid() -> Result<PSID, String> {
    let s: Vec<u16> = std::ffi::OsStr::new("S-1-15-2-1")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sid: PSID = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(s.as_ptr(), &mut sid) } == 0 || sid.is_null() {
        return Err("取不到 ALL APPLICATION PACKAGES 的 SID".to_string());
    }
    Ok(sid)
}
