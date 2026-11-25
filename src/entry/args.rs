//! 目的：入口的内部协议分派——守门子进程、机器可读探针与围栏台账；产品命令不在这里。
//! 管：取产品根前后的两段字面量分派；只认**内部协议**旗语（不进命令表，也不进 --help）。
//! 不管：产品命令的执行（在 presentation/cli/boot.rs 与 system 单元）、根规范化、装配（在 main 与 entry 的其它文件）。
//! 联动：`src/main.rs` 先分派启动相命令、再按两段调用本文件；探针实现在 `src/entry/probes/`，守门在 `src/guard.rs`。

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
        return Some(crate::guard::proc_watch(args, i));
    }
    // 守门模式（内部协议，用户不用）：把围栏装好再跑模块声明的命令，退出码即工具退出码。
    if let Some(i) = args
        .iter()
        .position(|a| a == crate::kernel::detail::confine::FENCE_FLAG)
    {
        return Some(crate::guard::fence_run(args, i));
    }
    // 回环中继（内部协议，只在容器里跑）：把一个容器身份下要连的本机端口沿继承来的管道路由到宿主，再起真正的命令。
    // 排在围栏旗之后：它是围栏的一个档（net=local），不是独立玩法。
    if let Some(i) = args
        .iter()
        .position(|a| a == crate::kernel::detail::container::RELAY_FLAG)
    {
        return Some(crate::kernel::detail::container::relay_entry(
            &args[i + 1..],
        ));
    }
    // 隐藏模式：走**产品自己那套**出站链路打一次最小 HTTPS 请求，三态如实回报
    // （ok / no-net / tls-fail|fail）——CI 三平台据此验本构建的 TLS 栈，不需要任何密钥。
    if let Some(i) = args.iter().position(|a| a == "--https-check") {
        return Some(super::probes::https_check(args, i));
    }
    // 隐藏模式：把归档解到目录（跨平台纯 Rust）——provision 单元用它解归档，不依赖用户装了什么解压工具。
    if let Some(i) = args.iter().position(|a| a == "--unpack") {
        return Some(super::probes::unpack_cli(args, i));
    }
    // 环境白名单（机器可读）：把运行期交给工具进程的环境逐行交出来——探针据此在**同一个环境**里驱动
    // 守门进程，不另抄一份（抄一份会漂移，也会漏掉只有真实环境才暴露的问题）。
    if let Some(i) = args.iter().position(|a| a == "--print-fence-env") {
        return Some(super::probes::print_fence_env(args, i));
    }
    // 机制验证（机器可读，探针与测试驱动）：不装围栏、不写任何权限项，只如实报"这次能不能强制住"。
    // 未授权时段的拒绝执行（见 units/tools/detail/proc_tools.rs）就靠这一份结论。
    if let Some(i) = args.iter().position(|a| a == "--fence-verify") {
        return Some(super::probes::fence_verify(args, i));
    }
    // 授权预写（探针与手工排查）：只把它写进本机、不执行命令，好让「授权」与「执行」分两步验。
    if let Some(i) = args.iter().position(|a| a == "--fence-prepare") {
        return Some(super::probes::fence_prepare(args, i));
    }
    // 入站契约（机器可读）：HTTP 路由目录的唯一定义（见 docs/presentation/contracts.md）。
    if args.iter().any(|a| a == "--print-routes") {
        super::probes::print_routes();
        return Some(0);
    }
    None
}

/// 目的：取到产品根之后的分派——内部协议的机器可读探针与围栏台账（产品命令已由命令表接管）。
/// 参数：`root` 是规范化后的产品根；`args` 是完整 argv。
/// 返回：要退出就给退出码，`None` 表示继续装配。
pub(crate) fn after_root(root: &Path, args: &[String]) -> Option<i32> {
    // 隐藏模式：按条处置围栏台账（列清单 / 还原一条 / 撤一条 / 删一个 profile）——比整体收尾更细，
    // 所以排在 --fence-clean 之前；两者共用同一份台账与同一套 ACE 读法（见 docs/kernel/README.md 的「围栏落点：必要与可选」）。
    if let Some(code) = crate::guard::fence_grant(args, root) {
        return Some(code);
    }
    // 隐藏模式：精确回收围栏写过的权限项（不需要装配核心，也就不需要提示词册）。
    if args.iter().any(|a| a == "--fence-clean") {
        return Some(crate::guard::fence_clean(root));
    }
    None
}
