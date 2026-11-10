# web（本地网页）

> 定位：本地 HTTP + 长轮询；路由目录是机器比对的锚点。
> 效力：规范｜核对：2026-11-10｜依据：`src/presentation/web/routes.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../../TESTING.md)。

## 管什么 / 不管什么

- 管：本地 HTTP（tiny_http，**只绑 `127.0.0.1`**）+ 长轮询增量推送 + HTTP 路由目录 + 浏览器端资源与冒烟（`assets/` 下递归发现的 `*.smoke.cjs` / `*.smoke.mjs`）。
- 管：浏览器端覆盖三种工作形态（single / collab / **proxy**）的建工作向导与会话视图：代理形态不问名单与需求（选它就是授予全权），会话说明与侧栏标出「已停止 / 已关闭」（持久运行态）。
- 管：转录里的**内容类系统提示**是 `notice` 行（有会话内 id、可回档 / 折叠）；过程性提示、工具层确认与基础设施故障仍是瞬时事件——前端两种都按系统行画，只有前者带 id（见 [session-model.md](../../session/session-model.md) 的「系统消息」）。
- **一轮 = 一条回复**：同 `reply` 的正文 / 思维链 / 工具行合成一个气泡（身份 / 思维链 / 正文 / 工具组，整轮可折叠）；定稿渲染只**追加**新行，同一回复的后续工具行只重建最后那一轮；流式块也按同一轮容器渲染（消息 + 随后的工具调用），增量按 `requestAnimationFrame` 合并。**工具卡只有「参数 / 结果」**；助手原文是回复级事实，整轮气泡里只显示一次（原生模式下与可见正文相同则不重复）。
- **输入区上方是会话状态栏**：最多三个状态 chip（图标 + 简要信息，点击展开锚定小卡片）——「上下文」显示最近一次调用的精确 token 数 / 窗口百分比（供应商没回 usage 时标「≈ 估算」；协作主会话没有持续对话，不显示），「共享区」显示当前 head 与相对时间。
- 「shell」在跑过 shell 后出现：运行中显示耗时（每秒更新），结束后按 `ShellLive.status` 分四种 chip 文案：`shell 退出码 N · 耗时` / `shell 超时 · 耗时` / `shell 停止 · 耗时` / `shell 中止（信号）· 耗时`；点开看命令、scope、cwd、状态、退出码、耗时与回执预览，卡片「状态」行同样区分成功 / 超时 / 用户停止 / 被信号结束（完整回执在转录的 shell 工具行）。
- 历史回放 / 重建的会话按转录派生同一份快照（chip 加「（历史）」，cwd 与耗时取自当时的回执，cwd 是本环境写法：虚拟机档是 guest 内路径）；运行中打开的卡片随状态刷新重画，不用重开。
- 「全部提交」打开提交历史（分页、`on_head` 标记、提交详情、文本文件 16 KiB 预览），选中提交可**非破坏性回滚**（目标之后的提交仍保留；`expect_head` 防陈旧，运行中的工作子树拒绝）。回滚成功落一条 `notice` 行，刷新 / 回放仍可见（见 [workspace/README.md](../../workspace/README.md) 与 [session-model.md](../../session/session-model.md)）。
- 另外管**依赖补齐的界面入口**：虚拟机档不可用、或选中的虚拟根未装齐时，配置面板与创建向导给出「补齐虚拟机依赖」（看清单与体积 → 确认 → 按 `seq` 长轮询进度 → 如实说清生效范围）；要装哪些条目由后端 `/api/setup/status` 的 `pending` 决定，界面不自己挑。
- 「资源与进程」页还列**现在在跑什么**（`GET /api/processes`：产品起的 QEMU / 共享进程 / 常驻服务守门进程与它们的归属；`alive=false` 如实标成「已退出，等对账清」，`orphanRisk` 标出父死子亡没挂上、核心被强杀会留下的那条）。
- 「存储空间」页给可回收缓存的大小、上限、自动回收策略、整卷 / 按模块清理、压缩、清空下载缓存 / 可写层 / 日志，以及虚拟根「还原到初始状态」（红字警告 + 二次确认；不备份）。
- **侧栏是会话树**：根会话可展开子会话，活跃路径自动展开；点条目直接打开会话，行尾铅笔打开「打开 / 编辑配置 / 删除」（子会话多一项「打开父会话」，不给删除）。桌面可把侧栏收成 64px 图标栏（偏好记 `localStorage` 的 `solomni.ui.sidebar`），移动端仍是抽屉。
- **标签是梯级**：第 1 行工作，第 2 行是当前工作的子会话，第 3 行是当前子会话的孩子……只沿活跃路径展开；未打开的孩子弱化显示（点开才加载），裁决角标向上聚合；关闭根标签连同它的子标签视图一起关（只关视图，不删数据）。
- **节点是执行进度**，不复用系统消息：独立卡片显示进行中 / 完成、执行人、返工轮次、回报全文与「进入子会话」；只有回报没有开工时建一张完成卡，不给入口。
- **设置中心**在侧栏底部：左侧 9 页（生成与上下文 / 沙箱围栏 / 权限默认值 / 资源与进程 / 存储空间 / 供应商 / 模型 / agent / 模块与服务），前 5 页共用一份保存（界面一次提交这些页的当前值；`set_settings` 本身是部分更新，未提交字段保留现值），后 4 页是即时操作。
- **模型表单的工具调用选择**：默认「原生工具调用」，保存前实测（通道基线 + tools + 回放形状），明确通过才保存；「手写信封」是显式选择，不测试直接保存。原生实测不通过时只如实报原因，不引导改信封。
- **agent 登记与向导共用一套卡片和弹层**：设置页 agent 管理是「列表 + 新建按钮」，新建 / 编辑在弹层里；名字、人设（`chara`）、模块、默认模型、说明都在同一份表单里。模块不再平铺勾选框，而是「已选 chips + 添加模块」→ 弹层里上方「已加入」始终可见、不参与搜索（点卡片移除），下方「可选」可搜索（只匹配模块名字与简介，点卡片加入）→ 确定应用；向导里的单 agent / 协作也走同一套卡片，临时 agent 点加号新建、从登记处加入是另一个入口。弹层是**叠层**的：向导 → agent 弹层 → 模块选择器逐层打开 / 关闭，下层已填内容不丢；Esc 或点遮罩只关最上面一层。
- **弹层关闭判据**：按下与松开**都在遮罩上**才关（面板内拖选到外面松手不误关）；Esc 只关最上面一层（`ui.bindDismiss` 的弹层栈）。
- **行内动作**（回档 / 恢复 / 撤回）是图标按钮，hover 或键盘聚焦浮现、触屏常显，说明走原生 title（长说明不会被滚动容器裁掉）；一次性结果（已停止 / 已回档 / 已删除 / 已上传 / 设置已保存 / 已启用 / 已改需求）用 toast 反馈。
- **围栏横幅**：`/api/state` 的 `fence` 给 `effective_fs/net`、`write_allowed`、`sandbox_missing`、`sandbox_degraded`；前端只在 `sandbox_degraded` 或 `sandbox_missing` 非空时显示（降级黄色 / 要求未满足红色），正常与用户主动选的 `best_effort` 不显示。保存策略后整体重启并重算，横幅随新状态显示或消失；无 TTY 且要求未满足时启动门退出，恢复走控制台或 `SOLOMNI_FENCE_WRITE=1`。红色「要求未满足」是 **bug 指示器**：保存 / 加载会拒绝 `require + write_acl=false`，正常运行的 Web 不会有这一态。
- 不管：不做业务判断、不持状态、不落盘；**永不接触端口对象，也拿不到 `Core` 本身**。

## 视觉、文案与开发陈列

- **只做深色**：色板与语义层都在 `src/presentation/web/assets/tokens.css`（中性色阶 + 5 档字号 + 4/8 间距 + 3 档阴影 + 动效时长）；`src/presentation/web/assets/ui.css` 与 `src/presentation/web/assets/style.css` 只用 `var(--…)`，不得出现字面量颜色、时长，也不得写字面量 `font-size` / `line-height` / `font-family`。判据是 `src/presentation/web/assets/tokens.smoke.cjs`。
- **对比度是门禁**：同一脚本对 23 组前景 / 背景按 WCAG 复算，正文 ≥ 4.5:1、控件边界与焦点 ≥ 3:1；色值或配对改了没重算，`frontend-smoke` 直接失败。`--color-bg` 这个名字被 `src/tests/routes.rs` 钉住，改名要同改测试。
- **图标只有一处来源**：`src/presentation/web/assets/app/ui/icons.js` 的 `ICONS`（内联 SVG、`stroke="currentColor"`、`aria-hidden="true"`）；字符字形不当图标用，图标按钮必须有 `aria-label` 或 `title`。
- **文案事实不删、只换位置**：首层只留做决定所需的信息（`lead` ≤ 40 汉字），原理与影响范围收进 `ui.detail`（短说明 → ⓘ 弹层，长说明 → 原生 `details` 的「详情」）；安全、不可逆、生效范围类文案用 `safety` 常显、不折叠、不截断。每个有首层文案的功能一份 `copy.js`（纯数据，不是组件），`src/presentation/web/assets/copy.smoke.cjs` 禁止 `copy.js` 之外出现 ≥ 40 汉字的内联中文。
- **动效只表示「正在工作」**：转录轮左缘 2px 状态轨在生成 / 工具执行时呼吸，完成收成 1px 中性线、失败转红；弹层进出 120ms。`prefers-reduced-motion: reduce` 下 `tokens.css` 把全部时长归零，不另写分支。
- **组件陈列页是开发工具**：`src/presentation/web/assets/dev/gallery.html` 经 `/assets/dev/gallery.html` 访问，只在 debug 构建嵌入（`build.rs` 的 `DEV_ONLY_DIRS` + `PROFILE`；`src/tests/routes.rs` 的 `dev_assets_follow_build_profile` 双向断言），发行版不带；`dev/gallery.smoke.cjs` 校验它引用的是真实组件工厂、不写死颜色、不引外网。

## 入站契约与状态归属

- 入站面：`src/presentation/web/routes.rs` 的 `ROUTES` 是 HTTP 入站契约的**唯一定义**，与 [contracts.md](../contracts.md) 的路由表机器比对（`src/tests/routes.rs`）。
- 入口：`src/main.rs` 调 `presentation::web::serve(ops, port, fence)`；默认端口是 `presentation::web::DEFAULT_PORT`（`webui` 与启动参数共用这一个来源）。
- 状态归属：不持状态、不落盘；页面偏好只记浏览器 `localStorage`（如侧栏的 `solomni.ui.sidebar`）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`conductor`（`Ops` 字段里带 `provision` 面）
- 谁在用我：（无）

## 一、前端分层

- 入口只有一个：`src/presentation/web/assets/index.html` 只放挂载点，加载 `src/presentation/web/assets/app/main.js`（`type="module"`）与 `src/presentation/web/assets/md.js`；`src/presentation/web/assets/app/main.js` 是唯一的组装点（建共享上下文、按顺序 `mount`、启动接线），页面没有全局业务函数。
- 目录即依赖方向：`src/presentation/web/assets/app/core/` 不依赖其它层；`src/presentation/web/assets/app/stores/` 与 `src/presentation/web/assets/app/ui/` 只依赖 `core/`；`src/presentation/web/assets/app/features/` 下的每个功能只依赖 `core/`、`stores/`、`ui/` 与自己的目录，功能之间互不 import——跨功能调用经装配点注入的晚绑定入口。
- 判定：`tests/gates/structure/webLayers.js`（T0 结构审查的 `structure/web-layers`）扫描 `src/presentation/web/assets/app/**/*.js` 的 import 方向与 `src/presentation/web/assets/index.html` 的入口脚本；文件 400 行只是回执软线，超线不判失败。

## 改动本单元时必须同步

- 增删路由 → 五处同改：`src/presentation/web/routes.rs` 的 `ROUTES`、前端调用点、`docs/presentation/contracts.md` 的 ROUTES 段、`src/tests/routes.rs`（机器比对）、`demo/*.mjs` 的调用点；前端资源改动跑冒烟。
- 增删前端文件 → **不改路由**：`build.rs` 编译期扫描 `assets/` 自动进嵌入表（经 `/assets/{*path}` 提供）；新扩展名要先加进 `build.rs` 的 `SERVED`，否则构建失败。
- 改令牌、字号阶梯、间距或对比度配对 → 同改 `tokens.smoke.cjs`（5 档字号严格递增且 `--fs-base: 16px`、`--sp-*` 是 4 的倍数、颜色令牌为不透明 hex、未引用令牌失败、23 组对比度）；改组件外观 → 同步 `assets/dev/gallery` 的对应分区并跑 `gallery.smoke.cjs`。
- 改首层文案 → 落进该功能的 `copy.js`（`lead` / `detail` / `safety` 三段），跑 `copy.smoke.cjs`；不要为了省事把长说明塞回首层。
- 增删或移动前端模块 → 依赖方向由 `tests/gates/structure/webLayers.js` 判定（`structure/web-layers`），并跑 `node run-tests.js` 的 `frontend-smoke`；分区职责改完同步本页与 [`unit-map.md`](unit-map.md)。
- 测试缺口：本单元**不是业务能力**，没有独立的缺口账；缺口记在 `tests/gaps.yaml`；格式见 [docs/testing/gaps-acceptance.md](../../../docs/testing/gaps-acceptance.md) 的「缺口账」。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
