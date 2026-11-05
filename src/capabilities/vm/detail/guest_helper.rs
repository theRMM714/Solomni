//! 目的：Linux guest 内的执行助手——把 host 侧翻好的 per-agent 允许集交给 Landlock，再执行命令。
//! 管：助手脚本文本、装进 guest、在 boot 后确认它可执行；超时/退出杀整棵进程树、环境白名单与输出由同一份策略驱动。
//! 不管：策略从哪里来（host 侧 FenceSpec）、guest 通道实现（qemu.rs）。
//! 联动：qemu.rs 在挂载后调用 install；exec_fenced 用 `HELPER_PATH` 执行。

use crate::capabilities::vm::domain::guest::GuestRun;
use crate::capabilities::vm::ports::GuestChannel;

/// 目的：guest 内助手路径（会话可写层里，随每台 guest 一起重建）。
pub const HELPER_PATH: &str = "/usr/local/sbin/solomni-guest-fence";

/// 目的：助手缺省超时（与本机守门进程的 30 秒同一口径）；每次执行的 timeout_secs 由策略给，缺省用它。
#[allow(dead_code)]
pub const HELPER_TIMEOUT_SECS: u64 = 30;

/// 目的：助手诊断行的统一前缀（围栏装不上 / 起不了进程这类「没执行」的事实）。
pub const HELPER_DIAG_PREFIX: &str = "solomni-guest-fence: ";

/// 目的：超时的机器标记——通道据此产出 `[shell] 超时（N 秒）` 并把标记剥掉（人读文案在 ProcessTexts）。
pub const HELPER_TIMEOUT_MARKER: &str = "solomni-guest-fence: timeout";

/// 目的：被信号结束的机器标记——通道据此产出「退出码未知（被信号结束）」并剥掉标记。
pub const HELPER_SIGNAL_MARKER: &str = "solomni-guest-fence: signal";

/// 目的：诊断出错时 helper 写到 stderr 的前缀（与围栏的 `solomni-guest-fence: ` 分开，便于宿主分流）。
pub const HELPER_DIAGNOSE_PREFIX: &str = "solomni-guest-diagnose: ";

/// 目的：诊断失败后的**有界预检**——一条命令区分「helper 不在 / 缺 python3 / 都对」，宿主据此给不同原因。
/// 约束：命令自带 `timeout 10`（未围栏通道的外层预算很长，不能靠它兜底）；三个标记互斥。
pub fn probe_command() -> String {
    format!(
        "timeout 10 sh -c 'test -x {helper} || {{ echo {no_helper}; exit 3; }}; command -v python3 >/dev/null 2>&1 || {{ echo {no_python}; exit 4; }}; echo {ok}'",
        helper = HELPER_PATH,
        no_helper = PROBE_NO_HELPER,
        no_python = PROBE_NO_PYTHON,
        ok = PROBE_OK
    )
}

/// 目的：预检标记——helper 可执行位不在。
pub const PROBE_NO_HELPER: &str = "SOLOMNI_DIAG_PROBE=no-helper";
/// 目的：预检标记——helper 在，但 guest 里没有 python3。
pub const PROBE_NO_PYTHON: &str = "SOLOMNI_DIAG_PROBE=no-python";
/// 目的：预检标记——helper 与 python3 都在（失败原因不是这两个）。
pub const PROBE_OK: &str = "SOLOMNI_DIAG_PROBE=ok";

/// 目的：助手脚本正文。guest 只需 python3（虚拟根基线已自证）；缺 python3 时执行必然失败，不降级。
pub const HELPER_SCRIPT: &str = r#"#!/usr/bin/env python3
import ctypes
import json
import os
import signal
import subprocess
import sys
import time
import warnings

warnings.filterwarnings("ignore", category=DeprecationWarning)

SYS_CREATE = 444
SYS_ADD = 445
SYS_RESTRICT = 446
RULE_PATH_BENEATH = 1
CLONE_NEWNET = 0x40000000
CHILD = None


def on_term(signum, frame):
    child = CHILD
    if child is not None:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except OSError:
            pass
    # 取消由宿主侧的取消旗判定（通道据此产出「用户停止」状态行）；这里只杀树、只留退出码。
    sys.exit(143)


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


def read_text(path):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def cpu_counters():
    for line in read_text("/proc/stat").splitlines():
        if line.startswith("cpu "):
            vals = []
            for part in line.split()[1:]:
                try:
                    vals.append(int(part))
                except ValueError:
                    vals.append(0)
            while len(vals) < 8:
                vals.append(0)
            return vals[:8]
    return [0, 0, 0, 0, 0, 0, 0, 0]


def counter_sum(vals):
    total = 0
    for v in vals:
        total += v
    return total


def read_pid_stat(pid):
    data = read_text("/proc/" + str(pid) + "/stat")
    if not data:
        return None
    left = data.find("(")
    right = data.rfind(")")
    if left < 0 or right < left:
        return None
    comm = data[left + 1:right]
    rest = data[right + 2:].split()
    if len(rest) < 20:
        return None
    try:
        return {
            "state": rest[0],
            "ppid": int(rest[1]),
            "ticks": int(rest[11]) + int(rest[12]),
            "starttime": int(rest[19]),
            "comm": comm,
        }
    except (ValueError, IndexError):
        return None


def process_table():
    table = {}
    try:
        names = os.listdir("/proc")
    except OSError:
        return table
    for name in names:
        if not name.isdigit():
            continue
        pid = int(name)
        stat = read_pid_stat(pid)
        if stat is None:
            continue
        table[pid] = stat
    return table


def read_rss_kb(pid):
    for line in read_text("/proc/" + str(pid) + "/status").splitlines():
        if line.startswith("VmRSS:"):
            parts = line.split()
            if len(parts) >= 2:
                try:
                    return int(parts[1])
                except ValueError:
                    return 0
    return 0


def read_args(pid, fallback):
    text = read_text("/proc/" + str(pid) + "/cmdline").replace(chr(0), " ").strip()
    if not text:
        text = fallback
    return text[:80]


def read_meminfo():
    out = {}
    for line in read_text("/proc/meminfo").splitlines():
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        parts = value.split()
        if not parts:
            continue
        try:
            out[key.strip()] = int(parts[0])
        except ValueError:
            continue
    return out


def read_loadavg():
    parts = read_text("/proc/loadavg").split()
    values = []
    for i in range(3):
        try:
            values.append(round(float(parts[i]), 2))
        except (ValueError, IndexError):
            values.append(0.0)
    return values


def read_uptime():
    parts = read_text("/proc/uptime").split()
    try:
        return float(parts[0])
    except (ValueError, IndexError):
        return 0.0


def read_mounts():
    out = []
    for line in read_text("/proc/mounts").splitlines():
        parts = line.split()
        if len(parts) < 3:
            continue
        source = parts[0]
        target = parts[1]
        fstype = parts[2]
        try:
            st = os.statvfs(target)
        except OSError:
            continue
        total = st.f_blocks * st.f_frsize
        free = st.f_bavail * st.f_frsize
        if total <= 0:
            continue
        out.append({
            "source": source[:120],
            "target": target[:160],
            "fstype": fstype[:32],
            "total_kb": total // 1024,
            "free_kb": free // 1024,
        })
        if len(out) >= 40:
            break
    return out


def read_nproc():
    try:
        return max(1, len(os.sched_getaffinity(0)))
    except (AttributeError, OSError):
        cpus = os.cpu_count()
        if cpus and cpus > 0:
            return cpus
        return 1


def read_hz():
    try:
        hz = int(os.sysconf("SC_CLK_TCK"))
    except (AttributeError, ValueError, OSError):
        hz = 100
    if hz < 1:
        return 100
    return hz


def encode_report(report):
    return json.dumps(report, ensure_ascii=False, separators=(",", ":"))


def shrink_report(report):
    limit = 11800
    if len(encode_report(report)) > limit:
        for row in report["processes"]:
            row["args"] = row["args"][:24]
            row["comm"] = row["comm"][:16]
        report["truncated"] = True
    while len(encode_report(report)) > limit and len(report["mounts"]) > 8:
        report["mounts"].pop()
        report["truncated"] = True
    while len(encode_report(report)) > limit and report["processes"]:
        report["processes"].pop()
        report["truncated"] = True
    if len(encode_report(report)) > limit:
        report["mounts"] = []
        report["processes"] = []
        report["truncated"] = True
    return report


def collect_diagnosis():
    first = process_table()
    first_cpu = cpu_counters()
    time.sleep(1.0)
    second = process_table()
    second_cpu = cpu_counters()
    up = read_uptime()
    hz = read_hz()
    ncpu = read_nproc()
    delta_total = counter_sum(second_cpu) - counter_sum(first_cpu)
    if delta_total < 1:
        delta_total = 1
    rows = []
    for pid in second:
        stat = second[pid]
        before = first.get(pid)
        delta = 0
        if before is not None:
            delta = stat["ticks"] - before["ticks"]
        if delta < 0:
            delta = 0
        percent = round(delta / float(delta_total) * 100.0 * ncpu, 1)
        etime = up - stat["starttime"] / float(hz)
        if etime < 0:
            etime = 0.0
        rows.append({
            "pid": pid,
            "ppid": stat["ppid"],
            "state": stat["state"][:8],
            "cpu_percent": percent,
            "rss_kb": read_rss_kb(pid),
            "etime_secs": int(etime),
            "comm": stat["comm"][:64],
            "args": read_args(pid, stat["comm"]),
        })
    process_total = len(rows)
    rows.sort(key=lambda row: (row["cpu_percent"], row["rss_kb"]), reverse=True)
    truncated = process_total > 80
    rows = rows[:80]
    mem = read_meminfo()
    total_kb = mem.get("MemTotal", 0)
    available_kb = mem.get("MemAvailable", 0)
    if available_kb == 0:
        available_kb = mem.get("MemFree", 0) + mem.get("Buffers", 0) + mem.get("Cached", 0)
    used_percent = 0.0
    if total_kb > 0:
        used_percent = round((total_kb - available_kb) * 100.0 / total_kb, 1)
    if used_percent < 0:
        used_percent = 0.0
    cpu = cpu_counters()
    cpu_sum = counter_sum(cpu)
    idle_percent = 0.0
    if cpu_sum > 0:
        idle_percent = round(cpu[3] * 100.0 / cpu_sum, 1)
    report = {
        "uptime_secs": round(up, 1),
        "loadavg": read_loadavg(),
        "process_total": process_total,
        "truncated": truncated,
        "processes": rows,
        "memory": {
            "total_kb": total_kb,
            "available_kb": available_kb,
            "free_kb": mem.get("MemFree", 0),
            "buffers_kb": mem.get("Buffers", 0),
            "cached_kb": mem.get("Cached", 0),
            "swap_total_kb": mem.get("SwapTotal", 0),
            "swap_free_kb": mem.get("SwapFree", 0),
            "used_percent": used_percent,
        },
        "cpu": {
            "user": cpu[0],
            "nice": cpu[1],
            "system": cpu[2],
            "idle": cpu[3],
            "iowait": cpu[4],
            "irq": cpu[5],
            "softirq": cpu[6],
            "steal": cpu[7],
            "idle_percent": idle_percent,
        },
        "mounts": read_mounts(),
        "guest": {
            "kernel": os.uname().release[:120],
            "nproc": ncpu,
        },
    }
    return shrink_report(report)


def run_diagnosis():
    try:
        report = collect_diagnosis()
    except Exception as exc:
        sys.stderr.write("solomni-guest-diagnose: " + str(exc) + "\n")
        sys.stderr.flush()
        sys.exit(111)
    sys.stdout.write(encode_report(report) + "\n")
    sys.stdout.flush()
    sys.exit(0)


def main():
    global CHILD
    if len(sys.argv) != 2:
        die("usage: policy.json")
    path = sys.argv[1]
    with open(path, "r", encoding="utf-8") as f:
        policy = json.load(f)
    try:
        os.unlink(path)
    except OSError:
        pass
    # 本机把本次 timeout_secs 写进策略；缺省 / 非法值退回 30 秒（helper 始终有界）。
    try:
        timeout_secs = int(policy.get("timeout_secs", 30))
    except (TypeError, ValueError):
        timeout_secs = 30
    if timeout_secs < 1:
        timeout_secs = 1
    # host 取消：QGA 发 SIGTERM 给 helper，handler 杀掉子进程组再退出。
    signal.signal(signal.SIGTERM, on_term)

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
    write_file = 1 << 1
    # 运行基线：与本机 Linux 后端同一份范围（guest 里也只需要这些就能起解释器）；
    # /dev/null 额外可写——标准重定向要能丢弃输出（`>/dev/null` / `2>/dev/null`），只给这个设备节点。
    for base in (
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32",
        "/etc/ld.so.cache", "/etc/localtime", "/etc/terminfo",
        "/etc/ssl/openssl.cnf", "/etc/pki/tls/openssl.cnf",
        "/dev/null", "/dev/zero", "/dev/urandom", "/dev/random",
        "/proc/self", "/proc/meminfo", "/sys/devices/system/cpu",
    ):
        rights = read_ok | write_file if base == "/dev/null" else read_ok
        add_dir(base, rights, fd, read_ok, handled)
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

    # 系统级只读诊断：Landlock 已 restrict（只读根由策略给），不 spawn 命令、只采 /proc。
    if policy.get("diagnose"):
        run_diagnosis()

    if cwd:
        try:
            os.chdir(cwd)
        except OSError as e:
            die("chdir(" + cwd + "): " + str(e))

    # 环境白名单：不继承守护进程环境；只保留运行时需要的最小集合 + 该模块的隐秘字段名。
    keep = ("PATH", "LANG", "LC_ALL", "TZ", "PYTHONIOENCODING", "PYTHONUTF8")
    env = {k: os.environ[k] for k in keep if k in os.environ}
    # QGA 不保证带 PATH（也可能给空串）；不给默认值，g++ 的 collect2 会找不到 ld。
    if not env.get("PATH"):
        env["PATH"] = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    for k in policy.get("env_names", []):
        if k and k in os.environ:
            env[k] = os.environ[k]
    home = private or cwd
    if home:
        env["HOME"] = home
        env["TMPDIR"] = home
        env["TMP"] = home
        env["TEMP"] = home

    # 本次调用没有网络 reach：给子进程一个空网络命名空间。失败必须 fail-closed，
    # 不能把"应该断网"悄悄降级成"照常联网"。
    if not policy.get("net", True):
        if libc.unshare(CLONE_NEWNET) != 0:
            die("unshare(CLONE_NEWNET): " + os.strerror(ctypes.get_errno()))

    try:
        child = subprocess.Popen(
            ["/bin/sh", "-c", policy.get("script", "")],
            env=env,
            cwd=cwd or None,
            start_new_session=True,
        )
    except OSError as e:
        die("spawn(/bin/sh): " + str(e))
    CHILD = child
    try:
        code = child.wait(timeout=timeout_secs)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except OSError:
            pass
        try:
            child.wait(timeout=5)
        except Exception:
            pass
        sys.stderr.write("solomni-guest-fence: timeout\n")
        sys.stderr.flush()
        sys.exit(124)
    if code is None or code < 0:
        sys.stderr.write("solomni-guest-fence: signal\n")
        sys.stderr.flush()
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
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
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
        timeout_secs: 30,
        output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
    });
    if !chmod.ok {
        return Err(format!("给 guest 助手加执行位失败：{}", chmod.output));
    }
    Ok(())
}
