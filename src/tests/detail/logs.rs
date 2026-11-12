//! 目的：文件日志适配器的落盘契约。
//! 管：每一级都写进 logs 下的时间戳文件。
//! 不管：日志消费、轮转与保留策略。
//! 联动：隔离根 `tests::scratch`。

use super::*;

#[test]
fn file_log_writes_every_level_into_a_timestamped_file_under_logs() {
    let root = scratch("file-log");
    {
        let log = FileLog::new(&root, "契约测试").expect("建日志文件成功");
        log.info("模块", "正常信息");
        log.warn("模块", "警示信息");
        log.error("模块", "错误信息");
    }
    let dir = root.join("logs");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("日志目录要建出来")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), 1, "一次运行一个文件：{:?}", names);
    assert!(names[0].ends_with(".log"));
    let text = std::fs::read_to_string(dir.join(&names[0])).expect("读日志");
    assert!(
        text.contains("契约测试")
            && text.contains("正常信息")
            && text.contains("警示信息")
            && text.contains("错误信息")
    );
    let _ = std::fs::remove_dir_all(&root);
}
