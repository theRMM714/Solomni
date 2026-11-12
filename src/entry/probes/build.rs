//! 目的：构建一个模块声明的产物（隐藏模式 --build）。
//! 管：参数解析、模块查找、构建期网络开关与回执打印。
//! 不管：构建用例、缓存与执行（在 builds 能力）；能力面装配（在 entry::assembly）。
//! 联动：由 main 在 --build 分支调用；模块契约见 MODULE_SPEC 的「构建」。

/// 目的：隐藏模式——**构建一个模块声明的产物**（系统动作：在目标环境里跑 `module.yaml` 的 `build.command`）。
/// 参数：root = 产品根；args = 原始参数；i = `--build` 的位置（`--build=<模块 id>`；可选 `--guest=<虚拟根>`）。
/// 返回：退出码（0 成功；1 构建失败或环境不齐；2 用法错、没有该模块、或模块没有 `build` 段）。
/// 约束：用例在 `builds` 能力（本机档用产品围栏、虚拟机档在 guest 里编）；这里只解析参数、装配能力面与打印回执。
pub fn build(root: &std::path::Path, args: &[String], i: usize) -> i32 {
    use crate::capabilities::builds::api::Target;
    let id = match args[i].strip_prefix("--build=") {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => args.get(i + 1).cloned().unwrap_or_default(),
    };
    if id.is_empty() {
        eprintln!("用法：solomni --build=<模块 id> [--guest=<虚拟根>]");
        return 2;
    }
    let guest = args.iter().find_map(|a| a.strip_prefix("--guest="));
    if let Some(g) = guest
        && g.trim().is_empty()
    {
        eprintln!("[构建] --guest 要给虚拟根名（`.vm/guest/<名>`）");
        return 2;
    }
    let Some(module) = module_by_id(root, &id) else {
        eprintln!("[构建] 没有模块「{}」（清单被拒收的话见 --doctor）", id);
        return 2;
    };
    let Some(decl) = module.manifest.build.clone() else {
        eprintln!(
            "[构建] 模块「{}」没声明 build 段：在 module.yaml 里写 build.command（见 MODULE_SPEC 的「构建」）",
            id
        );
        return 2;
    };
    let target = match guest {
        Some(g) => Target::Guest(g.trim().to_string()),
        None => Target::Host,
    };
    let builds = match crate::entry::assembly::build_service(root) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[构建] {}", e);
            return 1;
        }
    };
    let net = global_network_allowed(root) && decl.net;
    let at = match &target {
        Target::Host => "在模块根".to_string(),
        Target::Guest(g) => format!("在 guest「{}」里", g),
    };
    println!(
        "[构建] {}：{}跑声明的构建命令（出站网络：{}）",
        id,
        at,
        if net { "放行" } else { "不放行" }
    );
    match crate::capabilities::builds::api::Builds::build(&builds, &module, &target, net) {
        Ok(b) if b.skipped => {
            println!("[构建] {} 在目标 {} 上已是最新，跳过", id, b.target);
            0
        }
        Ok(b) => {
            print!("{}", b.output);
            println!("[构建] {} 构建完成（目标 {}）", id, b.target);
            0
        }
        Err(e) => {
            eprintln!("[构建] {}", e);
            1
        }
    }
}

/// 目的：按模块 id 扫出模块（扫盘；清单被拒收的在这里就是"找不到"，细节看 `--doctor`）。
fn module_by_id(
    root: &std::path::Path,
    id: &str,
) -> Option<crate::capabilities::modules::api::Module> {
    use crate::capabilities::modules::ports::ModuleSource;
    crate::capabilities::modules::detail::FsModules::new(
        root.join("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan()
    .modules
    .into_iter()
    .find(|m| m.manifest.id == id)
}

/// 目的：构建期的全局网络开关（与主程序同一份 settings.yaml；读不到/非法按拒绝）。
fn global_network_allowed(root: &std::path::Path) -> bool {
    std::fs::read_to_string(root.join(".home").join("settings.yaml"))
        .ok()
        .and_then(|t| {
            yaml_serde::from_str::<crate::capabilities::registry::api::AppSettings>(&t).ok()
        })
        .map(|a| a.sandbox.policy.network)
        .unwrap_or(false)
}
