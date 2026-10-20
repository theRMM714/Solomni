//! 目的：Linux guest 内的执行助手——把 host 侧翻好的 per-agent 允许集交给 Landlock，再执行命令。
//! 管：助手脚本文本、装进 guest、在 boot 后确认它可执行。
//! 不管：策略从哪里来（host 侧 FenceSpec）、guest 通道实现（qemu.rs）。
//! 联动：qemu.rs 在挂载后调用 install；exec_fenced 用 `HELPER_PATH` 执行。

use crate::capabilities::vm::domain::guest::GuestRun;
use crate::capabilities::vm::ports::GuestChannel;

/// 目的：guest 内助手路径（会话可写层里，随每台 guest 一起重建）。
pub const HELPER_PATH: &str = "/usr/local/sbin/solomni-guest-fence";

/// 目的：助手脚本正文。guest 只需 python3（虚拟根基线已自证）；缺 python3 时执行必然失败，不降级。
pub const HELPER_SCRIPT: &str = r#"#!/usr/bin/env python3
import ctypes
import json
import os
import sys
import warnings

warnings.filterwarnings("ignore", category=DeprecationWarning)

SYS_CREATE = 444
SYS_ADD = 445
SYS_RESTRICT = 446
RULE_PATH_BENEATH = 1


def die(msg):
    sys.stderr.write("solomni-guest-fence: " + msg + "\n")
    sys.exit(111)


def main():
    if len(sys.argv) != 2:
        die("usage: policy.json")
    path = sys.argv[1]
    with open(path, "r", encoding="utf-8") as f:
        policy = json.load(f)
    try:
        os.unlink(path)
    except OSError:
        pass

    libc = ctypes.CDLL(None, use_errno=True)
    abi = libc.syscall(SYS_CREATE, None, 0, 1)
    if abi < 1:
        die("landlock is unavailable")
    handled = (
        (1 << 0)
        | (1 << 1)
        | (1 << 2)
        | (1 << 3)
        | (1 << 4)
        | (1 << 5)
        | (1 << 6)
        | (1 << 7)
        | (1 << 8)
        | (1 << 9)
        | (1 << 10)
        | (1 << 11)
        | (1 << 12)
    )
    if abi >= 2:
        handled |= 1 << 13
    if abi >= 3:
        handled |= 1 << 14

    class Ruleset(ctypes.Structure):
        _fields_ = [("handled_access_fs", ctypes.c_uint64)]

    attr = Ruleset(handled)
    fd = libc.syscall(SYS_CREATE, ctypes.byref(attr), ctypes.sizeof(attr), 0)
    if fd < 0:
        die("landlock_create_ruleset: " + os.strerror(ctypes.get_errno()))

    class PathBeneath(ctypes.Structure):
        _pack_ = 1
        _fields_ = [
            ("allowed_access", ctypes.c_uint64),
            ("parent_fd", ctypes.c_int32),
        ]

    read_ok = (1 << 0) | (1 << 2) | (1 << 3)

    def rights_for(path, rights):
        if os.path.isdir(path):
            return rights
        file_bits = (1 << 0) | (1 << 1) | (1 << 2)
        if rights & (1 << 13):
            file_bits |= 1 << 13
        if rights & (1 << 14):
            file_bits |= 1 << 14
        return file_bits

    def add(path, rights):
        if not path:
            return
        try:
            pfd = os.open(path, os.O_PATH | os.O_CLOEXEC)
        except OSError:
            return
        try:
            rule = PathBeneath(rights_for(path, rights), pfd)
            if libc.syscall(SYS_ADD, fd, RULE_PATH_BENEATH, ctypes.byref(rule), 0) < 0:
                die("landlock_add_rule(" + path + "): " + os.strerror(ctypes.get_errno()))
        finally:
            os.close(pfd)

    for base in (
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/etc",
        "/proc",
        "/sys",
    ):
        add(base, read_ok)
    dev_ok = (1 << 1) | (1 << 2)
    for dev in ("/dev/null", "/dev/urandom", "/dev/random"):
        add(dev, dev_ok)
    for p in policy.get("ro_tree", []) + policy.get("ro", []):
        add(p, read_ok)
    for p in policy.get("rw", []):
        add(p, handled)
    private = policy.get("private", "")
    if private:
        add(private, handled)

    if libc.prctl(38, 1, 0, 0, 0) != 0:
        die("prctl(PR_SET_NO_NEW_PRIVS): " + os.strerror(ctypes.get_errno()))
    if libc.syscall(SYS_RESTRICT, fd, 0) < 0:
        die("landlock_restrict_self: " + os.strerror(ctypes.get_errno()))
    os.close(fd)

    cwd = policy.get("cwd", "")
    if cwd:
        try:
            os.chdir(cwd)
        except OSError as e:
            die("chdir(" + cwd + "): " + str(e))

    env = os.environ.copy()
    if private:
        env["HOME"] = private
        env["TMPDIR"] = private
        env["TMP"] = private
        env["TEMP"] = private
    os.execve("/bin/sh", ["/bin/sh", "-c", policy.get("script", "")], env)


main()
"#;

/// 目的：把助手装进 guest（先建目录、写脚本、给执行位）；任一步失败都返回原因。
pub fn install(channel: &mut dyn GuestChannel) -> Result<(), String> {
    let mk = channel.exec(&GuestRun {
        command: "mkdir -p /usr/local/sbin".to_string(),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    });
    if !mk.ok {
        return Err(format!("建 guest 助手目录失败：{}", mk.output));
    }
    channel.put_file(HELPER_PATH, HELPER_SCRIPT.as_bytes())?;
    let chmod = channel.exec(&GuestRun {
        command: format!("chmod 0755 {}", HELPER_PATH),
        args_json: "{}".to_string(),
        env: Vec::new(),
        cwd: String::new(),
    });
    if !chmod.ok {
        return Err(format!("给 guest 助手加执行位失败：{}", chmod.output));
    }
    Ok(())
}
