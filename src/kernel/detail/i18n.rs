//! 目的：把回执说明渲染成给人读的文本——文案唯一源在 `locales/<lang>.ftl`，码表在 kernel 契约里。
//! 管：语言解析（设置 → `SOLOMNI_LANG` → 缺省 zh-CN）、同一份目录供 Rust 与界面使用、说明与正文的渲染。
//! 不管：界面自己的文案（Web 的 `copy.js`，另见 tests/gaps.yaml 的 P1 缺口）；业务层文案；语言设置的落盘。
//! 联动：`kernel::contracts::note`（码与轻重）；会话的消息构造、CLI 与 Web 的展示都从这里取文本。

use crate::kernel::contracts::note::{Note, Severity, codes};
use crate::kernel::detail::io::env;
use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use unic_langid::LanguageIdentifier;

/// 目的：源语言——码表文案以它为准；模型通道固定用它，界面语言与它无关。
pub const SOURCE_LANG: &str = "zh-CN";

/// 目的：环境变量名——没在设置里选语言时按它协商（与常见的 LC_*/LANG 习惯一致）。
pub const ENV_LANG: &str = "SOLOMNI_LANG";

/// 目的：源语言目录——构建期嵌入，发行包不依赖外部文件。
const CATALOG: &str = include_str!("../../../locales/zh-CN.ftl");

/// 目的：可用的界面语言（今天只有 zh-CN；加语言 = 加一条 `locales/<tag>.ftl` 并登记在这里）。
const AVAILABLE: &[&str] = &[SOURCE_LANG];

/// 目的：一种**可展示**的语言——只由 `available()` 里的标签造出来，拒绝任意字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lang(&'static str);

impl Lang {
    /// 目的：源语言（zh-CN）——缺省与兜底都用它。
    pub const fn source() -> Lang {
        Lang(SOURCE_LANG)
    }

    /// 目的：稳定的语言标签（写进设置、HTTP 响应与日志）。
    pub fn tag(self) -> &'static str {
        self.0
    }
}

/// 目的：可用的界面语言标签。
pub fn available() -> &'static [&'static str] {
    AVAILABLE
}

/// 目的：判断一个标签是不是可用语言（大小写不敏感）——设置校验用它，别把任意字符串写进设置。
pub fn is_available(tag: &str) -> bool {
    let want = tag.trim();
    !want.is_empty() && AVAILABLE.iter().any(|t| t.eq_ignore_ascii_case(want))
}

/// 目的：把请求的语言协商成可用语言——先精确匹配，再按主语言（`zh` → `zh-CN`）兜底，最后回源语言。
pub fn resolve(requested: &str) -> Lang {
    let want = requested.trim();
    if want.is_empty() {
        return Lang::source();
    }
    for tag in AVAILABLE {
        if tag.eq_ignore_ascii_case(want) {
            return Lang(tag);
        }
    }
    let base = want.split(['-', '_']).next().unwrap_or_default();
    for tag in AVAILABLE {
        if tag
            .split('-')
            .next()
            .unwrap_or_default()
            .eq_ignore_ascii_case(base)
        {
            return Lang(tag);
        }
    }
    Lang::source()
}

/// 目的：按「设置 → 环境变量 → 缺省」定界面语言；两处都空就是源语言。
pub fn pick_with(settings: &str, env: &str) -> Lang {
    if !settings.trim().is_empty() {
        return resolve(settings);
    }
    if !env.trim().is_empty() {
        return resolve(env);
    }
    Lang::source()
}

/// 目的：同上，环境变量从进程里读。
pub fn pick(settings: &str) -> Lang {
    pick_with(settings, &env::var(ENV_LANG).unwrap_or_default())
}

/// 目的：取某语言的目录——今天只有 zh-CN 一份；加语言时在这里按标签分派。
fn catalog_for(_lang: Lang) -> &'static BTreeMap<&'static str, &'static str> {
    catalog()
}

/// 目的：目录解析结果——`id` → 单行模板源文本。
/// 约束：只认 `id = 文本` 的一行式消息；`#` 开头是注释，空行跳过（与 `locales/*.ftl` 的写法约定一致）。
fn catalog() -> &'static BTreeMap<&'static str, &'static str> {
    static MAP: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut out = BTreeMap::new();
        for line in CATALOG.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((id, text)) = line.split_once(" = ") else {
                continue;
            };
            out.insert(id.trim(), text.trim());
        }
        out
    })
}

/// 目的：在**本线程**的 Fluent 运行时上做事；没有就先装一份。
/// 约束：`FluentBundle` 不是 `Sync`（内部类型表是 `RefCell`），所以每线程各持一份，目录也只在本线程解析一次。
fn with_bundle<R>(f: impl FnOnce(&FluentBundle<FluentResource>) -> R) -> R {
    thread_local! {
        static BUNDLE: std::cell::RefCell<Option<FluentBundle<FluentResource>>> =
            const { std::cell::RefCell::new(None) };
    }
    BUNDLE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(build_bundle());
        }
        f(slot.as_ref().expect("刚刚装好目录"))
    })
}

/// 目的：解析目录、造一份 bundle——解析失败属于构建缺陷，直接 panic 暴露，不静默降级。
fn build_bundle() -> FluentBundle<FluentResource> {
    let resource =
        FluentResource::try_new(CATALOG.to_string()).expect("locales/zh-CN.ftl 必须语法正确");
    let lang: LanguageIdentifier = SOURCE_LANG.parse().expect("zh-CN 必须是合法语言标签");
    let mut bundle = FluentBundle::new(vec![lang]);
    // 关掉双向文本隔离：Fluent 默认给代入值包上 U+2068/U+2069，会把路径与数字的明文弄脏
    // （模型通道与 CLI 都按纯文本读，这两处不是 HTML 排版）。
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .expect("locales/zh-CN.ftl 不能有重复消息");
    bundle
}

/// 目的：稳定码 → Fluent 消息 id——Fluent 的 id 只认 ASCII 字母、数字、`_`、`-`，点分码要换写法。
/// 约束：只做一对一替换（`.` → `-`），两种写法都不含别的字符。
fn fluent_id(code: &str) -> String {
    code.replace('.', "-")
}

/// 目的：导出整份目录——界面拿它给回执说明上文案，不必自己复制一份词表。
/// 约束：键是稳定码，值是**源语言**模板；界面语言变时换的是目录，不是码。
pub fn messages(lang: Lang) -> Vec<(&'static str, &'static str)> {
    let catalog = catalog_for(lang);
    codes::ALL
        .iter()
        .filter_map(|code| {
            catalog
                .get(fluent_id(code).as_str())
                .map(|text| (*code, *text))
        })
        .collect()
}

/// 目的：渲染一条说明——按参数代入模板；目录里没有这条码时退回 `[code]`（显眼但不崩）。
pub fn render(lang: Lang, note: &Note) -> String {
    debug_assert!(is_available(lang.tag()), "渲染只接受已登记的语言");
    with_bundle(|bundle| {
        let Some(message) = bundle.get_message(fluent_id(note.code.as_str()).as_str()) else {
            return format!("[{}]", note.code);
        };
        let Some(rendered) = message.value() else {
            return format!("[{}]", note.code);
        };
        let mut args = FluentArgs::new();
        for (key, value) in &note.args {
            args.set(key.as_str(), value.as_str());
        }
        let mut errors = Vec::new();
        bundle
            .format_pattern(rendered, Some(&args), &mut errors)
            .to_string()
    })
}

/// 目的：把一组说明渲染成多行文本（每行一条，顺序即给的顺序）。
pub fn render_notes(lang: Lang, notes: &[Note]) -> String {
    notes
        .iter()
        .map(|note| render(lang, note))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 目的：把「原始输出 + 结构化说明」拼成一段文本——正文在前，说明逐行在后，空段不留空行。
/// 约束：正文与说明之间的分隔就是换行；正文自带的行尾换行会形成空行（与旧回执形状一致）。
pub fn render_receipt(lang: Lang, payload: &str, notes: &[Note]) -> String {
    let body = render_notes(lang, notes);
    match (payload.is_empty(), body.is_empty()) {
        (true, _) => body,
        (false, true) => payload.to_string(),
        (false, false) => format!("{payload}\n{body}"),
    }
}

/// 目的：三档分级的中文标签（CLI 归成「成功 / 警告 / 失败」时用）。
pub fn label(lang: Lang, severity: Severity) -> String {
    let code = match severity {
        Severity::Info => codes::LABEL_OK,
        Severity::Warning => codes::LABEL_WARNING,
        Severity::Error => codes::LABEL_ERROR,
    };
    render(lang, &Note::info(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::domain::receipt::{ShellEnd, shell_receipt};

    fn zh() -> Lang {
        Lang::source()
    }

    /// 目的：码表里的每条码都在目录里，且不是空文本——新增码漏写目录由这条点名。
    #[test]
    fn catalog_covers_every_code() {
        let listed = messages(zh());
        assert_eq!(listed.len(), codes::ALL.len(), "目录必须覆盖整张码表");
        for (code, text) in listed {
            assert!(!text.is_empty(), "目录条目不能为空：{}", code);
        }
    }

    /// 目的：shell 收尾说明渲染出的文本与改造前的回执逐字一致（模型侧的读法不变）。
    #[test]
    fn shell_end_notes_render_like_the_old_receipt() {
        let done = shell_receipt("hello\n", "warn\n", ShellEnd::Exit(Some(3)));
        assert_eq!(
            render_receipt(zh(), &done.payload, &done.notes),
            "hello\n\n[stderr]\nwarn\n\n[shell] 退出码 3"
        );
        let timed = shell_receipt("", "", ShellEnd::TimedOut { secs: 2 });
        assert_eq!(
            render_receipt(zh(), &timed.payload, &timed.notes),
            "[shell] 超时（2 秒）：工具进程与它拉起的整棵进程树已被终止"
        );
        let stopped = shell_receipt("", "", ShellEnd::Cancelled);
        assert_eq!(
            render_receipt(zh(), &stopped.payload, &stopped.notes),
            "[shell] 用户停止：工具进程与它拉起的整棵进程树已被终止"
        );
        let signal = shell_receipt("", "", ShellEnd::Exit(None));
        assert_eq!(
            render_receipt(zh(), &signal.payload, &signal.notes),
            "[shell] 退出码未知（被信号结束）"
        );
    }

    /// 目的：正文为空时只出说明；说明为空时只出正文。
    #[test]
    fn rendering_skips_empty_parts() {
        assert_eq!(
            render_receipt(zh(), "", &[Note::error(codes::FENCE_FAILED)]),
            "[围栏] 工具进程没有进入围栏：命令未执行（当前能力等级见启动报告）"
        );
        assert_eq!(render_receipt(zh(), "plain", &[]), "plain");
    }

    /// 目的：参数逐个代入（同一个参数出现在多处也一致）。
    #[test]
    fn args_are_substituted_in_place() {
        let note = Note::warning(codes::SHELL_SHARED_EXEC).arg("script", "./run.sh");
        assert_eq!(
            render(zh(), &note),
            "[shell] 共享区不承载执行位：./run.sh 不能直接执行，用解释器跑（例如 bash ./run.sh 或 sh ./run.sh）"
        );
        let place = Note::info(codes::SHELL_PLACE)
            .arg("cwd", "/work")
            .arg("secs", 3);
        assert_eq!(render(zh(), &place), "[shell] cwd /work · 耗时 3 秒");
        let prep = Note::info(codes::SHELL_PLACE_PREP).arg("prep", 2);
        assert_eq!(render(zh(), &prep), "[shell] 围栏准备 2 秒（已含在耗时里）");
    }

    /// 目的：目录里没有的码退回 `[code]`——显眼地暴露漏配，而不是静默空白。
    #[test]
    fn unknown_codes_render_as_the_code() {
        assert_eq!(render(zh(), &Note::info("nope.missing")), "[nope.missing]");
    }

    /// 目的：语言协商——精确匹配 > 主语言 > 源语言；设置优先于环境变量。
    #[test]
    fn language_negotiation_prefers_settings_then_env_then_source() {
        assert_eq!(resolve("zh-CN").tag(), "zh-CN");
        assert_eq!(resolve("ZH-cn").tag(), "zh-CN");
        assert_eq!(resolve("zh").tag(), "zh-CN");
        assert_eq!(resolve("en-US").tag(), SOURCE_LANG);
        assert_eq!(pick_with("", "").tag(), SOURCE_LANG);
        assert_eq!(pick_with("en", "zh-CN").tag(), SOURCE_LANG);
        assert_eq!(pick_with("zh-CN", "en").tag(), "zh-CN");
        assert!(is_available("zh-CN") && !is_available("en-US") && !is_available(""));
        assert_eq!(label(zh(), Severity::Info), "成功");
        assert_eq!(label(zh(), Severity::Warning), "警告");
        assert_eq!(label(zh(), Severity::Error), "失败");
    }
}
