//! 目的：把所有归档解到目录（隐藏模式 --unpack）。
//! 管：归档格式识别与落盘（经 kernel 的解包机制）。
//! 不管：用户可见的帮助与参数校验以外的业务语义。
//! 联动：由 main 在 --unpack 分支调用。

/// 目的：把归档解到目录（跨平台纯 Rust，不依赖用户装解压工具；用法 solomni --unpack <archive> <dest>）。
pub fn unpack_cli(args: &[String], i: usize) -> i32 {
    let archive = args.get(i + 1).cloned().unwrap_or_default();
    let dest = args.get(i + 2).cloned().unwrap_or_default();
    if archive.is_empty() || dest.is_empty() {
        eprintln!("用法：solomni --unpack <archive> <dest>");
        return 2;
    }
    match crate::kernel::detail::unpack::extract(
        std::path::Path::new(&archive),
        std::path::Path::new(&dest),
    ) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("[解包] {}", e);
            1
        }
    }
}
