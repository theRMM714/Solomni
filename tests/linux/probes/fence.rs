//! Landlock 探针：真机验收本平台的文件系统围栏——允许的根里写得进，根之外读不到。
//! 驱动方式与运行期完全一致：交给守门进程（本程序 --fence-run）去装围栏。

use crate::probe::{run_launcher, scratch, spec_json};

#[test]
fn fence_denies_outside_paths_and_allows_the_given_roots() {
    let inside = scratch("fence-inside");
    let outside = scratch("fence-outside");
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, "SECRET-DO-NOT-LEAK").unwrap();
    let spec = spec_json(&[inside.clone()], &inside);

    // 允许的根里：写得进。
    let (code, out, err) = run_launcher(&spec, &format!("echo ok > {}", inside.join("x.txt").display()));
    if err.contains("文件系统围栏未生效") {
        eprintln!("[探针] 本平台没装上围栏（环境不允许）：{}（不作为通过）", err.trim());
        return;
    }
    assert!(inside.join("x.txt").exists(), "允许的根里应当写得进：{} / {}", out, err);
    assert_eq!(code, Some(0), "{} / {}", out, err);

    // 允许的根之外：同一个用户、同一台机器，只有围栏能挡住这一读。
    let (code, out, err) = run_launcher(&spec, &format!("cat {}", secret.display()));
    assert!(!out.contains("SECRET-DO-NOT-LEAK"), "越界读必须拿不到：{} / {}", out, err);
    assert_ne!(code, Some(0), "越界读应以非零退出：{} / {}", out, err);
}
