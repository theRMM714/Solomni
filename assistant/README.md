# assistant

去中心化为常态的跨平台 AI 助手。

- 理念（抽象概念与不变量）：[PHILOSOPHY.md](PHILOSOPHY.md)
- **模块开发指南（给 AI 开发者，多开会话直接分发此文）：[MODULE_DEV_GUIDE.md](MODULE_DEV_GUIDE.md)**
- 产品与用户旅程（产品形态、启动/退出语义、分层职责）：[PRODUCT.md](PRODUCT.md)

## 结构

```
packages/
  protocol/            字典：封套、声明、能力词汇、核心元能力（无行为）
  transport/           线路层：JSON 行编解码 + 模块侧连接与配对推送（模块只依赖它，不依赖核心）
  ui_vocab/            UI 词汇表：组件类型/scope/贡献 schema（意图，无像素）
  ui_canvas/           画布模型：布局数据结构与放置规则（UI 模块共享词汇）
  core/                交易所：校验、配对、投递、TCP 守护、注册表列举
apps/
  modules/             模块区：一个文件夹 = 一个可装卸的自治程序
    <模块>/
      lib/             模块逻辑（内建端口实现 + 共享适配 + UI 贡献）
      bin/standalone.dart    单机入口：无核心，全内建
      bin/coordinated.dart   协作入口：守护进程，连核心（--port=9100）
  solomni/             产品：装配者 + 终端 REPL（发现/registry/-gui 拉起；规范见 PRODUCT.md）
```

## 装卸语义

- 安装 = 放一个文件夹进 `apps/modules/`（进程内组合需同时加进产品 registry）
- 卸载 = 移除文件夹，或产品装配时排除（`--without=<id>`，演示见下）
- 目录扫描只回答"有哪些模块文件夹"；能力声明经 hello 协议到达，核心不读文件
- **运行时是动态的**：声明到达即重配对，离线即降级（消费方自动回内建或功能降级），
  回归即恢复；配对变化推送给受影响消费方，接入顺序无关紧要

## UI 约定

- UI 也是普通模块（ui_cli 是 CLI 渲染器；Flutter UI 模块同样消费贡献）
- 有 UI 的模块提供 ui.contribution（意图清单：组件 id/kind/scope/bind）+ ui.event（事件路由回声明模块）
- 发现：ui 模块 rpc core.modules（注册表只读列举，事实非决策）-> 定向 call 各模块拉贡献；
  成员变化经配对推送到达，调色板/画布可实时刷新
- 模块 id 全局唯一（重复注册被校验拒收）-> 定向调用无歧义，聚合问题不存在
- 组件 scope：public 任意画布可用 / private 仅本模块画布；布局数据归 UI 模块私有
- 流式：处理器返回 Streamed(chunks)，消费方 rpcStream 逐块接收；ok 收全量，非流式消费方无感知

## 验证

```bash
cd packages/core && dart tool/smoke_test.dart        # 配对/路由规则（集合/候选/降级/校验拒收）
cd packages/core && dart test                        # 同一套用例（test 包版）
cd packages/core && dart tool/dynamic_test.dart     # 动态配对端到端（真 TCP：顺序无关/离线降级/回归恢复/推送）
cd packages/ui_canvas && dart tool/canvas_test.dart  # 画布模型规则（scope/持久化/增删改）
cd apps/modules/<任一> && dart bin/standalone.dart   # 单机模式
# 产品验收（自举假 LLM 服务器：无密钥拒绝/配钥后聊天/流式/多轮历史）：
cd apps/solomni && dart tool/smoke.dart
cd apps/solomni && dart tool/smoke.dart --without=llm_gateway  # 卸载 -> 回退内建（降级链路）
cd apps/solomni && dart bin/main.dart                          # 产品入口：终端 REPL（help 查看命令，exit 退出）
cd apps/modules/ui_flutter && flutter test           # Flutter UI 模块 mock 全链路（发现/布局/楼层/事件路由/配对推送）
```

## 环境备注

- 工具链/缓存按平台后缀目录放仓库根（dart-sdk-linux/、flutter-windows/、.pub-cache-\<平台\>/…），
  一键安装与目录说明见仓库根 [SETUP.md](../SETUP.md)
- 开发一律用仓库内包装命令 bin/dart / bin/flutter（自动配好 PUB_CACHE/TMP/HOME，跨平台一致）
