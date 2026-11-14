//! 目的：命令行旗语的唯一解析器——`--x v` 与 `--x=v` 等价，未知旗语报错并给最近建议。
//! 管：用户可见旗语（根 / Web / 端口 / 纯文本 / 帮助 / 版本）的取值与校验、位置参数收集。
//! 不管：根目录怎么规范化、各模式怎么执行（entry 与 main）、命令怎么分发（cli.rs）。
//! 联动：`src/main.rs` 取根之前解析一次；手册文案取这里的旗语表（`catalog::help_text`）。

/// 目的：一个用户可见旗语的规格：手册与解析共用同一份定义。
pub(crate) struct Flag {
    pub(crate) long: &'static str,
    pub(crate) short: Option<&'static str>,
    pub(crate) value: Option<&'static str>,
    pub(crate) desc: &'static str,
    pub(crate) hidden: bool,
}

/// 目的：用户可见旗语表：解析、`--help` 与手册都只认这一份。
pub(crate) const FLAGS: &[Flag] = &[
    Flag {
        long: "--root",
        short: None,
        value: Some("dir"),
        desc: "工作区根目录，默认当前目录",
        hidden: false,
    },
    Flag {
        long: "--web",
        short: None,
        value: None,
        desc: "直接进 Web 转录中心（只绑 127.0.0.1）",
        hidden: false,
    },
    Flag {
        long: "--port",
        short: None,
        value: Some("n"),
        desc: "Web 监听端口，默认 3081",
        hidden: false,
    },
    Flag {
        long: "--plain",
        short: None,
        value: None,
        desc: "关闭颜色、边框与动画，纯文本输出（也可用 NO_COLOR）",
        hidden: false,
    },
    Flag {
        long: "--no-anim",
        short: None,
        value: None,
        desc: "生成态只画静态帧，不轮流刷 spinner（录屏、慢终端）",
        hidden: false,
    },
    Flag {
        long: "--setup",
        short: None,
        value: Some("mode"),
        desc: "便携维护：PATH 或围栏预授（path/unset/undo/status/fence/all，省略=status）",
        hidden: false,
    },
    Flag {
        long: "--help",
        short: Some("-h"),
        value: None,
        desc: "打印本手册；`--help --all` 连内部旗语一起列",
        hidden: false,
    },
    Flag {
        long: "--version",
        short: Some("-v"),
        value: None,
        desc: "打印版本与构建档位",
        hidden: false,
    },
];

/// 目的：内部协议旗语——只给测试、探针与守门子进程用，不进取值解析，原样留给入口扫描。
const OPAQUE: &[&str] = &[
    "--proc-watchdog",
    "--fence-run",
    "--https-check",
    "--unpack",
    "--print-fence-env",
    "--fence-verify",
    "--print-routes",
    "--doctor",
    "--provision",
    "--work-gc",
    "--storage",
    "--gc",
    "--build",
    "--probe-tools",
    "--probe-replay",
];

/// 目的：解析结果：启动形态、根目录与端口，以及无法归类的剩余参数。
#[derive(Debug, Default)]
pub(crate) struct Flags {
    pub(crate) root: Option<String>,
    pub(crate) web: bool,
    pub(crate) port: Option<u16>,
    pub(crate) plain: bool,
    pub(crate) no_anim: bool,
    pub(crate) help: bool,
    pub(crate) version: bool,
    pub(crate) all: bool,
    /// 目的：`--setup=<模式>` 的取值：便携维护（PATH shim / 围栏机器级预授），省略值等于 `status`。
    pub(crate) setup: Option<String>,
    pub(crate) positionals: Vec<String>,
}

/// 目的：解析 argv（不含程序名）：短旗、长旗、`=值` 与 `--` 分隔都收。
/// 参数：`args` 是程序名之后的参数表。
/// 返回：解析好的旗语；旗语写错时给一句人话（含最近建议）。
pub(crate) fn parse(args: &[String]) -> Result<Flags, String> {
    let mut flags = Flags::default();
    let mut i = 0;
    let mut only_positional = false;
    while i < args.len() {
        let arg = &args[i];
        if only_positional || !arg.starts_with('-') || arg == "-" {
            flags.positionals.push(arg.clone());
            i += 1;
            continue;
        }
        if arg == "--" {
            only_positional = true;
            i += 1;
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        if OPAQUE.contains(&name) {
            // 内部协议模式自己解析整段 argv，这里不再往下列。
            flags.positionals.extend(args[i..].iter().cloned());
            return Ok(flags);
        }
        match name {
            "--root" => flags.root = Some(take(inline, args, &mut i, name)?),
            "--port" => {
                let raw = take(inline, args, &mut i, name)?;
                flags.port =
                    Some(raw.parse::<u16>().map_err(|_| {
                        format!("旗语 --port 要一个 1-65535 的整数，收到「{}」", raw)
                    })?);
            }
            // 维护旗：只吃 `=值` 或紧跟的已知模式名（不吃 `--root` 这类旗语）；省略值 = status。
            "--setup" => {
                let mode =
                    match inline {
                        Some(v) if v.is_empty() => return Err(
                            "旗语 --setup= 要一个模式：path / unset / undo / status / fence / all"
                                .to_string(),
                        ),
                        Some(v) => v,
                        None => match args.get(i + 1).map(String::as_str) {
                            Some(v @ ("path" | "unset" | "undo" | "status" | "fence" | "all")) => {
                                i += 1;
                                v.to_string()
                            }
                            _ => "status".to_string(),
                        },
                    };
                flags.setup = Some(mode);
            }
            "--web" => {
                no_value(&inline, name)?;
                flags.web = true;
            }
            "--plain" => {
                no_value(&inline, name)?;
                flags.plain = true;
            }
            "--no-anim" => {
                no_value(&inline, name)?;
                flags.no_anim = true;
            }
            "--help" | "-h" => {
                no_value(&inline, name)?;
                flags.help = true;
            }
            // `--all` 只修饰 `--help`：展开隐藏旗语，单独出现时无副作用。
            "--all" => {
                no_value(&inline, name)?;
                flags.all = true;
            }
            "--version" | "-v" => {
                no_value(&inline, name)?;
                flags.version = true;
            }
            // 台账维护旗：由 guard 从原始 argv 分派；这里只吃下旗语本身，后面的 `--root` 仍照常解析。
            "--fence-clean" | "--fence-reconcile" | "--fence-ledger" | "--fence-restore"
            | "--fence-revoke" | "--fence-profile-rm" => {}
            // 台账清单的可读版修饰符：由 guard 扫描原始 argv，解析层只吃下。
            "--human" => {}
            other => return Err(unknown(other)),
        }
        i += 1;
    }
    Ok(flags)
}

/// 目的：手册里的旗语行：一条旗语一行（占位符与说明来自同一张表）。
/// 参数：`all` 为真时把内部旗语也列出来。
pub(crate) fn flag_rows(all: bool) -> Vec<String> {
    let mut rows = Vec::new();
    for flag in FLAGS.iter().filter(|f| all || !f.hidden) {
        let spec = match (flag.short, flag.value) {
            (Some(short), Some(value)) => format!("{}, {} <{}>", short, flag.long, value),
            (Some(short), None) => format!("{}, {}", short, flag.long),
            (None, Some(value)) => format!("{} <{}>", flag.long, value),
            (None, None) => flag.long.to_string(),
        };
        rows.push(row(&spec, flag.desc));
    }
    if all {
        rows.push(String::new());
        rows.push("  内部协议（测试、探针与守门子进程用；正常使用不要传）".to_string());
        for (spec, desc) in [
            (
                "--probe-tools <model>",
                "列出模型可见的工具面（要真实网络）",
            ),
            (
                "--probe-replay <model>",
                "重放一次探测请求，看供应商接受哪种回放形状",
            ),
            ("--doctor", "自检：这台机器能承载哪些测试"),
            ("--provision", "依赖补齐状态与执行"),
            (
                "--work-gc <id>",
                "显式回收一个不再可达的内容对象（默认 dry-run）",
            ),
            ("--storage", "存储空间报告与回收"),
            ("--gc <id>", "回收一个内容对象（默认 dry-run）"),
            ("--build <模块id>", "在目标环境里构建一个模块声明的产物"),
        ] {
            rows.push(row(spec, desc));
        }
    }
    rows
}

/// 目的：手册里的一行旗语：占位符按显示宽度补齐（说明里的中文按两格算）。
fn row(spec: &str, desc: &str) -> String {
    let mut out = format!("  {}", spec);
    let used = super::width::cols(spec);
    if used < 24 {
        out.push_str(&" ".repeat(24 - used));
    } else {
        out.push_str("  ");
    }
    out.push_str(desc);
    out
}

/// 目的：解析失败时的「未知旗语」文案：编辑距离 ≤2 就给最近的那条建议。
fn unknown(name: &str) -> String {
    match nearest(name) {
        Some(near) => format!(
            "未知旗语 {}；你是不是想用 {}？（--help 看全部旗语）",
            name, near
        ),
        None => format!("未知旗语 {}；--help 看全部旗语", name),
    }
}

fn nearest(name: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for flag in FLAGS {
        for candidate in std::iter::once(flag.long).chain(flag.short) {
            let d = distance(name, candidate);
            if d <= 2 && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((candidate, d));
            }
        }
    }
    best.map(|(name, _)| name)
}

fn take(
    inline: Option<String>,
    args: &[String],
    i: &mut usize,
    name: &str,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value);
    }
    match args.get(*i + 1) {
        Some(value) if !value.starts_with('-') => {
            *i += 1;
            Ok(value.clone())
        }
        _ => Err(format!(
            "旗语 {} 需要一个值（写法：{} <值> 或 {}=<值>）",
            name, name, name
        )),
    }
}

fn no_value(inline: &Option<String>, name: &str) -> Result<(), String> {
    match inline {
        Some(_) => Err(format!("旗语 {} 不带值（布尔开关直接写它本身）", name)),
        None => Ok(()),
    }
}

/// 目的：两个词的编辑距离（只用于旗语建议，长度都短）。
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Flags, String> {
        let argv: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        super::parse(&argv)
    }

    /// 单个 `-` 与 `--` 之后的内容都进位置参数；`--x=` 的空值按空串收下。
    #[test]
    fn positionals_and_empty_values_are_collected() {
        let flags = run(&["-", "--", "--web", "-v"]).expect("合法");
        assert!(!flags.web && !flags.version, "-- 之后不再解析旗语");
        assert_eq!(flags.positionals, vec!["-", "--web", "-v"]);

        let flags = run(&["--root=", "x"]).expect("空值合法");
        assert_eq!(flags.root.as_deref(), Some(""), "空串也是一个值");
        assert_eq!(flags.positionals, vec!["x"]);
    }

    /// 端口只收 1..=65535 的整数：非数字与越界都报错，两种写法等价。
    #[test]
    fn port_must_be_a_u16() {
        assert_eq!(run(&["--port=3081"]).expect("合法端口").port, Some(3081));
        assert_eq!(run(&["--port", "3082"]).expect("空格写法").port, Some(3082));
        let err = run(&["--port=abc"]).expect_err("非数字要报错");
        assert!(err.contains("1-65535") && err.contains("abc"), "{err}");
        let err = run(&["--port=70000"]).expect_err("越界要报错");
        assert!(err.contains("70000"), "{err}");
        let err = run(&["--port="]).expect_err("空值要报错");
        assert!(err.contains("旗语 --port"), "{err}");
    }

    /// 布尔开关不带值：写了 `=值` 就报错；重复写同一个开关没有副作用。
    #[test]
    fn boolean_flags_take_no_value_and_repeat_safely() {
        let flags = run(&["--web", "--web", "--plain", "--plain", "--no-anim"]).expect("合法");
        assert!(flags.web && flags.plain && flags.no_anim);
        let err = run(&["--plain=1"]).expect_err("布尔旗不带值");
        assert!(err.contains("不带值"), "{err}");
        assert!(run(&["-h"]).expect("短旗").help);
        assert!(run(&["-v"]).expect("短旗").version);
        assert!(run(&["--version"]).expect("长旗").version);
        assert!(run(&["--help", "--all"]).expect("展开内部旗语").all);
        assert!(!run(&["--all"]).expect("单独出现无副作用").help);
    }

    /// 缺值：后面没有值、或紧跟另一个旗语，都报错，不把旗语当值。
    #[test]
    fn value_flags_report_missing_values() {
        let err = run(&["--root"]).expect_err("缺值");
        assert!(err.contains("需要一个值"), "{err}");
        let err = run(&["--root", "-x"]).expect_err("下一个是旗语，不能当值");
        assert!(err.contains("需要一个值"), "{err}");
        assert_eq!(
            run(&["--root", "work"]).expect("空格写法").root.as_deref(),
            Some("work")
        );
        assert_eq!(
            run(&["--root=work"]).expect("= 写法").root.as_deref(),
            Some("work")
        );
    }

    /// 未知旗语：编辑距离 ≤2 给最近的一条建议（短旗也算），否则只说看手册。
    #[test]
    fn unknown_flags_suggest_or_just_point_at_help() {
        let err = run(&["--por", "1"]).expect_err("拼错");
        assert!(err.contains("你是不是想用 --port"), "{err}");
        let err = run(&["--h"]).expect_err("短旗也算候选");
        assert!(err.contains("你是不是想用 -h"), "{err}");
        let err = run(&["--zzzzzz"]).expect_err("差太远");
        assert!(
            err.contains("未知旗语 --zzzzzz") && !err.contains("你是不是想用"),
            "{err}"
        );
    }

    /// 内部协议旗语原样透传：解析器不再往下列，整段 argv 留给入口自己扫。
    #[test]
    fn internal_flags_pass_through_untouched() {
        let flags = run(&["--probe-tools", "--model", "x"]).expect("透传");
        assert_eq!(flags.positionals, vec!["--probe-tools", "--model", "x"]);
        let flags = run(&["--web", "--fence-verify", "--json"]).expect("--fence-* 也透传");
        assert!(flags.web, "前面的旗语照常解析");
        assert_eq!(flags.positionals, vec!["--fence-verify", "--json"]);
    }

    /// 维护旗：`--setup=path` 取值、单独出现 = status、紧跟已知模式名也收、空值报错。
    #[test]
    fn setup_flag_takes_mode_or_defaults_to_status() {
        assert_eq!(
            run(&["--setup=path"]).expect("取值").setup.as_deref(),
            Some("path")
        );
        assert_eq!(
            run(&["--setup"]).expect("省略值").setup.as_deref(),
            Some("status")
        );
        assert_eq!(
            run(&["--setup", "fence"])
                .expect("空格写法")
                .setup
                .as_deref(),
            Some("fence")
        );
        // 未知模式留给分派去报（解析层不猜），根旗语照常解析。
        let flags = run(&["--setup", "--root", "work"]).expect("不吃旗语当值");
        assert_eq!(flags.setup.as_deref(), Some("status"));
        assert_eq!(flags.root.as_deref(), Some("work"));
        let err = run(&["--setup="]).expect_err("空值要报错");
        assert!(err.contains("--setup="), "{err}");
    }

    /// 台账维护旗不是 opaque 协议：后面的 `--root` 要照常解析（端到端收尾用 `--fence-clean --root <夹具>`）。
    #[test]
    fn fence_maintenance_flags_keep_parsing_root() {
        let flags = run(&["--fence-clean", "--root", "fixture"]).expect("合法");
        assert_eq!(flags.root.as_deref(), Some("fixture"));
        let flags = run(&["--fence-reconcile", "--root=fixture"]).expect("合法");
        assert_eq!(flags.root.as_deref(), Some("fixture"));
        assert!(
            run(&["--fence-ledger", "--human"]).is_ok(),
            "可读版修饰符要能解析"
        );
    }

    /// 旗语表：默认只列用户可见的那些，`all` 时连内部协议一起列。
    #[test]
    fn flag_rows_lists_visible_and_internal_flags() {
        let visible = flag_rows(false);
        assert_eq!(visible.len(), FLAGS.iter().filter(|f| !f.hidden).count());
        assert!(visible.iter().any(|r| r.starts_with("  --root <dir>")));
        assert!(visible.iter().any(|r| r.starts_with("  --web ")));
        assert!(visible.iter().any(|r| r.starts_with("  -h, --help ")));
        assert!(visible.iter().any(|r| r.starts_with("  --setup <mode>")));
        assert!(!visible.iter().any(|r| r.contains("--probe-tools")));
        let all = flag_rows(true);
        assert!(all.iter().any(|r| r.contains("--probe-tools")));
        assert!(all.iter().any(|r| r.contains("内部协议")));
        assert!(all.len() > visible.len());
    }

    /// 旗语行：占位符不足 24 列按空格补齐，超过就隔两个空格接说明。
    #[test]
    fn row_pads_short_specs_and_separates_long_ones() {
        let short = row("--web", "说明");
        assert!(short.starts_with("  --web "), "{short}");
        assert_eq!(
            crate::presentation::cli::width::cols(&short),
            2 + 24 + 4,
            "短占位符补到 24 列再接说明"
        );
        let long = row("--a-very-long-flag <value>", "说明");
        assert!(
            long.starts_with("  --a-very-long-flag <value>  说明"),
            "{long}"
        );
    }
}
