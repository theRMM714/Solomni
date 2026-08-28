/// ui_flutter 模块测试：mock Outbound（无核心）驱动发现/渲染/事件路由全链路。
/// LayoutStore 注入内存实现，不落盘、不依赖网络与核心。
library;

import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:protocol/outbound.dart';
import 'package:protocol/protocol.dart';
import 'package:ui_canvas/ui_canvas.dart';
import 'package:ui_flutter/layout_store.dart';
import 'package:ui_flutter/widgets/shell.dart';
import 'package:ui_vocab/ui_vocab.dart';

/// 内存布局存储：测试替换默认文件存储（DIP 注入点）
class _MemStore extends LayoutStore {
  LayoutDoc? saved;

  @override
  Future<LayoutDoc?> load() async => saved;

  @override
  Future<void> save(LayoutDoc doc) async => saved = doc;
}

/// mock 出边：rpc 只应答 core.modules（注册表列举）；
/// 定向 call 按 "模块id.方法" 查 responses 应答并记录全部调用
class _MockOutbound implements Outbound {
  final List<String> moduleIds;
  final Map<String, Object?> responses;
  final calls = <(String, String, Object?)>[];

  _MockOutbound({required this.moduleIds, required this.responses});

  @override
  Future<Object?> rpc(String method, Object? params) async {
    if (method == CoreCaps.modules) {
      return {
        'modules': [
          for (final id in moduleIds)
            Declaration(id,
                provides: [Provide(UiCaps.contribution), Provide(UiCaps.event)]).toJson(),
        ],
      };
    }
    throw NoProviderException(method);
  }

  @override
  Stream<Object?> rpcStream(String method, Object? params) async* {}

  @override
  Future<Object?> call(String moduleId, String method, Object? params) async {
    calls.add((moduleId, method, params));
    return responses['$moduleId.$method'];
  }

  /// 配对快照流（可注入模拟核心推送；默认静默）
  final wiringController = StreamController<WiringSnapshot>.broadcast();
  @override
  Stream<WiringSnapshot> get wiring => wiringController.stream;
}

/// 模拟 conversation + secrets 两模块的贡献（与真实模块声明一致）
_MockOutbound _demoOutbound() => _MockOutbound(
      moduleIds: ['conversation', 'secrets'],
      responses: {
        'conversation.ui.contribution': const UiContribution(
          components: [
            UiComponent('chat_floor', UiKind.textStream,
                label: '对话楼层', bind: Caps.chatHistory),
            UiComponent('chat_input', UiKind.textInput, label: '输入', bind: Caps.chatSend),
          ],
          commands: [
            UiCommand('send', args: ['text'],
                description: '发送一条消息', component: 'chat_input', event: 'submit'),
          ],
        ).toJson(),
        'secrets.ui.contribution': const UiContribution(
          components: [
            UiComponent('key_input', UiKind.formField,
                scope: UiScope.private, label: 'API Key', bind: Caps.secretsPut),
            UiComponent('key_list', UiKind.list,
                scope: UiScope.private, label: '已存密钥', bind: Caps.secretsGet),
          ],
          commands: [
            UiCommand('set-key', args: ['name', 'value'],
                description: '写入密钥', component: 'key_input', event: 'submit'),
          ],
        ).toJson(),
        'conversation.${Caps.chatHistory}': [
          {'role': 'user', 'content': '你好'},
          {'role': 'assistant', 'content': '在的'},
        ],
      },
    );

void main() {
  testWidgets('standalone：无核心 -> 空调色板提示 + 徽标', (tester) async {
    await tester.pumpWidget(UiShell(store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('调色板为空（无模块声明 UI）'), findsOneWidget);
    expect(find.text('未连接核心（standalone）'), findsOneWidget);
  });

  testWidgets('发现：调色板列出各模块组件与命令', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('conversation'), findsOneWidget);
    expect(find.text('secrets'), findsOneWidget);
    expect(find.text('chat_floor'), findsOneWidget);
    expect(find.text('chat_input'), findsOneWidget);
    expect(find.text('key_input'), findsOneWidget);
    expect(find.text('key_list'), findsOneWidget);
    expect(find.text('send'), findsOneWidget);
    expect(find.text('set-key'), findsOneWidget);
  });

  testWidgets('初始布局：public 进默认画布，private 进模块私有画布', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('默认画布'), findsOneWidget);
    expect(find.text('secrets 私有画布'), findsOneWidget);
    // 默认画布上是 public 放置（标题 = 模块id.组件id）
    expect(find.text('conversation.chat_floor'), findsOneWidget);
    expect(find.text('conversation.chat_input'), findsOneWidget);
    // 切到 secrets 私有画布
    await tester.tap(find.text('secrets 私有画布'));
    await tester.pumpAndSettle();
    expect(find.text('secrets.key_input'), findsOneWidget);
    expect(find.text('secrets.key_list'), findsOneWidget);
  });

  testWidgets('楼层：按 bind 拉取历史并渲染消息气泡', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('你好'), findsOneWidget);
    expect(find.text('在的'), findsOneWidget);
  });

  testWidgets('输入 submit -> ui.event 路由回声明模块并清空输入', (tester) async {
    // 桌面布局较宽：给足画布尺寸，输入卡片的发送按钮可点
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final mock = _demoOutbound();
    await tester.pumpWidget(UiShell(outbound: mock, store: _MemStore()));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '早上好');
    await tester.tap(find.byIcon(Icons.send));
    await tester.pumpAndSettle();
    // 提交后楼层会再拉一次历史，因此按方法过滤而非取末条
    final events = mock.calls.where((c) => c.$2 == UiCaps.event).toList();
    expect(events, hasLength(1));
    expect(events.single.$1, 'conversation');
    expect(events.single.$3, {
      'component': 'chat_input',
      'event': 'submit',
      'payload': {'text': '早上好'},
    });
    // 成功提交后输入框清空
    expect(find.text('早上好'), findsNothing);
  });

  testWidgets('配对推送：成员变化 -> 调色板实时刷新（勿轮询）', (tester) async {
    final mock = _demoOutbound();
    await tester.pumpWidget(UiShell(outbound: mock, store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('便签'), findsNothing);

    // 模拟新模块上线：注册表加入成员 + 贡献应答就位
    mock.moduleIds.add('notes');
    mock.responses['notes.ui.contribution'] = const UiContribution(
      components: [
        UiComponent('note_pad', UiKind.textOutput, label: '便签', bind: 'notes.read'),
      ],
    ).toJson();
    // 核心推送配对快照（无需求的 UI 模块靠推送感知成员变化）
    mock.wiringController.add(const {});
    await tester.pumpAndSettle();

    expect(find.text('notes'), findsOneWidget);
    expect(find.text('note_pad'), findsOneWidget); // 调色板列组件 id（契约身份）
  });
}
