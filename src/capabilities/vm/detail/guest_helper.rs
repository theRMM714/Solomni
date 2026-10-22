//! 目的：Linux guest 内的执行助手——把 host 侧翻好的 per-agent 允许集交给 Landlock，再执行命令。
//! 管：助手脚本文本、装进 guest、在 boot 后确认它可执行；超时/退出杀整棵进程树、环境白名单与输出由同一份策略驱动。
//! 不管：策略从哪里来（host 侧 FenceSpec）、guest 通道实现（qemu.rs）。
//! 联动：qemu.rs 在挂载后调用 install；exec_fenced 用 `HELPER_PATH` 执行。

use crate::capabilities::vm::domain::guest::GuestRun;
use crate::capabilities::vm::ports::GuestChannel;

/// 目的：guest 内助手路径（会话可写层里，随每台 guest 一起重建）。
pub const HELPER_PATH: &str = "/usr/local/sbin/solomni-guest-fence";

/// 目的：助手超时（与本机守门进程的 30 秒同一口径；到点杀整棵进程树）。
pub const HELPER_TIMEOUT_SECS: u64 = 30;

/// 目的：助手脚本正文。guest 只需 python3（虚拟根基线已自证）；缺 python3 时执行必然失败，不降级。
pub const HELPER_SCRIPT: &str = r#"#!/usr/bin/env python3
import ctypes
import json
import os
import signal
import subprocess
import sys
import warnings

warnings.filterwarnings("ignore", category=DeprecationWarning)

SYS_CREATE = 444
SYS_ADD = 445
SYS_RESTRICT = 446
RULE_PATH_BENEATH = 1
TIMEOUT_SECS = 30


def die(msg):
    sys.stderr.write("solomni-guest-fence: " + msg + "\n")
    sys.exit(111)


def add_dir(path, rights, ruleset, read_ok, handled):
    if not path:
        return
    try:
        pfd = os.open(path, os.O_PATH | os.O_CLOEXEC)
    except OSError:
        return
    try:
        if os.path.isdir(path):
            allowed = rights
        else:
            allowed = ((1 << 0) | (1 << 1) | (1 << 2)) & rights
            if rights & (1 << 13):
                allowed |= 1 << 13
            if rights & (1 << 14):
                allowed |= 1 << 14
        rule = PathBeneath(allowed, pfd)
        if libc.syscall(SYS_ADD, ruleset, RULE_PATH_BENEATH, ctypes.byref(rule), 0) < 0:
            die("landlock_add_rule(" + path + "): " + os.strerror(ctypes.get_errno()))
    finally:
        os.close(pfd)


def required_dir(path, rights, ruleset, read_ok, handled):
    if not path:
        return
    if not os.path.exists(path):
        die("permission root is missing: " + path)
    add_dir(path, rights, ruleset, read_ok, handled)


def install_dir(bin_dir):
    leaf = os.path.basename(bin_dir).lower()
    if leaf in ("bin", "scripts"):
        up = os.path.dirname(bin_dir)
        if up and up != "/":
            return up
    return bin_dir


def interpreter_dirs(script):
    path_var = os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin")
    dirs = set()
    for raw in script.replace("\n", " ").split(" "):
        token = raw.strip("\"'()")
        if not token or token.startswith(("-", "/", "%")):
            continue
        if os.path.isabs(token):
            continue
        for entry in path_var.split(":"):
            entry = entry.strip('"')
            if not entry:
                continue
            candidate = os.path.join(entry, token)
            if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
                dirs.add(install_dir(os.path.dirname(candidate)))
                real = os.path.realpath(candidate)
                dirs.add(install_dir(os.path.dirname(real)))
                break
    return sorted(d for d in dirs if d and os.path.isdir(d))


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

    global libc
    libc = ctypes.CDLL(None, use_errno=True)
    abi = libc.syscall(SYS_CREATE, None, 0, 1)
    if abi < 1:
        die("landlock is unavailable")
    handled = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6)
    handled |= (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12)
    if abi >= 2:
        handled |= 1 << 13
    if abi >= 3:
        handled |= 1 << 14

    global PathBeneath
    class Ruleset(ctypes.Structure):
        _fields_ = [("handled_access_fs", ctypes.c_uint64)]

    class PathBeneath(ctypes.Structure):
        _pack_ = 1
        _fields_ = [("allowed_access", ctypes.c_uint64), ("parent_fd", ctypes.c_int32)]

    attr = Ruleset(handled)
    fd = libc.syscall(SYS_CREATE, ctypes.byref(attr), ctypes.sizeof(attr), 0)
    if fd < 0:
        die("landlock_create_ruleset: " + os.strerror(ctypes.get_errno()))

    read_ok = (1 << 0) | (1 << 2) | (1 << 3)
    # 只读运行基线：与本机 Linux 后端同一份范围（guest 里也只需要这些就能起解释器）。
    for base in (
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32",
        "/etc/ld.so.cache", "/etc/localtime", "/etc/terminfo",
        "/etc/ssl/openssl.cnf", "/etc/pki/tls/openssl.cnf",
        "/dev/null", "/dev/zero", "/dev/urandom", "/dev/random",
        "/proc/self", "/proc/meminfo", "/sys/devices/system/cpu",
    ):
        add_dir(base, read_ok, fd, read_ok, handled)
    for p in policy.get("ro_tree", []) + policy.get("ro", []):
        required_dir(p, read_ok, fd, read_ok, handled)
    cwd = policy.get("cwd", "")
    if cwd:
        ordinary = cwd
        if os.path.isdir(ordinary):
            add_dir(ordinary, read_ok, fd, read_ok, handled)
    for p in policy.get("rw", []):
        required_dir(p, handled, fd, read_ok, handled)
    private = policy.get("private", "")
    if private:
        required_dir(private, handled, fd, read_ok, handled)
    for p in interpreter_dirs(policy.get("script", "")):
        add_dir(p, read_ok, fd, read_ok, handled)

    if libc.prctl(38, 1, 0, 0, 0) != 0:
        die("prctl(PR_SET_NO_NEW_PRIVS): " + os.strerror(ctypes.get_errno()))
    if libc.syscall(SYS_RESTRICT, fd, 0) < 0:
        die("landlock_restrict_self: " + os.strerror(ctypes.get_errno()))
    os.close(fd)

    if cwd:
        try:
            os.chdir(cwd)
        except OSError as e:
            die("chdir(" + cwd + "): " + str(e))

    # 环境白名单：不继承守护进程环境；只保留运行时需要的最小集合 + 该模块的隐秘字段名。
    keep = ("PATH", "LANG", "LC_ALL", "TZ", "PYTHONIOENCODING", "PYTHONUTF8")
    env = {k: os.environ[k] for k in keep if k in os.environ}
    for k in policy.get("env_names", []):
        if k and k in os.environ:
            env[k] = os.environ[k]
    home = private or cwd
    if home:
        env["HOME"] = home
        env["TMPDIR"] = home
        env["TMP"] = home
        env["TEMP"] = home

    try:
        child = subprocess.Popen(
            ["/bin/sh", "-c", policy.get("script", "")],
            env=env,
            cwd=cwd or None,
            start_new_session=True,
        )
    except OSError as e:
        die("spawn(/bin/sh): " + str(e))
    try:
        code = child.wait(timeout=TIMEOUT_SECS)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except OSError:
            pass
        try:
            child.wait(timeout=5)
        except Exception:
            pass
        sys.stderr.write("solomni-guest-fence: 工具进程超时，已终止整棵进程树\n")
        sys.exit(124)
    if code is None or code < 0:
        sys.exit(111)
    sys.exit(code)


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
