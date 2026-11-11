//! 目的：入站契约的机器可读目录（隐藏模式 --routes）。
//! 管：把 HTTP 路由目录打成 JSON。
//! 不管：路由定义本身（在 presentation::web::routes）。
//! 联动：由 main 在 --routes 分支调用；契约见 docs/presentation/contracts.md。

/// 目的：入站契约（机器可读）：HTTP 路由目录的唯一定义（见 docs/presentation/contracts.md）。
pub fn print_routes() {
    println!("{}", crate::presentation::web::routes::catalog_json());
}
