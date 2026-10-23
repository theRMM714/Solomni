//! 目的：本机档 `--build` 真用 `runtimes/` 里的包——真实二进制、真实围栏、真实 PATH。
//! 管：包 `bin/` 注入构建命令的 PATH；缺包如实拒绝并说清缺什么。
//! 不管：虚拟机档构建（在 `src/tests/diagnostics.rs` 与 vm 目标）、包选型纯逻辑（`src/tests/packages.rs`）。
//! 联动：契约见 RUNTIME_SPEC.md；被测入口是产品二进制的 `--build`。

use crate::probe::{bin, scratch};
use std::process::Command;

fn write_pkg_script(bin: &std::path::Path, name: &str, body: &str) {
    let path = bin.join(name);
    std::fs::write(&path, body).expect("写包工具");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&path).expect("读权限位").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).expect("加执行位");
    }
}

#[test]
fn host_build_uses_a_runtime_package_from_runtimes() {
    let root = scratch("host-pkg-build");
    let pkg_bin = root.join("runtimes").join("pkgtool").join("bin");
    std::fs::create_dir_all(&pkg_bin).expect("建包 bin");
    std::fs::write(
        root.join("runtimes").join("pkgtool").join("package.yaml"),
        "id: pkgtool\nversion: 1\nprefix: opt/rt/pkgtool\n",
    )
    .expect("写包清单");

    #[cfg(windows)]
    let (tool, artifact) = (
        "pkgtool.cmd",
        "@echo off\r\nif not exist build mkdir build\r\n> build\\hi.cmd echo @echo off\r\n>> build\\hi.cmd echo echo hi\r\n",
    );
    #[cfg(not(windows))]
    let (tool, artifact) = (
        "pkgtool",
        "#!/bin/sh\nset -e\nmkdir -p build\nprintf '#!/bin/sh\\necho hi\\n' > build/hi\nchmod +x build/hi\n",
    );
    write_pkg_script(&pkg_bin, tool, artifact);

    let module = root.join("modules").join("hello");
    std::fs::create_dir_all(&module).expect("建模块");
    std::fs::write(
        module.join("module.yaml"),
        "id: hello\nbrief: 本机包构建\nsystem: 用法：hi\nbuild:\n  command: pkgtool\n  needs: [pkgtool]\ntools:\n  hi:\n    command: build/hi\n",
    )
    .expect("写模块清单");

    let out = Command::new(bin())
        .arg("--root")
        .arg(&root)
        .arg("--build=hello")
        .output()
        .expect("跑产品二进制");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "本机包构建要成功：{}\n{}",
        stdout,
        stderr
    );
    let artifact = if cfg!(windows) {
        module.join("build").join("hi.cmd")
    } else {
        module.join("build").join("hi")
    };
    assert!(
        artifact.is_file(),
        "产物要落进模块目录：{}",
        artifact.display()
    );
    let state = root.join(".vm").join("build-state").join("hello.json");
    assert!(state.is_file(), "状态要落盘：{}", state.display());

    // 缺包：如实拒绝，点出缺哪个能力与去哪放。
    let needy = root.join("modules").join("needy");
    std::fs::create_dir_all(&needy).expect("建缺包模块");
    std::fs::write(
        needy.join("module.yaml"),
        "id: needy\nbrief: 缺包\nsystem: 用法：hi\nbuild:\n  command: pkgtool\n  needs: [no-such-cap]\ntools:\n  hi:\n    command: build/hi\n",
    )
    .expect("写缺包清单");
    let out = Command::new(bin())
        .arg("--root")
        .arg(&root)
        .arg("--build=needy")
        .output()
        .expect("跑产品二进制");
    assert!(!out.status.success(), "缺包要如实拒绝");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no-such-cap") && stderr.contains("runtimes/"),
        "要说清缺哪个能力与怎么补：{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&root);
}
