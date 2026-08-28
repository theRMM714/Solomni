# 模块开发指南（面向 AI 开发者）

> 本文档写给被指派开发模块的 AI agent：读完即可动手，不需要了解项目历史。
> 理念背景见 [PHILOSOPHY.md](PHILOSOPHY.md)（先读"终极目标"和"不变量"两节）。
> 本文只讲规范与边界。**违反边界条款 = 返工**。

## 一分钟理解项目

- 这是一个去中心化为常态的 AI 助手：每个功能是一个**自治程序（模块）**，通过一个内容无关的**核心（交易所）**协作
- 核心只做校验、配对、投递，不懂任何业务，没有上帝；换掉任何模块、换掉核心，互不牵连
- 模块没有核心也能跑（standalone）；有核心时值得共享的才被共享（coordinated）
- 配对随声明实时变化：接入顺序无关紧要；提供方离线你自动降级，回归自动恢复
- 你的任务：在 `apps/modules/` 下写一个模块文件夹。文件夹 = 安装卸载单元

## 模块结构（必须遵守）

```
apps/modules/<module_id>/
  pubspec.yaml            # name 必须等于文件夹名（= 模块 id，全局唯一）
  lib/<module_id>.dart    # 模块逻辑 + ModuleProgram
  bin/standalone.dart     # 单机入口：无核心、全内建，必须能独立运行
  bin/coordinated.dart    # 协作入口：守护进程，连核心（--port=9100）
```

依赖白名单（只能依赖这三个，且通常只需要前两个）：

```yaml
dependencies:
  protocol:
    path: ../../../packages/protocol    # 封套、声明、能力词汇（字典）
  transport:
    path: ../../../packages/transport    # Outbound / ModuleClient（线路层）
  ui_vocab:                               # 仅当模块有 UI
    path: ../../../packages/ui_vocab
```

## 声明：模块对世界的全部认知

```dart
final class XxxProgram implements ModuleProgram {
  @override
  Declaration get declaration => const Declaration(
        'xxx',                                   // 模块 id = 文件夹名，全局唯一
        provides: [Provide('xxx.hello')],        // 我愿意把这些能力当共同物
        needs: [                                 // 我需要什么，三选一策略：
          Need('llm.chat', NeedVia.preferShared), // 有共享用共享，没有回退内建
          // Need('llm.chat', NeedVia.preferShared, provider: 'llm_gateway'),
                                                 // 多提供方在线时声明你的首选（消费方策略选择）
          // Need('xxx.store', NeedVia.builtinOnly), // 模块私有，永不共享
          // Need('stt.transcribe', NeedVia.sharedOnly), // 造不出来，缺则功能降级
        ],
      );

  @override
  ModuleHandler bind(Outbound outbound) { /* 见模板 */ }
}
```

- 能力地址命名：**私有能力用自己的 id 前缀**（`xxx.hello`）；跨模块公共能力（如 `llm.chat`）已有词汇表 `packages/protocol/lib/protocol.dart` 的 `Caps`，**新增公共词汇需要走治理**（见文末），不要私自发明同义能力名
- `bind(outbound)` 返回消息处理器：收到封套，返回**纯 JSON 数据**

## 边界（红线，逐条可判）

1. **不 import core**。核心是可替换的交易所；模块只认识 protocol/transport/ui_vocab
2. **边界上只有消息**。跨模块传的参数与返回值必须是 Map/List/String/num/bool/null，永不共享对象引用
3. **状态私有**。你的数据（会话、配置、缓存）存你模块内部；没有共享数据库；状态所有权排他 = 模块间无冲突的根源
4. **组件 id 是契约**。UI 贡献里的组件 id 对 UI 是只读身份；你改 id = 破坏契约
5. **内建有意的薄**。prefer-shared 需求的内建兜底做到"单机够用、测试够用"即止，富实现只放共享侧一处
6. **事件路由回声明模块**。UI 只发 `ui.event {component, event, payload}` 给你，组件语义由你的模块自己解释，业务永远不出模块边界
7. **fail-fast 与降级的边界**。你不认识的 method/组件事件直接抛 UnsupportedError；
   非法声明（重复 id、成环）由核心校验拒收；缺提供方不由任何一方失败--
   调用时抛 NoProviderException，你按声明策略降级（preferShared 回内建、
   sharedOnly 功能降级），不静默吞错
8. **协作单路径**。模块间不直连、不点对点；一切经核心（rpc 按配对路由 / call 按模块 id 定向）

## 出边 API（你发消息的方式）

```dart
outbound.rpc('llm.chat', {'messages': [...]})       // 业务调用：按当前配对路由
   //   0 提供方 -> NoProviderException（preferShared 的内建兜底 = catch 它走本地实现）
   //   1 提供方 -> 直达
   //   N 提供方 -> 显式接线 > 你声明的首选（Need.provider）；两者皆无 ->
   //               CandidatesException（附候选清单）：你 catch 后用 call() 定向选择，
   //              或在声明里加 provider 后重连
outbound.rpcStream('llm.chat', {'messages': [...]}) // 流式：同 rpc 路由；token 经 event 封逐块到达
outbound.call('conversation', 'ui.event', {...})    // 定向调用：按模块 id 直达（发现/事件路由/多提供方选择）
outbound.rpc('core.modules', null)                   // 元能力：注册表只读列举（事实，非决策）
outbound.wiring                                     // Stream：本模块各需求 -> 当前提供方集合的快照
                                                     // 成员变化时核心推送新快照（勿轮询；UI 据此实时刷新）
```

- 配对随声明到达/离开实时重算：提供方离线你自动降级，回归自动恢复，接入顺序无关
- `call` 靠模块 id 全局唯一保证无歧义

## 流式（提供 token 类输出的模块）

- 处理器返回 `Streamed(chunks)`（protocol 提供）而非 String：线路层把每块作为同 id 的 event 封逐个转发，最后 ok 收全量
- 消费方用 `rpcStream` 逐块接收；普通消费方（`rpc`）对流式提供方无感知：ok 里是拼好的全量文本

## 最小模块模板（复制即用）

```dart
// lib/xxx.dart
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

final class XxxProgram implements ModuleProgram {
  @override
  Declaration get declaration => const Declaration(
        'xxx',
        provides: [Provide('xxx.hello')],
        needs: [],
      );

  @override
  ModuleHandler bind(Outbound outbound) {
    return (env) async {
      if (env.method == 'xxx.hello') return {'ok': true};
      throw UnsupportedError('xxx 不认识 ' + env.method);
    };
  }
}
```

```dart
// bin/coordinated.dart
import 'dart:io';
import 'package:transport/transport.dart';
import 'package:xxx/xxx.dart';

Future<void> main(List<String> args) async {
  var port = 9100;
  for (final a in args) {
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  await ModuleClient.connect(InternetAddress.loopbackIPv4, port, XxxProgram());
  print('[xxx] 已连接核心 :' + port.toString());
}
```

## UI 贡献（可选，若模块有界面）

```dart
static const contribution = UiContribution(
  components: [
    UiComponent('xxx_panel', UiKind.textOutput, label: '面板', bind: 'xxx.data'),
    UiComponent('xxx_input', UiKind.textInput, scope: UiScope.private, label: '输入'),
  ],
  commands: [
    UiCommand('do', args: ['text'], component: 'xxx_input', event: 'submit'),
  ],
);
// 声明里加 Provide(UiCaps.contribution) + Provide(UiCaps.event)
// handler 里响应：
//   UiCaps.contribution -> return contribution.toJson();
//   UiCaps.event        -> 解释 component/event，执行业务
```

原则：贡献里**只有意图**（组件 id/类型/绑定/事件/命令），**没有像素**（坐标/颜色/布局不归你管）；布局归 UI 模块私有。scope：public 任意画布可用，private 仅你的画布。UI 词汇表里没有的组件类型用 `xxx.my_kind` 命名空间自定义，UI 会降级渲染。

## 测试要求（缺一不可）

1. **standalone 必须能跑**：`dart bin/standalone.dart` 无核心、无网络依赖（或明确跳过网络部分）
2. **协作链路验证**：在 `apps/desktop_demo/bin/main.dart` 的 registry 加一行工厂 + pubspec 加一个 path 依赖，跑通全链路（这是装配者的地盘，只做最小编辑，别重构别人代码）
3. 消费外部服务时，本地起假服务器验证（参考 `apps/modules/llm_gateway/bin/fake_server.dart`），不依赖真实密钥
4. 协作语义用 mock Outbound 测试：配对快照（wiring）、候选错误（CandidatesException）、
   降级（NoProviderException）都可注入模拟，不必起核心

## 环境备注（跨平台，clone 即用）

- Dart / Flutter SDK 与缓存统一放仓库根的平台后缀目录（dart-sdk-linux/、flutter-windows/…，
  .gitignore 排除）；用仓库内 `bin/setup.mjs` 一键安装：
  - `node bin/setup.mjs` 装 Dart SDK 并 `pub get`；加 `--flutter` 再装 Flutter SDK
- 开发请用仓库内包装命令 `bin/dart` / `bin/flutter`（自动配置缓存），无需手动改环境变量或路径
- 详见仓库根 `SETUP.md`

- 沙箱限制：HTTPS 下载走 node fetch；子进程管道被禁（不能 spawn 捕获输出的进程）；多进程守护无法在本沙箱演示，但单进程 TCP 拓扑可全链路验证
- 模块被 `scanModules` 发现的条件：文件夹内有 `bin/coordinated.dart`

## 治理：不要碰的东西

| 位置 | 归属 | 你能做 |
|---|---|---|
| `packages/protocol` | 全局字典（封套/声明/Caps） | 只读；新增公共能力名需协调，不改已有 |
| `packages/transport` | 线路层 | 只读 |
| `packages/core` | 核心交易所 | 只读 |
| `packages/ui_vocab` | UI 词汇表 | 只读；新增公共组件类型需协调 |
| `packages/ui_canvas` | 画布模型（布局结构/放置规则） | 只读；布局规则变更需协调 |
| `apps/modules/<别人的模块>` | 该模块的 agent | 只读 |
| `apps/desktop_demo` | 产品装配 | 仅最小编辑：registry 一行 + pubspec 一行 |

**多 agent 并行开发约定**：一人一个模块文件夹；共享包改动（新公共能力名、新组件类型）是对齐事项而不是顺手事项--先在会话里提出，达成一致再动字典。字典可以长，上帝不能长。

## 完整参考实现

- 带多轮业务 + UI 贡献 + 事件路由：`apps/modules/conversation/`
- 带外部 HTTP 依赖 + 密钥需求 + 假服务器验证：`apps/modules/llm_gateway/`
- UI 渲染器（发现/拉取/画布/命令路由）：`apps/modules/ui_cli/`
- Flutter 渲染器（拖放画布/布局持久化/事件路由/mock 测试）：`apps/modules/ui_flutter/`
