//! 目的：provision 端口实现的用例——宿主文件系统、台账、取数与进程账。
//! 管：`detail::host` / `detail::ledger` / `detail::fetch` / `detail::procs` 的契约。
//! 不管：编排与镜像（见 `service.rs`、`image.rs`）。
//! 联动：被测实现是 `capabilities::provision::detail` 的各端口适配器。

use crate::capabilities::provision::detail::fetch::HttpFetch;
use crate::capabilities::provision::detail::host::RealHost;
use crate::capabilities::provision::detail::ledger::FileLedger;
use crate::capabilities::provision::detail::procs::BookProcs;
use crate::capabilities::provision::domain::{install_identity, Dep};
use crate::capabilities::provision::ports::{Fetcher, HostFs, Ledger, Procs};
use std::fs;

/// 目的：造一条任意安装方式的清单条目（端口契约用例夹具）。
fn dep_of(id: &str, install: &str, probe: &str) -> Dep {
    Dep {
        id: id.to_string(),
        os: "any".to_string(),
        arch: "any".to_string(),
        install: install.to_string(),
        probe: probe.to_string(),
        source: "release:x".to_string(),
        kind: "file".to_string(),
        guest: None,
        toolchain: None,
        family: None,
        strip: 0,
        size_mb: 0,
        sha256: String::new(),
        note: String::new(),
    }
}

/// 目的：宿主落点端口的契约——建、查、删都能在临时根上往返，静默失败即不成立。
#[test]
fn host_fs_round_trips_directories_files_and_the_exec_bit() {
    let root = crate::tests::scratch("provision-ports-host");
    let host = RealHost;
    let dir = root.join("a").join("b");
    assert!(!host.exists(&dir), "临时根上不该凭空存在");
    host.mkdir_all(&dir).expect("建目录要成功");
    assert!(host.is_dir(&dir));
    let file = dir.join("tool");
    fs::write(&file, b"x").expect("写文件");
    assert!(host.is_file(&file));
    host.make_executable(&file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&file).expect("元数据").permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "执行位要真的落上：{:o}", mode);
    }
    host.remove_file(&file).expect("删文件要成功");
    assert!(!host.exists(&file));
    host.remove_dir_all(&root.join("a")).expect("删目录要成功");
    assert!(!host.exists(&root.join("a")));
    let _ = fs::remove_dir_all(&root);
}

/// 目的：台账端口的契约——在场判定看探针、登记按安装身份、划掉只剩空台账。
#[test]
fn ledger_records_by_identity_and_forgets_the_entry() {
    let root = crate::tests::scratch("provision-ports-ledger");
    let ledger = FileLedger::new(&root);
    let dep = dep_of("demo", "inst/demo", "inst/demo/demo.txt");
    assert!(!ledger.present(&dep), "没落盘就不该算在场");
    fs::create_dir_all(root.join("inst").join("demo")).expect("建安装目录");
    fs::write(root.join("inst").join("demo").join("demo.txt"), b"x").expect("落探针");
    assert!(ledger.present(&dep), "探针在场就要算在场");
    assert!(ledger.snapshot().is_empty(), "还没登记台账应当为空");
    ledger.record(&dep).expect("登记要成功");
    assert_eq!(ledger.snapshot().get("demo"), Some(&install_identity(&dep)));
    ledger.forget("demo").expect("划掉要成功");
    assert!(ledger.snapshot().is_empty(), "划掉后台账应当为空");
    let _ = fs::remove_dir_all(&root);
}

/// 目的：取数端口的契约——摘要按内容算成小写十六进制，同一份内容稳定。
#[test]
fn fetcher_digests_file_contents_as_lowercase_hex() {
    let root = crate::tests::scratch("provision-ports-digest");
    let file = root.join("payload.bin");
    fs::write(&file, b"abc").expect("写文件");
    let fetch = HttpFetch::new(15, 7200);
    let digest = fetch.digest(&file).expect("摘要要成功");
    assert_eq!(
        digest,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(fetch.digest(&file).expect("再算一次"), digest);
    let _ = fs::remove_dir_all(&root);
}

/// 目的：进程端口的契约——台账为空时如实说没有，不凭空造条目。
#[test]
fn procs_reports_nothing_before_any_guest_runs() {
    let root = crate::tests::scratch("provision-ports-procs");
    let procs = BookProcs::new(&root);
    assert!(procs.entries().is_empty(), "空台账不该有条目");
    assert!(!procs.guest_running(), "没有条目就没有 guest 在跑");
    let _ = fs::remove_dir_all(&root);
}
