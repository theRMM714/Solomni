# 目的：回执说明的唯一文案源——每条消息的 id 是 kernel 码表的稳定码把 "." 换成 "-"（码表见 src/kernel/contracts/note.rs）。
# 管：模型通道与界面共用的短句；占位符 { $name } 由结构化的 args 代入。
# 不管：子进程自己的原始输出（原样进 payload）、日志（只有码）。
# 约束：一行一条简单消息，不用 select / plural / 多行值——Web 侧用同一份文件做极简解析（不引 @fluent/bundle）；id 用连字符（Fluent 的 id 不认 "."）。
# 联动：src/kernel/detail/i18n.rs（Rust 渲染 + 目录导出）；新增码时同次补一条，测试按码表点名。

shell-exit = [shell] 退出码 { $code }
shell-signal = [shell] 退出码未知（被信号结束）
shell-timeout = [shell] 超时（{ $secs } 秒）：工具进程与它拉起的整棵进程树已被终止
shell-cancelled = [shell] 用户停止：工具进程与它拉起的整棵进程树已被终止
shell-place = [shell] cwd { $cwd } · 耗时 { $secs } 秒
shell-place-prep = [shell] 围栏准备 { $prep } 秒（已含在耗时里）
shell-shared-exec = [shell] 共享区不承载执行位：{ $script } 不能直接执行，用解释器跑（例如 bash { $script } 或 sh { $script }）
run-timeout = [超时] 工具进程与它拉起的整棵进程树已被终止
run-spawn-failed = 工具进程启动失败：{ $error }
fence-failed = [围栏] 工具进程没有进入围栏：命令未执行（当前能力等级见启动报告）
result-truncated = [结果截断] 输出共 { $total } 字符；保留开头 { $head } 与结尾 { $tail } 字符。
result-truncated-command = 需要更多请缩小命令的输出（只取要看的行、或用 head/tail 之类的筛选），或把输出写到文件再用 read 分段查看。
result-truncated-read = 需要更多请用 offset/limit 分段读取。
result-truncated-context = [上下文将满] 本次只按剩余空间保留 { $limit } 字符；继续前请先压缩会话，或分页读取。

# 分级标签：CLI 把一次调用归成三态时用它们（界面用图标与颜色，不用这些词）。
cli-ok = 成功
cli-warning = 警告
cli-error = 失败
