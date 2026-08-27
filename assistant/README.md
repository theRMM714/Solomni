# assistant

去中心化为常态的跨平台 AI 助手。

- 理念（抽象概念与不变量）：[PHILOSOPHY.md](PHILOSOPHY.md)
- **模块开发指南（给 AI 开发者，多开会话直接分发此文）：[MODULE_DEV_GUIDE.md](MODULE_DEV_GUIDE.md)**

## 结构

```
packages/
  protocol/            字典：封套、声明、能力词汇、核心元能力（无行为）
  transport/           线路层：JSON 行编解码 + 模块侧连接（模块只依赖它，不依赖核心）
  ui_vocab/            UI 词汇表：组件类型/scope/贡献 schema（意图，无像素）
  core/                交易所：撮合、路由、TCP 守护、目录发现、注册表列举
apps/
  modules/             模块区：一个文件夹 = 一个可装卸的自治程序
    <模块>/
      lib/             模块逻辑（内建端口实现 + 共享适配 + UI 贡献）
      bin/standalone.dart    单机入口：无核心，全内建
      bin/coordinated.dart   协作入口：守护进程，连核心（--port=9100）
  desktop_demo/        桌面产品：扫描 apps/modules -> TCP 拓扑 -> 撮合 -> 全链路
```

## 装卸语义

- 安装 = 放一个文件夹进 `apps/modules/`（进程内组合需同时加进产品 registry）
- 卸载 = 移除文件夹，或产品装配时排除（`--without=<id>`，演示见下）
- 目录扫描只回答"有哪些模块文件夹"；能力声明经 hello 协议到达，核心不读文件

## UI 约定（本轮新增）

- UI 也是普通模块（ui_cli 是 CLI 渲染器；Flutter UI 模块将来同样消费贡献）
- 有 UI 的模块提供 ui.contribution（意图清单：组件 id/kind/scope/bind）+ ui.event（事件路由回声明模块）
- 发现：ui 模块 rpc core.modules（注册表只读列举，事实非决策）-> 定向 call 各模块拉贡献
- 模块 id 全局唯一（重复注册 fail-fast）-> 定向调用无歧义，聚合问题不存在
- 组件 scope：public 任意画布可用 / private 仅本模块画布；布局数据归 UI 模块私有
- 流式：处理器返回 Streamed(chunks)，消费方 rpcStream 逐块接收；ok 收全量，非流式消费方无感知

## 验证

```bash
cd packages/core && dart tool/smoke_test.dart        # 撮合规则 8/8（含重复 id）
cd apps/modules/<任一> && dart bin/standalone.dart   # 单机模式
cd apps/desktop_demo && dart bin/main.dart           # 协作 + UI 调色板/画布/命令全链路
cd apps/desktop_demo && dart bin/main.dart --without=llm_gateway  # 卸载 -> 回退内建
```

## 环境备注（本机沙箱）

- Dart SDK 解压于 `../dart-sdk/dart-sdk`（沙箱无全局 dart，HTTPS 需经 node 下载）
- 运行前重定向缓存：APPDATA/LOCALAPPDATA/PUB_CACHE/TMP 指向工作区目录
- 沙箱禁子进程管道：多进程守护（三个终端各跑 coordinated + 核心）在本沙箱无法演示，
  但 TCP 传输路径已在单进程内全链路验证；用户机上直接可用
