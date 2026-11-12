//! 目的：入口的参数表——隐藏探针、维护动作与启动形态的分派只在这里写一次。
//! 管：取产品根之前的守门子进程与探针分派，以及取根之后的自检、依赖补齐、回收、构建与围栏台账。
//! 不管：产品根如何规范化、工具链如何就地应用、各能力如何装配——那些在 entry 的其它文件与 main。
//! 联动：`src/main.rs` 只按两段调用（取根前 / 取根后）；探针实现在 `src/entry/probes/`，守门在 `src/guard/`。

use std::path::Path;

/// 目的：取产品根之前的分派——只看 argv：守门子进程、隐藏探针、路由目录。
/// 参数：`args` 是完整 argv。
/// 返回：要退出就给退出码，`None` 表示继续启动。
pub(crate) fn before_root(args: &[String]) -> Option<i32> {
    // 长驻子进程守门模式（内部协议，用户不用）：macOS 没有 PR_SET_PDEATHSIG，由本程序自己盯核心，
    // 核心一死就把子进程组收掉（见 kernel::detail::procs::watch）。
    if let Some(i) = args
        .iter()
        .position(|a| a == crate::kernel::detail::procs::WATCHDOG_FLAG)
    {
        return Some(crate::guard::proc_watch(&args, i));
    }
    // 守门模式（内部协议，用户不用）：把围栏装好再跑模块声明的命令，退出码即工具退出码。
    if let Some(i) = args
        .iter()
        .position(|a| a == crate::kernel::detail::confine::FENCE_FLAG)
    {
        return Some(crate::guard::fence_run(&args, i));
    }
    // 隐藏模式：走**产品自己那套**出站链路打一次最小 HTTPS 请求，三态如实回报
    // （ok / no-net / tls-fail|fail）——CI 三平台据此验本构建的 TLS 栈，不需要任何密钥。
    if let Some(i) = args.iter().position(|a| a == "--https-check") {
        return Some(super::probes::https_check(&args, i));
    }
    // 隐藏模式：把归档解到目录（跨平台纯 Rust）——provision 能力用它解归档，不依赖用户装了什么解压工具。
    if let Some(i) = args.iter().position(|a| a == "--unpack") {
        return Some(super::probes::unpack_cli(&args, i));
    }
    // 环境白名单（机器可读）：把运行期交给工具进程的环境逐行交出来——探针据此在**同一个环境**里驱动
    // 守门进程，不另抄一份（抄一份会漂移，也会漏掉只有真实环境才暴露的问题）。
    if let Some(i) = args.iter().position(|a| a == "--print-fence-env") {
        return Some(super::probes::print_fence_env(&args, i));
    }
    // 机制验证（机器可读，探针与测试驱动）：不装围栏、不写任何权限项，只如实报"这次能不能强制住"。
    // 未授权时段的拒绝执行（见 capabilities/tools/detail/proc_tools.rs）就靠这一份结论。
    if let Some(i) = args.iter().position(|a| a == "--fence-verify") {
        return Some(super::probes::fence_verify(&args, i));
    }
    // 入站契约（机器可读）：HTTP 路由目录的唯一定义（见 docs/presentation/contracts.md）。
    if args.iter().any(|a| a == "--print-routes") {
        super::probes::print_routes();
        return Some(0);
    }
    None
}

/// 目的：取到产品根之后的分派——自检、依赖补齐、回收、构建与围栏台账。
/// 参数：`root` 是规范化后的产品根；`args` 是完整 argv。
/// 返回：要退出就给退出码，`None` 表示继续装配。
pub(crate) fn after_root(root: &Path, args: &[String]) -> Option<i32> {
    // 自检（机器可读）：把"这台机器能承载哪些测试"如实交出来——测试入口据此判定，不靠猜（见 TESTING.md）。
    // 放在根解析之后：它要按**产品根**读"项目内已装了什么"（与就绪判据同一份来源）。
    if args.iter().any(|a| a == "--doctor") {
        return Some(super::probes::doctor(root));
    }
    // 隐藏模式：依赖补齐的状态（机器可读，只报事实）——缺什么、装到哪、从哪来。
    if args.iter().any(|a| a == "--provision") {
        return Some(super::probes::provision(root, &args));
    }
    // 维护动作：显式回收一个工作里不再可达的内容对象（默认 dry-run，--yes 才删）。
    if let Some(i) = args
        .iter()
        .position(|a| a == "--work-gc" || a.starts_with("--work-gc="))
    {
        return Some(super::probes::work_gc(root, &args, i));
    }
    // 存储空间：报告与回收（隐藏维护入口；Web 设置页是同一份能力的另一种呈现）。
    if args.iter().any(|a| a == "--storage") {
        return Some(super::probes::storage(root));
    }
    if let Some(i) = args
        .iter()
        .position(|a| a == "--gc" || a.starts_with("--gc="))
    {
        return Some(super::probes::gc(root, &args, i));
    }
    // 隐藏模式：构建一个模块声明的产物（**系统动作**：在目标环境里跑 module.yaml 的 build.command）。
    if let Some(i) = args
        .iter()
        .position(|a| a == "--build" || a.starts_with("--build="))
    {
        return Some(super::probes::build(root, &args, i));
    }
    // 隐藏模式：按条处置围栏台账（列清单 / 还原一条 / 撤一条 / 删一个 profile）——比整体收尾更细，
    // 所以排在 --fence-clean 之前；两者共用同一份台账与同一套 ACE 读法（见 docs/kernel/README.md 的「围栏落点：必要与可选」）。
    if let Some(code) = crate::guard::fence_grant(&args, root) {
        return Some(code);
    }
    // 隐藏模式：精确回收围栏写过的权限项（不需要装配核心，也就不需要提示词册）。
    if args.iter().any(|a| a == "--fence-clean") {
        return Some(crate::guard::fence_clean(root));
    }
    None
}
