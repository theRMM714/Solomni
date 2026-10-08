//! Windows 后端：AppContainer（文件系统与网络围栏）+ Job Object（进程树围栏）。
//! 机制：按 agent 派生一个容器 SID → 把「可达范围」逐条授权给它
//! （共享区与私有沙箱读写、模块目录读写、解释器安装目录只读+执行）
//! → 用 STARTUPINFOEX 的 SECURITY_CAPABILITIES 启动工具（**不给任何 capability = 默认断网**）。
//! 祖先目录不用授权：容器令牌自带 SeChangeNotifyPrivilege（绕过遍历检查），按名走到被放行的根不需要 FILE_TRAVERSE。
//! 授权只落在用户自己拥有的目录上（不需要管理员）；撤销用同一套机制反向做。
//! 授权与撤销由外层进程做（见 confine::prepare_fence），一次性做好并记在会话内存里；
//! 守门进程只负责"按同一个名字派生同一个 SID 并把工具放进去"。
//! 机制不可用时一律如实降级（stderr 说明 + 启动报告 fs/net=false），绝不假装有围栏。

use super::{shell_command, Capability, FencePrep, FenceVerdict, FENCE_FAILED};
use crate::capabilities::tools::api::FenceSpec;
use crate::capabilities::tools::domain::fence::FencePart;
use std::ffi::c_void;

mod acl;
mod container;
mod record;
pub(crate) use acl::*;
pub(crate) use container::*;
pub(crate) use record::*;
#[cfg(test)]
mod tests;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Security::{ACL, PSID};

/// 文件对象（SetNamedSecurityInfoW / GetNamedSecurityInfoW 的对象类型）。
pub(crate) const SE_FILE_OBJECT: i32 = 1;
// 权限位**只用具体位**：通用位（GENERIC_READ / WRITE / EXECUTE / ALL）的常量值极易记错，写错一个给出去的
// 就是完全不同的权限。注意：标着 GENERIC_READ 的是 0x4000_0000（其实是 GENERIC_WRITE）、
// 标着 GENERIC_EXECUTE 的是 0x1000_0000（其实是 GENERIC_ALL）——于是"只读"的解释器基线实际授出了全权。
pub(crate) const FILE_GENERIC_READ: u32 = 0x0012_0089;
pub(crate) const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
pub(crate) const FILE_GENERIC_EXECUTE: u32 = 0x0012_00A0;
pub(crate) const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
/// 目录里建/删子项（工具要能重写自己的产物）。
pub(crate) const FILE_DELETE_CHILD: u32 = 0x0000_0040;
pub(crate) const DELETE: u32 = 0x0001_0000;
pub(crate) const WRITE_DAC: u32 = 0x0004_0000;
/// 通用位：ACL 里存的可能是它们，也可能是内核展开后的具体位，比较覆盖关系时两者等价。
pub(crate) const GENERIC_READ: u32 = 0x8000_0000;
pub(crate) const GENERIC_WRITE: u32 = 0x4000_0000;
pub(crate) const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub(crate) const GENERIC_ALL: u32 = 0x1000_0000;

/// 数据边界（会话目录、模块目录）→ 读写 + 删子项 + 写 DACL（撤权要用）。
pub(crate) const RIGHTS_RW: u32 = FILE_GENERIC_READ
    | FILE_GENERIC_WRITE
    | FILE_GENERIC_EXECUTE
    | FILE_DELETE_CHILD
    | DELETE
    | WRITE_DAC;
/// 只读 + 执行（解释器安装目录：脚本要跑就得读得到它）。
pub(crate) const RIGHTS_RO: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
/// 只读属性：**能判断"这个目录在不在"**，但读不到内容、列不了目录。
/// 数据边界的父目录只授这一位（理由见 grant_targets）。
pub(crate) const RIGHTS_STAT: u32 = 0x0000_0080;

/// 一次工具执行最多这么多进程（含 shell 与它拉起的子进程）。
pub(crate) const MAX_PROCESSES: u32 = 32;

extern "system" {
    /// 递归给整棵树设 DACL：SDK 头文件在部分版本里标了废弃，但 advapi32 始终导出（本机已实测）。
    fn TreeSetNamedSecurityInfoW(
        object_name: *const u16,
        object_type: u32,
        security_info: u32,
        owner: PSID,
        group: PSID,
        dacl: *mut ACL,
        sacl: *mut ACL,
        action: u32,
        progress: *mut c_void,
        invoke: u32,
        args: *mut c_void,
    ) -> u32;
}

/// TREE_SEC_INFO_SET（递归设置）。
pub(crate) const TREE_SEC_INFO_SET: u32 = 1;
/// ProgressInvokeNever（不回调进度）。
pub(crate) const PROGRESS_INVOKE_NEVER: u32 = 1;

pub fn capability() -> Capability {
    match self_check() {
        Ok(()) => Capability {
            fs: true,
            net: true,
            tree: true,
            note: "Windows：AppContainer（文件系统可达范围 + 默认断网）与 Job Object（进程树）都由内核强制".to_string(),
        },
        Err(e) => Capability {
            fs: false,
            net: false,
            tree: true,
            note: format!("Windows：容器围栏装不上（{}）——只有进程树围栏与环境白名单，如实降级", e),
        },
    }
}

/// 目的：派生 SID 并真去改一个目录的 DACL，验证本机允许写权限。
/// 约束：改不动或清不干净都报 fs=false；profile 名在 PROFILE_PREFIX 之下，--fence-clean 覆盖得到它。
pub(crate) fn self_check() -> Result<(), String> {
    let sid = container_sid(&format!("{}FenceSelfCheck", PROFILE_PREFIX))?;
    let scratch =
        std::env::temp_dir().join(format!("solomni-fence-selfcheck-{}", std::process::id()));
    let outcome = self_check_scratch(sid, &scratch);
    free_sid(sid);
    outcome
}

/// 目的：在临时目录里做一次写→校验→还原→删除；任何一步清不掉都返回原因，不静默留痕。
fn self_check_scratch(sid: PSID, scratch: &Path) -> Result<(), String> {
    std::fs::create_dir_all(scratch).map_err(|e| format!("建自检目录失败：{}", e))?;
    let mut problems: Vec<String> = Vec::new();
    let bytes = match sd_bytes(scratch) {
        Ok(b) => b,
        Err(e) => {
            problems.push(format!("读自检目录安全描述符失败：{}", e));
            Vec::new()
        }
    };
    if let Err(e) = grant_verified(sid, scratch, RIGHTS_RO, false, false) {
        problems.push(format!("改不动目录 ACL：{}", e));
    }
    if !bytes.is_empty() {
        if let Err(e) = restore_sd(scratch, &bytes) {
            problems.push(format!("自检目录还原失败：{}", e));
        }
    }
    if let Err(e) = std::fs::remove_dir_all(scratch) {
        problems.push(format!("删自检目录失败：{}", e));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("；"))
    }
}

/// 本程序建的容器 profile 前缀（`container_name` 生成的就是它；`--fence-clean` 按它扫整族）。
pub(crate) const PROFILE_PREFIX: &str = "Solomni.Agent.";

/// 目的：判断授权落点是不是**不存在**（派生与执行之间有竞态时兜底；不存在就跳过，不判整次失败）。
fn missing(path: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
    )
}

/// 目的：把围栏要用的授权一次性做好（按 (SID, 路径, 权限) 去重，不重复改 ACL），并如实分出
///   **必要**落点（授不上就要问用户或拒绝）与**可选**落点（只记事实）。
/// 约束：每条授权先落台账再动 ACL，写后核对；失败就回滚并如实记进结论（不静默降级）。
pub fn prepare_fence(spec: &FenceSpec, command: &str, home: &Path) -> FencePrep {
    // 这一轮真正写下去的授权（用于如实打印足迹）。
    let mut written: Vec<(String, PathBuf, u32)> = Vec::new();
    let mut prep = FencePrep::default();
    // 产品根 = .home 的父目录：根内路径存原始安全描述符收尾还原，根外只记 ACE 摘要精确撤销。
    let root = product_root(home);
    let mut rec = load_record(home);
    let container = container_name(spec);
    if let Err(e) = journal_add_profile(home, &mut rec, &container) {
        // 台账落不下就不能动本机权限项（这一环是必要的）：如实收尾，让调用方去问用户。
        eprintln!("[围栏] 授权台账落盘失败：{}", e);
        prep.fail(FencePart::Ledger, PathBuf::new(), e);
        return prep;
    }
    // 基线都授给 ALL APPLICATION PACKAGES（与 agent 无关）：已有**够用**的 ACE 就跳过，第一次之后不再重走整棵树。
    let base = match baseline_sid() {
        Ok(sid) => sid,
        Err(e) => {
            prep.fail(FencePart::Interpreter, PathBuf::new(), e);
            return prep;
        }
    };
    let base_text = sid_to_string(base);
    let interpreters = interpreter_dirs(command);
    // 基线一：解释器安装目录（只读+执行）——**必要**：拿不到它，容器里连解释器都起不来。
    for dir in interpreters.iter().cloned() {
        if missing(&dir) {
            eprintln!("[诊断] 授权落点不存在，跳过（{}）", dir.display());
            continue;
        }
        if has_ace_for(base, &dir, RIGHTS_RO) {
            continue;
        }
        let target = GrantTarget {
            path: dir.clone(),
            rights: RIGHTS_RO,
            recursive: true,
            inherit: true,
            part: FencePart::Interpreter,
        };
        match grant_one_journaled(home, &mut rec, base, &base_text, &target, &root) {
            Ok(()) => written.push((base_text.clone(), dir, RIGHTS_RO)),
            Err(e) => {
                eprintln!("[围栏] 解释器目录授权未完成（{}）：{}", dir.display(), e);
                prep.fail(FencePart::Interpreter, dir, e);
            }
        }
    }
    // **祖先链不用授**：容器的令牌里有 SeChangeNotifyPrivilege（Bypass traverse checking，真机 whoami /priv
    // 确认 Enabled），按名走到被放行的根不需要祖先上的 FILE_TRAVERSE。"给祖先授穿过"那一趟只在改写
    // C:\、C:\Users 这种巨型目录的 DACL 时付出代价——Windows 会顺着整棵树重算继承，真机实测 ~90 s/条
    // （CI 上两条 ACL 契约测试各 95 s，就是它）。删掉这一趟：授权面更小，也不再碰产品目录之外的系统目录。
    free_sid(base);

    // 数据边界（会话目录、模块目录）→ 授给该 agent 自己的容器 SID（互相看不见）；
    // 落点清单由 grant_targets 统一给出（叶子 + 父目录的只读属性），prepare 与 release 共用同一份。
    let sid = match container_sid(&container) {
        Ok(sid) => sid,
        Err(e) => {
            // 派生不出容器身份就谈不上围栏：必要的一环，如实收尾（不再动任何权限项）。
            prep.fail(FencePart::ContainerIdentity, PathBuf::new(), e);
            return prep;
        }
    };
    let sid_text = sid_to_string(sid);
    let todo = grant_targets(spec);
    for target in todo {
        let (path, rights) = (&target.path, target.rights);
        // 落点不存在（例如没有 userdata 的模块，或授权面派生与执行之间有竞态）：跳过这一条，不判整次失败。
        if missing(path) {
            eprintln!("[诊断] 授权落点不存在，跳过（{}）", path.display());
            continue;
        }
        // 跳过条件看**实际 ACE**而不是内存缓存：权限收窄并撤权后，下一次 prepare 必须能把仍需要的授权补回来，
        // 否则「撤权 + 重授」会留下"缓存说已授、ACE 已撤"的空洞（扩根时被缓存吞掉）。
        if has_ace_for(sid, path, rights) {
            continue;
        }
        match grant_one_journaled(home, &mut rec, sid, &sid_text, &target, &root) {
            Ok(()) => written.push((sid_text.clone(), target.path.clone(), rights)),
            Err(e) => {
                // 按这一环的**必要性**分流：必要落点授不上要问用户（不许降级），可选落点只记事实。
                eprintln!("[围栏] 授权未完成：{}", e);
                prep.fail(target.part, target.path.clone(), e);
            }
        }
    }
    free_sid(sid);
    if !written.is_empty() {
        // 如实打印足迹：用户看得见到底动了哪些目录、授给了谁。
        let list: Vec<String> = written
            .iter()
            .map(|(s, p, _)| format!("{} → {}", s, p.display()))
            .collect();
        eprintln!("[围栏] 已写权限 {} 处：{}", written.len(), list.join("；"));
    }
    prep
}

/// 本机能不能强制住容器围栏（**不写任何目录 ACL**）：建容器 profile + 派生容器 SID 就是容器能起来的全部前提。
/// 三态：profile 建不起来（环境拒绝建）= 环境结论；profile 建得起来却派生不出 SID = 我们的步骤写错了。
pub fn verify(spec: &FenceSpec, _command: &str) -> FenceVerdict {
    let name = container_name(spec);
    if let Err(e) = ensure_profile(&name) {
        return FenceVerdict::EnvUnavailable(format!("容器 profile 建不起来：{}", e));
    }
    match container_sid(&name) {
        Ok(sid) => {
            free_sid(sid);
            FenceVerdict::Enforced
        }
        Err(e) => FenceVerdict::Broken(format!("容器 profile 已建起却派生不出容器 SID：{}", e)),
    }
}

pub fn run_fenced(spec: &FenceSpec, prepared: bool, home: Option<&Path>, command: &str) -> i32 {
    if let Err(e) = join_kill_on_close_job(MAX_PROCESSES) {
        eprintln!("[围栏] 进程树围栏安装失败：{}", e);
    }
    // 容器要先把读放行与落点做好（改本机目录 ACL）才可能真跑起来：外层没授权就直接按无围栏执行，
    // 不去试一个注定读不到模块目录与解释器的容器（那样只会把工具报成一堆"拒绝访问"）。
    if !prepared {
        eprintln!("[围栏] 外层未授权本机写入：容器围栏不可用，按如实降级继续执行");
        return run_unfenced(spec, command);
    }
    let name = container_name(spec);
    // 容器身份先立起来（profile 是容器能读到系统目录的前提）。
    if let Err(e) = ensure_profile(&name) {
        eprintln!("[围栏] {}{}）：按如实降级继续执行", ENV_BLOCKED_MARK, e);
        return run_unfenced(spec, command);
    }
    // 建过就记进台账：守门进程是唯一真正建 profile 的地方，外层只知道"该建"、不知道"建成了"。
    if let Some(h) = home {
        record_profile(h, &name);
    }
    let sid = match container_sid(&name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[围栏] 容器围栏未生效（{}）：按如实降级继续执行", e);
            return run_unfenced(spec, command);
        }
    };
    let outcome = run_in_container(sid, spec, command);
    free_sid(sid);
    match outcome {
        Ok(code) => {
            super::note_fence_boundary(spec, true, code);
            code
        }
        Err(e) => {
            // 容器起不来也要如实说清，并退回普通方式执行（能力等级已在启动报告里说明）。
            eprintln!("[围栏] 容器围栏未生效（{}）：按如实降级继续执行", e);
            run_unfenced(spec, command)
        }
    }
}

/// 无围栏执行：容器不可用（外层没授权、profile 建不起来、容器起不来）时的如实降级——
/// 命令仍交系统 shell 解释、cwd 仍是模块根，只是少了容器那层强制（启动报告里已说明能力等级）。
pub(crate) fn run_unfenced(spec: &FenceSpec, command: &str) -> i32 {
    match shell_command(command).current_dir(&spec.cwd).status() {
        Ok(s) => s.code().unwrap_or(FENCE_FAILED),
        Err(e) => {
            eprintln!("[围栏] 工具进程启动失败：{}", e);
            FENCE_FAILED
        }
    }
}
