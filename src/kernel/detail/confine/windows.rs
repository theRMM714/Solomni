//! 目的：Windows 后端——AppContainer（文件系统与网络围栏）+ Job Object（进程树围栏）。
//! 管：按 agent 派生容器 SID，逐条授权可达范围（沙箱/模块目录/解释器目录），再用 STARTUPINFOEX 启动工具（按 net 授/不授 internetClient）。
//! 不管：祖先目录不用授**穿过**（容器令牌自带 SeChangeNotifyPrivilege）；例外是 shell 的 cwd = private 时给祖先只读属性（PowerShell 解析 cwd 需要），不递归、不继承、不授内容。
//! 联动：授权与撤销由外层进程做（见 confine::windows::prepare_fence），守门进程只按同一个名字派生同一个 SID；机制不可用时一律如实降级。

use super::{Capability, FENCE_FAILED, FencePrep, FenceStep, FenceVerdict, shell_command};
use crate::kernel::contracts::fence::FencePlan;
use crate::kernel::detail::io::env;
use std::sync::OnceLock;
use std::time::Instant;

mod machine;
pub(crate) use crate::kernel::detail::acl::*;
pub(crate) use crate::kernel::detail::launch::*;
pub(crate) use crate::kernel::detail::ledger::*;
pub(crate) use machine::*;
#[cfg(test)]
pub(crate) mod tests;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Security::PSID;

/// 目的：把一个落点环节的失败如实记进这次准备（环节名与补法由 ACL 层的 `part.rs` 定，这里只转述）。
fn record(prep: &mut FencePrep, part: FencePart, path: PathBuf, why: String) {
    prep.fail(part.blocked(path, why), part.necessary());
}

/// 目的：一次工具执行最多这么多进程（含 shell 与它拉起的子进程）。
pub(crate) const MAX_PROCESSES: u32 = 32;

/// 目的：本机能力（首次调用真探一次，之后复用——探一次要写目录 ACL，并行用例各探一次会互相踩）。
pub fn capability() -> Capability {
    static PROBED: OnceLock<Capability> = OnceLock::new();
    PROBED.get_or_init(probe).clone()
}

fn probe() -> Capability {
    match self_check() {
        Ok(()) => Capability {
            fs: true,
            net: true,
            tree: true,
            note: "Windows：AppContainer（文件系统可达范围；net 决定是否授 internetClient）与 Job Object（进程树）都由内核强制".to_string(),
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
        env::temp_dir().join(format!("solomni-fence-selfcheck-{}", std::process::id()));
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
    if !bytes.is_empty()
        && let Err(e) = restore_sd(scratch, &bytes)
    {
        problems.push(format!("自检目录还原失败：{}", e));
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

/// 目的：判断授权落点是不是**不存在**（派生与执行之间有竞态时兜底；不存在就跳过，不判整次失败）。
fn missing(path: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
    )
}

/// 目的：分步计时的脱敏标签（固定词表；不写用户绝对路径）。
const STEP_INTERPRETER: &str = "解释器目录";
const STEP_ANCESTOR: &str = "祖先链";
const STEP_ROOT: &str = "受保护根";
const STEP_BASE: &str = "可信基座";

// 首次真写 ACL 前的 stderr 提示：同进程每类只提示一次（之后静默；纯命中不提示）。
static PROMPT_INTERPRETER: std::sync::Once = std::sync::Once::new();
static PROMPT_ANCESTOR: std::sync::Once = std::sync::Once::new();
static PROMPT_ROOT: std::sync::Once = std::sync::Once::new();
static PROMPT_BASE: std::sync::Once = std::sync::Once::new();

/// 目的：真写前提示一次「这一类落点第一次要写、可能要几十秒」——用户在等待时看得到原因。
fn prompt_first_write(once: &std::sync::Once, label: &str) {
    once.call_once(|| eprintln!("[围栏] 首次为{}补权限（可能要几十秒）…", label));
}

/// 目的：记下这一步真写 ACL 的耗时（只在真的改了 DACL 时产生步骤；命中已有 ACE 为零步骤）。
fn push_step(prep: &mut FencePrep, label: &str, started: Instant) {
    prep.steps.push(FenceStep {
        label: label.to_string(),
        ms: started.elapsed().as_millis() as u64,
    });
}

/// 目的：首次真写后的一条汇总（只报 ≥1 秒的类别；纯命中 / 总耗时 <1 秒保持安静）。
/// 约束：最多一行；类别固定词表，不含用户路径。
fn print_slow_summary(prep: &FencePrep) {
    let mut order: Vec<&str> = Vec::new();
    let mut totals: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for s in &prep.steps {
        let label = s.label.as_str();
        if !totals.contains_key(label) {
            order.push(label);
        }
        *totals.entry(label).or_insert(0) += s.ms;
    }
    let slow: Vec<String> = order
        .into_iter()
        .filter_map(|label| {
            let ms = totals.get(label).copied().unwrap_or(0);
            (ms >= 1000).then(|| format!("{} {} 秒", label, ms / 1000))
        })
        .collect();
    if !slow.is_empty() {
        eprintln!("[围栏] 写权限完成：{}", slow.join("、"));
    }
}

/// 目的：把围栏要用的授权一次性做好（按 (SID, 路径, 权限) 去重，不重复改 ACL），并如实分出
///   **必要**落点（授不上就要问用户或拒绝）与**可选**落点（只记事实）。
/// 约束：每条授权先落台账再动 ACL，写后核对；失败就回滚并如实记进结论（不静默降级）。归属分三种：
///   容器数据边界按本次调用记（收尾可精确撤）；祖先链的只读属性、可信执行基座是**机器级**——授给本产品能力 SID、
///   跨会话复用（见 is_machine_target / is_trusted_base 与 record 的 GrantScope）。
///   台账锁只罩「读—改—写」（record/store.rs 的 with_ledger）：慢的 ACL 写在锁外，并发调用不互相排队。
pub fn prepare_fence(spec: &FencePlan, command: &str, root: &Path) -> FencePrep {
    // 这一轮真正写下去的授权（用于如实打印足迹）。
    let mut written: Vec<(String, PathBuf, u32)> = Vec::new();
    let mut prep = FencePrep::default();
    // 产品根 = 产品安装根：根内路径存原始安全描述符收尾还原，根外只记 ACE 摘要精确撤销。
    let product = product_root(root);
    // 台账事务只罩「读—改—写」：先确认台账拿得到（拿不到就整次如实收尾），之后每条授权各自加锁，
    // 慢的 ACL 写不占锁——同进程并发调用不会因某一条 193 秒的整树重算互相排队。
    if let Err(e) = with_ledger(root, |_| Ok::<(), String>(())) {
        eprintln!("[围栏] 授权台账落盘失败：{}", e);
        record(&mut prep, FencePart::Ledger, PathBuf::new(), e);
        return prep;
    }
    // 容器数据授权带本次会话的租约：同名 agent 的多个并发会话各持一份，释放时互不影响。
    let mut container_owner = current_owner();
    container_owner.lease = spec.lease.clone();
    // 解释器基线不属于任何会话（全应用包共享组）：进程级归属。
    let base_owner = current_owner();
    let container = container_name(spec);
    // profile 名先有主：这条落不下就不能动本机权限项。
    if let Err(e) = with_ledger(root, |rec| {
        journal_add_profile(root, rec, &container, container_owner.clone())
    }) {
        eprintln!("[围栏] 授权台账落盘失败：{}", e);
        record(&mut prep, FencePart::Ledger, PathBuf::new(), e);
        return prep;
    }
    // 基线都授给本产品的能力 SID（与 agent 无关）：已有**够用**的 ACE 就跳过，第一次之后不再重走整棵树。
    let base_owned = match product_capability_sid() {
        Ok(sid) => sid,
        Err(e) => {
            record(&mut prep, FencePart::Interpreter, PathBuf::new(), e);
            return prep;
        }
    };
    let base = base_owned.as_ptr() as PSID;
    let base_text = sid_to_string(base);
    let interpreters = interpreter_dirs(command);
    // 基线一：解释器安装目录（只读+执行）——**必要**：拿不到它，容器里连解释器都起不来。
    for dir in interpreters.iter().cloned() {
        if missing(&dir) {
            eprintln!("[诊断] 授权落点不存在，跳过（{}）", dir.display());
            continue;
        }
        // 基线的 ACE 可能**早已存在**（系统目录上常有全应用包的允许项）：那不是我们写的，记进台账会让收尾
        // 去撤系统目录（拒绝访问）。已存在就跳过，不记归属、也不改 DACL。两个主体都认（见 interpreter_baseline_present）。
        if interpreter_baseline_present(&dir).unwrap_or(false) {
            continue;
        }
        // 预检即终局：这一条授权写不进去时，连尝试都不做——不产生"写一半/回滚"的状态。
        if let Err(why) = interpreter_dir_usable(&dir) {
            eprintln!("[围栏] 解释器目录不可授权（{}）：{}", dir.display(), why);
            record(&mut prep, FencePart::Interpreter, dir, why);
            continue;
        }
        let target = GrantTarget {
            path: dir.clone(),
            rights: RIGHTS_RO,
            recursive: true,
            inherit: true,
            part: FencePart::Interpreter,
        };
        prompt_first_write(&PROMPT_INTERPRETER, STEP_INTERPRETER);
        let started = Instant::now();
        match grant_one_journaled(
            root,
            base,
            &base_text,
            &target,
            &product,
            base_owner.clone(),
        ) {
            Ok(true) => {
                push_step(&mut prep, STEP_INTERPRETER, started);
                written.push((base_text.clone(), dir, RIGHTS_RO));
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!("[围栏] 解释器目录授权未完成（{}）：{}", dir.display(), e);
                record(&mut prep, FencePart::Interpreter, dir, e);
            }
        }
    }
    // **祖先链不用授穿过**：令牌自带 SeChangeNotifyPrivilege；给祖先授穿过要改写巨型目录 DACL（真机 ~90 s/条）。
    // shell 例外只补 RIGHTS_STAT；视图盘生效时这条链止于产品根（见 grant_targets 的 product 参数）；base 在数据边界循环里还要用。
    let sid = match container_sid(&container) {
        Ok(sid) => sid,
        Err(e) => {
            // 派生不出容器身份就谈不上围栏：必要的一环，如实收尾（不再动任何权限项）。
            record(&mut prep, FencePart::ContainerIdentity, PathBuf::new(), e);
            return prep;
        }
    };
    let sid_text = sid_to_string(sid);
    // 容器临时目录：AppContainer 的 GetTempPath2 指到 %LOCALAPPDATA%\Packages\<容器名>\AC\Temp（不读 TMP/TEMP），
    // 不建它编译器连临时文件都写不了；先建后授权——它在 rw 之下，下面的递归 ACE 会覆盖到它，不必单列授权。
    if let Some(temp) = container_temp_dir(spec)
        && let Err(e) = std::fs::create_dir_all(&temp)
    {
        eprintln!(
            "[围栏] 容器临时目录建不出来（{}）：{}（这次执行里用临时文件的命令会自己报错）",
            temp.display(),
            e
        );
    }
    // 视图盘只对 shell 这类 cwd == 私有沙箱的调用生效：祖先链只补到产品根（含）一级，根外一级都不写（那 193 秒）。
    // 模块工具的 cwd 是模块根，不走视图盘；它本来就不算深叶（deep 只对 cwd == private 成立），只补一条直接父目录 STAT。
    let shell_view = crate::kernel::detail::view_drive::active_shell_view(spec).is_some();
    let plan_root = shell_view.then_some(product.as_path());
    let todo = grant_targets(spec, plan_root);
    // 本轮写下去的机器级授权条数（祖先链 + 可信基座；只用于在"已写权限"行后补一句去向说明）。
    let mut machine_written = 0usize;
    for target in todo {
        let (path, rights) = (&target.path, target.rights);
        // 落点不存在（例如没有 userdata 的模块，或授权面派生与执行之间有竞态）：跳过这一条，不判整次失败。
        if missing(path) {
            eprintln!("[诊断] 授权落点不存在，跳过（{}）", path.display());
            continue;
        }
        // 祖先链的只读属性是**机器级**授权：授给本产品能力 SID（与容器 SID、会话都无关）。
        // 已有 ACE 就直接跳过——这正是后续会话"零写"的来源；不命中才落台账并写一次。
        if is_machine_target(&target) {
            if has_ace_for(base, path, rights) {
                continue;
            }
            prompt_first_write(&PROMPT_ANCESTOR, STEP_ANCESTOR);
            let started = Instant::now();
            match grant_machine_journaled(root, base, &base_text, &target, &product) {
                Ok(true) => {
                    machine_written += 1;
                    push_step(&mut prep, STEP_ANCESTOR, started);
                    written.push((base_text.clone(), target.path.clone(), rights));
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("[围栏] 机器级授权未完成（{}）：{}", path.display(), e);
                    record(&mut prep, target.part, target.path.clone(), e);
                }
            }
            continue;
        }
        // 工具链 / 运行包是**可信执行基座**：只读、无 scope 信息，与祖先链同属机器级授权
        // （授给本产品能力 SID、台账 scope=machine），跨会话复用、启动对账不回收。
        let to_base = rights == RIGHTS_RO && is_trusted_base(path, &product);
        if to_base {
            if !has_ace_for(base, path, rights) {
                prompt_first_write(&PROMPT_BASE, STEP_BASE);
            }
            let started = Instant::now();
            match grant_machine_journaled(root, base, &base_text, &target, &product) {
                Ok(true) => {
                    machine_written += 1;
                    push_step(&mut prep, STEP_BASE, started);
                    written.push((base_text.clone(), target.path.clone(), rights));
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("[围栏] 可信基座授权未完成（{}）：{}", path.display(), e);
                    record(&mut prep, target.part, target.path.clone(), e);
                }
            }
            continue;
        }
        // 其余落点授给本次调用的容器 SID（按 reach 收窄）；归属总要补记（同名 agent 的另一个会话靠它保住这条授权）。
        // 改不改 ACL 由 grant_one_journaled 按**实际 ACE** 判断：权限收窄并撤权后，下一次 prepare 必须能把仍需要的授权补回来。
        if !has_ace_for(sid, path, rights) {
            prompt_first_write(&PROMPT_ROOT, STEP_ROOT);
        }
        let started = Instant::now();
        match grant_one_journaled(
            root,
            sid,
            &sid_text,
            &target,
            &product,
            container_owner.clone(),
        ) {
            Ok(true) => {
                push_step(&mut prep, STEP_ROOT, started);
                written.push((sid_text.clone(), target.path.clone(), rights));
            }
            Ok(false) => {}
            Err(e) => {
                // 按这一环的**必要性**分流：必要落点授不上要问用户（不许降级），可选落点只记事实。
                eprintln!("[围栏] 授权未完成：{}", e);
                record(&mut prep, target.part, target.path.clone(), e);
            }
        }
    }
    check_integrity_labels(spec, &mut prep);
    free_sid(sid);
    if !written.is_empty() {
        // 如实打印足迹：用户看得见到底动了哪些目录、授给了谁。
        let list: Vec<String> = written
            .iter()
            .map(|(s, p, _)| format!("{} → {}", s, p.display()))
            .collect();
        eprintln!("[围栏] 已写权限 {} 处：{}", written.len(), list.join("；"));
    }
    if machine_written > 0 {
        // 机器级授权的去向单独交代：它们不随会话回收，之后每次调用都不必再写。
        eprintln!(
            "[围栏] 其中机器级授权 {} 处：授给本产品能力 SID（跨会话复用，启动对账不回收）",
            machine_written
        );
    }
    print_slow_summary(&prep);
    prep
}

/// 目的：这条落点是不是**机器级**授权——祖先链的只读属性，授给本产品能力 SID，跨会话复用。
/// 约束：只认 grant_targets 给祖先链的唯一形状（part = Parent、只授 RIGHTS_STAT、不递归、不继承）；
///   可信基座也走机器级归属（在 grant_targets 循环里按 is_trusted_base 判），解释器基线、数据边界仍按本次调用归属。
pub(crate) fn is_machine_target(target: &GrantTarget) -> bool {
    target.part == FencePart::Parent
        && target.rights == RIGHTS_STAT
        && !target.recursive
        && !target.inherit
}

/// 目的：授权后核对完整性标签——DACL 给对了也不代表容器写得进，高于 Low 的「不许向上写」标签会另行挡写。
/// 约束：只读、不递归，只看**授写叶子本身**：写一个对象只对它做完整性检查，祖先只被穿过（不算写），
///   所以祖先带标签不挡叶子；叶子继承到的标签会出现在它自己的 SACL 里，读叶子就够。
///   命中按必要落点走 FencePrep::fail（裁决 / fail-closed），不静默报成功。
fn check_integrity_labels(spec: &FencePlan, prep: &mut FencePrep) {
    for leaf in &spec.write_roots() {
        if leaf.as_os_str().is_empty() || missing(leaf) {
            continue;
        }
        match integrity_label(leaf) {
            Ok(label) => {
                if let Some(rid) = blocking_integrity_rid(&label) {
                    let why = format!(
                        "这个目录带 {} 完整性标签且不许向上写，容器令牌是 Low，DACL 授权再对也写不进",
                        integrity_name(rid)
                    );
                    eprintln!("[围栏] 完整性标签挡写（{}）：{}", leaf.display(), why);
                    record(prep, FencePart::IntegrityLabel, leaf.clone(), why);
                }
            }
            Err(e) => {
                eprintln!("[围栏] 完整性标签读不出来（{}）：{}", leaf.display(), e);
                record(prep, FencePart::IntegrityLabel, leaf.clone(), e);
            }
        }
    }
}

/// 目的：本机能不能强制住容器围栏（**不写任何目录 ACL**）：建容器 profile + 派生容器 SID 是容器能起来的全部前提；
///   `net=true` 还要能建出 internetClient 能力 SID（建不出就是我们写错了，不静默按断网/无围栏跑）。
///   三态：profile 建不起来（环境拒绝建）= 环境结论；profile 建得起来却派生不出 SID / 能力 = 我们的步骤写错了。
/// 目的：本次实际生效的路径级围栏——Windows 的容器围栏依赖外层写 ACL 的授权。
pub fn effective_paths(fs: bool, net: bool, write_allowed: bool) -> (bool, bool) {
    if write_allowed {
        (fs, net)
    } else {
        (false, false)
    }
}

pub fn verify(spec: &FencePlan, _command: &str) -> FenceVerdict {
    let name = container_name(spec);
    if let Err(e) = ensure_profile(&name) {
        return FenceVerdict::EnvUnavailable(format!("容器 profile 建不起来：{}", e));
    }
    if spec.net.on()
        && let Err(e) = internet_client_sid(true)
    {
        return FenceVerdict::Broken(format!("internetClient 能力建不起来：{}", e));
    }
    match container_sid(&name) {
        Ok(sid) => {
            free_sid(sid);
            FenceVerdict::Enforced
        }
        Err(e) => FenceVerdict::Broken(format!("容器 profile 已建起却派生不出容器 SID：{}", e)),
    }
}

pub fn run_fenced(
    spec: &FencePlan,
    prepared: bool,
    root: Option<&Path>,
    command: &str,
    policy: &crate::kernel::contracts::fence::FenceRunPolicy,
) -> i32 {
    if let Err(e) = join_kill_on_close_job(MAX_PROCESSES) {
        eprintln!("[围栏] 进程树围栏安装失败：{}", e);
    }
    let strict = (policy.require_fs || policy.require_net) && !policy.allow_degraded;
    // 回环中继档同样不许降级：一降级就没有容器，等于白送整个宿主回环（比断网还宽）。
    let local_ports = spec.net.local_ports().map(|p| p.to_vec());
    let hard = strict || local_ports.is_some();
    let refuse = |why: &str| -> i32 {
        eprintln!("[围栏] {}；按全局策略拒绝执行", why);
        FENCE_FAILED
    };
    // 容器要先把读放行与落点做好（改本机目录 ACL）才可能真跑起来：外层没授权就直接按无围栏执行，
    // 不去试一个注定读不到模块目录与解释器的容器（那样只会把工具报成一堆"拒绝访问"）。
    if !prepared {
        if hard {
            return refuse("外层未授权本机写入，容器围栏不可用");
        }
        eprintln!("[围栏] 外层未授权本机写入：容器围栏不可用，按如实降级继续执行");
        return run_unfenced(spec, command);
    }
    // 守门进程：挂现成视图盘（读记录 + 校验映射；不分配、不登记 pid），容器 cwd 才会写成盘符路径。
    // 已在本进程生效时是幂等的：只是重设同一个 active 视图。
    if let Some(h) = root {
        let _ = crate::kernel::detail::view_drive::attach_view_drive(h);
    }
    let name = container_name(spec);
    // 容器身份先立起来（profile 是容器能读到系统目录的前提）。
    if let Err(e) = ensure_profile(&name) {
        if hard {
            return refuse(&format!("{}{}）", ENV_BLOCKED_MARK, e));
        }
        eprintln!("[围栏] {}{}）：按如实降级继续执行", ENV_BLOCKED_MARK, e);
        return run_unfenced(spec, command);
    }
    // 建过就记进台账：守门进程是唯一真正建 profile 的地方，外层只知道"该建"、不知道"建成了"。
    if let Some(h) = root {
        record_profile(h, &name, &spec.lease);
    }
    let sid = match container_sid(&name) {
        Ok(s) => s,
        Err(e) => {
            if hard {
                return refuse(&format!("容器围栏未生效（{}）", e));
            }
            eprintln!("[围栏] 容器围栏未生效（{}）：按如实降级继续执行", e);
            return run_unfenced(spec, command);
        }
    };
    let outcome = match &local_ports {
        Some(ports) => relay_run(sid, spec, command, ports),
        None => run_in_container(sid, spec, command),
    };
    free_sid(sid);
    match outcome {
        Ok(code) => code,
        Err(e) => {
            // 容器起不来要如实说清；net=Public 时不能退回无围栏（那样会连文件系统围栏一起丢掉），直接拒绝。
            eprintln!("[围栏] 容器围栏未生效（{}）", e);
            if hard {
                eprintln!(
                    "[围栏] 本次要求放行出站网络（或只放行指定回环端口）或强制围栏，但能力/容器授不上：拒绝执行（不按无围栏跑）"
                );
                return FENCE_FAILED;
            }
            eprintln!("[围栏] 按如实降级继续执行");
            run_unfenced(spec, command)
        }
    }
}

/// 目的：无围栏执行：容器不可用（外层没授权、profile 建不起来、容器起不来）时的如实降级——
///   命令仍交系统 shell 解释、cwd 仍是模块根，只是少了容器那层强制（启动报告里已说明能力等级）。
pub(crate) fn run_unfenced(spec: &FencePlan, command: &str) -> i32 {
    match shell_command(command).current_dir(&spec.cwd).status() {
        Ok(s) => s.code().unwrap_or(FENCE_FAILED),
        Err(e) => {
            eprintln!("[围栏] 工具进程启动失败：{}", e);
            FENCE_FAILED
        }
    }
}

/// 目的：本平台后端（AppContainer 容器围栏 + 目录 ACL 授权）。
pub(crate) struct Backend;

impl super::Confine for Backend {
    fn capability(&self) -> super::Capability {
        capability()
    }

    fn verify(&self, plan: &FencePlan, command: &str) -> super::FenceVerdict {
        verify(plan, command)
    }

    fn effective_paths(&self, fs: bool, net: bool, write_allowed: bool) -> (bool, bool) {
        effective_paths(fs, net, write_allowed)
    }

    /// 目的：本平台有回环中继（容器内 bind 127.0.0.2，宿主按端口清单 dial 127.0.0.1）。
    fn local_relay(&self) -> bool {
        true
    }

    fn run(
        &self,
        plan: &FencePlan,
        prepared: super::Prepared,
        command: &str,
        policy: &crate::kernel::contracts::fence::FenceRunPolicy,
    ) -> i32 {
        run_fenced(plan, prepared.prepared, prepared.root, command, policy)
    }
}
