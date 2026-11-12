//! 目的：工作区根的策略——按档位判一条外部工作区根能不能用，并在会话重开时用同一套判定复检。
//! 管：网络路径与档位的匹配（本机档拒绝网络工作区）、根的形态（不能是文件系统根 / UNC 共享根）、虚拟机档要求
//!   UTF-8、与产品根的互包含（文本 + 落点身份），以及目录与可写性。
//! 不管：会话 meta 的读写（在 session）；锁（在 workspace 能力）；挂载计划（在 guest_job）。
//! 联动：创建走 resolve，加载复检走 recheck；两者共用一个 policy，手改 meta 也过不去；规范化、元数据与可写探测都走 `HostProbe`。

use crate::kernel::api::Tier;
use crate::kernel::ports::HostProbe;
use std::path::{Path, PathBuf};

/// 目的：把产品根规范化成判定用的写法（规范化失败时退回原样）。
pub(crate) fn product_root(root: &Path, probe: &dyn HostProbe) -> PathBuf {
    crate::kernel::api::native(
        &probe
            .canonicalize(root)
            .unwrap_or_else(|_| root.to_path_buf()),
    )
}

/// 目的：不需要 IO 的那部分策略——绝对路径、档位与网络路径、根的形态、虚拟机档的编码。
/// 约束：本机档的网络路径在 canonicalize **之前**就拒绝——不为了报一条错去连一次共享。canonicalize 之后
///   再跑一遍：目录联接与映射盘的真身要到那时才露出来。
pub(crate) fn policy(path: &Path, tier: Tier) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("工作区必须是绝对路径：{}", path.display()));
    }
    if tier == Tier::Host && crate::kernel::api::is_network_path(path) {
        return Err(host_network(path));
    }
    if crate::kernel::api::is_unc(path)
        && let Some((_, _, rest)) = crate::kernel::api::unc_parts(path)
        && rest.as_os_str().is_empty()
    {
        return Err(format!(
            "工作区不能是网络共享的根目录：{}（请绑到共享下的具体目录）",
            path.display()
        ));
    }
    if path.parent().is_none() {
        return Err("工作区不能是文件系统根目录".to_string());
    }
    // 虚拟机档要把根编进 guest 写法：非 UTF-8 会在 to_string_lossy 下丢字节，单射不成立。
    if tier == Tier::Vm && path.to_str().is_none() {
        return Err(format!(
            "虚拟机档的工作区必须是 UTF-8 路径：{}",
            path.display()
        ));
    }
    Ok(())
}

/// 目的：创建时的完整校验——策略 → 存在且是目录 → 与产品根互不包含 → 可写。
/// 返回：规范化后的根（native 写法，绑定与投影都按它存）。
pub(crate) fn resolve(
    raw: &Path,
    tier: Tier,
    product: &Path,
    probe: &dyn HostProbe,
) -> Result<PathBuf, String> {
    policy(raw, tier)?;
    let root = crate::kernel::api::native(
        &probe
            .canonicalize(raw)
            .map_err(|e| format!("工作区不可达：{}（{}）", raw.display(), e))?,
    );
    policy(&root, tier)?;
    let meta = probe
        .host_metadata(&root)
        .map_err(|e| format!("读工作区失败：{}（{}）", root.display(), e))?;
    if !meta.is_dir {
        return Err(format!("工作区不是目录：{}", root.display()));
    }
    check_bounds(&root, product)?;
    // 写探测：外部目录必须真的可写；探针由端口在根目录下建删。
    probe
        .probe_writable(&root)
        .map_err(|_| format!("工作区不可写：{}", root.display()))?;
    Ok(root)
}

/// 目的：加载复检——会话记录里的绑定按 meta 的档位重跑策略。
/// 约束：目录暂时不可达不算策略违例（会话仍能打开，工作区操作按真实原因报错）；越界、网络路径进本机档、
///   共享根、非 UTF-8 都如实拒绝——否则手改 meta 就能绕过创建时的拦截。
pub(crate) fn recheck(
    root: &Path,
    tier: Tier,
    product: &Path,
    probe: &dyn HostProbe,
) -> Result<(), String> {
    policy(root, tier)?;
    if probe.host_metadata(root).is_err() {
        return Ok(());
    }
    check_bounds(root, product)
}

/// 目的：本机档的网络工作区——说清为什么不行（容器围栏只放互联网、文件围栏要递归改共享权限）。
fn host_network(path: &Path) -> String {
    format!(
        "本机档不能用网络工作区：{}（Windows 容器围栏访问不了私有网络与回环，文件围栏还会递归改共享的权限；把工作区放到本机目录，或改用虚拟机档）",
        path.display()
    )
}

/// 目的：外部根与产品根互不包含——文本一次，落点身份再一次（UNC 别名、映射盘、8.3 短名都骗不过身份）。
fn check_bounds(root: &Path, product: &Path) -> Result<(), String> {
    let text = root == product || product.starts_with(root) || root.starts_with(product);
    let place = crate::kernel::api::place_contains(root, product)
        || crate::kernel::api::place_contains(product, root);
    if text || place {
        return Err("工作区不能与产品目录互相包含（会越过会话 / 模块 / 配置的边界）".to_string());
    }
    Ok(())
}
