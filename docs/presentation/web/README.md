# web（本地网页）

> 本地 HTTP + 长轮询；路由目录是机器比对的锚点。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：本地 HTTP（tiny_http，**只绑 `127.0.0.1`**）+ 长轮询增量推送 + HTTP 路由目录 + 浏览器端资源与冒烟（`assets/*.smoke.cjs`）。浏览器端覆盖三种工作形态（single / collab / **proxy**）的建工作向导与会话视图：代理形态不问名单与需求（选它就是授予全权），会话说明与侧栏标出"已停止 / 已关闭"（持久运行态）。
转录里的**内容类系统提示**是 `notice` 行（有会话内 id、可回档/折叠）；过程性提示、工具层确认与基础设施故障仍是瞬时事件——前端两种都按系统行画，只有前者带 id（见 [session-model.md](../../session/session-model.md) 的「系统消息」）。
**一轮 = 一条回复**：同 `reply` 的正文 / 思维链 / 工具行合成一个气泡（身份 / 思维链 / 正文 / 工具组，整轮可折叠）；定稿渲染只**追加**新行，同一回复的后续工具行只重建最后那一轮；流式块也按同一轮容器渲染（消息 + 随后的工具调用），增量按 `requestAnimationFrame` 合并。
另外管**依赖补齐的界面入口**：虚拟机档不可用时，配置面板与创建向导给出「补齐虚拟机依赖」（看清单与体积 → 确认 → 按 `seq` 长轮询进度 → 如实说清生效范围）；
要装哪些条目由后端 `/api/setup/status` 的 `pending` 决定，界面不自己挑。
「资源与进程」页还列**现在在跑什么**（`GET /api/processes`：产品起的 QEMU / 共享进程 / 常驻服务守门进程与它们的归属；`alive=false` 如实标成「已退出，等对账清」，`orphanRisk` 标出父死子亡没挂上、核心被强杀会留下的那条）。
**侧栏是会话树**：根会话可展开子会话，活跃路径自动展开；行尾铅笔打开「打开 / 编辑配置 / 删除」（子会话多一项「打开父会话」，不给删除）。桌面可把侧栏收成 64px 图标栏（偏好记 `localStorage` 的 `solomni.ui.sidebar`），移动端仍是抽屉。
**标签是梯级**：第 1 行工作，第 2 行是当前工作的子会话，第 3 行是当前子会话的孩子……只沿活跃路径展开；未打开的孩子弱化显示（点开才加载），裁决角标向上聚合；关闭根标签连同它的子标签视图一起关（只关视图，不删数据）。
**节点是执行进度**，不复用系统消息：独立卡片显示进行中 / 完成、执行人、返工轮次、回报全文与「进入子会话」；只有回报没有开工时建一张完成卡，不给入口。
**设置中心**在侧栏底部：左侧 8 页（生成与上下文 / 沙箱围栏 / 权限默认值 / 资源与进程 / 供应商 / 模型 / agent / 模块与服务），1–4 页共用一份保存（`set_settings` 是全量写），5–8 页是即时操作。
**弹层关闭判据**：按下与松开**都在遮罩上**才关（面板内拖选到外面松手不误关）；Esc 只关最上面一层（`ui.bindDismiss` 的弹层栈）。
**行内动作**（回档 / 恢复 / 撤回）是图标按钮，hover 或键盘聚焦浮现、触屏常显，说明走原生 title（长说明不会被滚动容器裁掉）；
一次性结果（已停止 / 已回档 / 已删除 / 已上传 / 设置已保存 / 已启用 / 已改需求）用 toast 反馈。
**围栏横幅**：`/api/state` 的 `fence` 给 `effective_fs/net`、`write_allowed`、`sandbox_missing`、`sandbox_degraded`；
前端只在 `sandbox_degraded` 或 `sandbox_missing` 非空时显示（降级黄色 / 要求未满足红色），正常与用户主动选的
`best_effort` 不显示。保存策略后整体重启并重算，横幅随新状态显示或消失；无 TTY 且要求未满足时启动门退出，
恢复走控制台或 `SOLOMNI_FENCE_WRITE=1`。红色「要求未满足」是 **bug 指示器**：保存/加载会拒绝
`require + write_acl=false`，正常运行的 Web 不会有这一态。

**不管**：不做业务判断、不持状态、不落盘；**永不接触端口对象，也拿不到 `Core` 本身**。

## 二、入站契约与状态归属

`src/presentation/web/routes.rs` 的 `ROUTES` 是 HTTP 入站契约的**唯一定义**，与 `docs/presentation/contracts.md` 的路由表机器比对（`src/tests/routes.rs`）。

入口：`src/main.rs` 调 `presentation::web::serve(ops, port, fence)`；默认端口是 `presentation::web::DEFAULT_PORT`（`webui` 与启动参数共用这一个来源）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`conductor`（`Ops` 字段里带 `provision` 面）
- 谁在用我：（无）

## 四、改动本单元时必须同步

- 增删路由 → 五处同改：`src/presentation/web/routes.rs` 的 `ROUTES`、前端调用点、`docs/presentation/contracts.md` 的 ROUTES 段、`src/tests/routes.rs`（机器比对）、`demo/*.mjs` 的调用点；前端资源改动跑冒烟。
- 测试缺口：本单元**不是业务能力**，没有独立的缺口账；缺口记在 `tests/gaps.yaml`；格式见 [docs/testing/gaps-acceptance.md](../../../docs/testing/gaps-acceptance.md) §十二。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
