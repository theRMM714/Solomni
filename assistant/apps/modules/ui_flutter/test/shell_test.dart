/// ui_flutter 模块测试：mock Outbound（无核心）驱动发现/画布创建/选择器/事件路由全链路。
/// LayoutStore 注入内存实现，不落盘、不依赖网络与核心。
/// 覆盖 DESIGN.md：空白起步、模块画布创建、"+"规则过滤、离线冻结、命令执行、导入导出。
library;

import 'dart:async';
import 'dart:convert';
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

/// mock 出边：rpc 应答 core.modules 与 ui.command；
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
    if (method == 'ui.command') {
      calls.add(('core', method, params));
      return 'ok';
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

/// 建一张自定义画布并经"+"选择器放置组件（模拟用户操作路径）
Future<void> _addViaPicker(
    WidgetTester tester, String canvasTitle, String entryTitle) async {
  await tester.tap(find.text('新建画布'));
  await tester.pumpAndSettle();
  await tester.enterText(find.byType(TextField), canvasTitle);
  await tester.tap(find.text('创建'));
  await tester.pumpAndSettle();
  await tester.tap(find.text('添加组件'));
  await tester.pumpAndSettle();
  await tester.tap(find.text(entryTitle));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('standalone：无核心 -> 无模块提示 + 徽标', (tester) async {
    await tester.pumpWidget(UiShell(store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('无模块声明 UI'), findsOneWidget);
    expect(find.text('未连接核心（standalone）'), findsOneWidget);
  });

  testWidgets('发现：左面板列模块与命令，不列组件', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('conversation'), findsOneWidget);
    expect(find.text('secrets'), findsOneWidget);
    expect(find.text('send'), findsOneWidget);
    expect(find.text('set-key'), findsOneWidget);
    // 组件不再是全局列表（入口在画布"+"）
    expect(find.text('chat_floor'), findsNothing);
    expect(find.text('key_input'), findsNothing);
  });

  testWidgets('空白起步：无保存布局 -> 无画布，但创建入口可达', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('无画布：从左侧模块列表创建模块画布，或新建自定义画布'),
        findsOneWidget);
    expect(find.text('新建画布'), findsOneWidget);
  });

  testWidgets('模块画布创建：每模块一张，创建后入口消失', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    // secrets 有私有控件 -> 有创建入口；conversation 全公共 -> 没有
    expect(find.text('创建画布'), findsOneWidget);
    await tester.tap(find.text('创建画布'));
    await tester.pumpAndSettle();
    expect(find.text('secrets'), findsWidgets); // 模块名 + 画布 tab
    // 每模块一张：入口不再出现
    expect(find.text('创建画布'), findsNothing);
  });

  testWidgets('“+”选择器：自定义画布只列公共组件', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    await tester.tap(find.text('新建画布'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '我的画布');
    await tester.tap(find.text('创建'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('添加组件'));
    await tester.pumpAndSettle();
    // 公共组件可选
    expect(find.text('conversation.chat_floor'), findsOneWidget);
    expect(find.text('conversation.chat_input'), findsOneWidget);
    // 私有组件不出现（边界由构造保证）
    expect(find.text('secrets.key_input'), findsNothing);
    expect(find.text('secrets.key_list'), findsNothing);
    await tester.tap(find.text('conversation.chat_floor'));
    await tester.pumpAndSettle();
    // 放置后标题与楼层渲染
    expect(find.text('conversation.chat_floor'), findsOneWidget);
    expect(find.text('你好'), findsOneWidget);
    expect(find.text('在的'), findsOneWidget);
  });

  testWidgets('“+”选择器：模块画布含本模块私有组件', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    await tester.tap(find.text('创建画布')); // secrets 模块画布
    await tester.pumpAndSettle();
    await tester.tap(find.text('添加组件'));
    await tester.pumpAndSettle();
    expect(find.text('secrets.key_input'), findsOneWidget);
    expect(find.text('secrets.key_list'), findsOneWidget);
    expect(find.text('conversation.chat_floor'), findsOneWidget); // 公共仍可选
    await tester.tap(find.text('secrets.key_input'));
    await tester.pumpAndSettle();
    expect(find.text('secrets.key_input'), findsOneWidget); // 已放置
  });

  testWidgets('输入 submit -> ui.event 路由回声明模块并清空输入', (tester) async {
    // 桌面布局较宽：给足画布尺寸，输入卡片的发送按钮可点
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final mock = _demoOutbound();
    await tester.pumpWidget(UiShell(outbound: mock, store: _MemStore()));
    await tester.pumpAndSettle();
    await _addViaPicker(tester, '对话画布', 'conversation.chat_input');
    await tester.enterText(find.byType(TextField), '早上好');
    await tester.tap(find.byIcon(Icons.send));
    await tester.pumpAndSettle();
    final events = mock.calls.where((c) => c.$2 == UiCaps.event).toList();
    expect(events, hasLength(1));
    expect(events.single.$1, 'conversation');
    expect(events.single.$3, {
      'component': 'chat_input',
      'event': 'submit',
      'payload': {'text': '早上好'},
    });
    expect(find.text('早上好'), findsNothing);
  });

  testWidgets('命令可点击执行：弹参数 -> ui.command 路由', (tester) async {
    final mock = _demoOutbound();
    await tester.pumpWidget(UiShell(outbound: mock, store: _MemStore()));
    await tester.pumpAndSettle();
    await tester.tap(find.text('send')); // 左面板命令
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '你好世界');
    await tester.tap(find.text('确定'));
    await tester.pumpAndSettle();
    final cmds =
        mock.calls.where((c) => c.$2 == 'ui.command').toList();
    expect(cmds, hasLength(1));
    expect(cmds.single.$3, {
      'name': 'send',
      'args': ['你好世界'],
    });
    expect(find.text('send -> ok'), findsOneWidget); // 结果回显
  });

  testWidgets('离线冻结：模块下线组件保留禁用，重连恢复', (tester) async {
    final mock = _demoOutbound();
    await tester.pumpWidget(UiShell(outbound: mock, store: _MemStore()));
    await tester.pumpAndSettle();
    // 建 secrets 画布并放一个私有组件
    await tester.tap(find.text('创建画布'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('添加组件'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('secrets.key_input'));
    await tester.pumpAndSettle();
    expect(find.text('secrets.key_input'), findsOneWidget);

    // 模块下线（贡献仍在缓存里 -> 冻结禁用）
    mock.moduleIds.remove('secrets');
    mock.wiringController.add(const {});
    await tester.pumpAndSettle();
    expect(find.text('secrets.key_input · 离线'), findsOneWidget);
    expect(
        find.byWidgetPredicate((w) => w is AbsorbPointer && w.absorbing),
        findsOneWidget); // 交互禁用
    expect(find.text('secrets.key_input'), findsNothing); // 不再是在线态

    // 重连：自动恢复（配对推送触发）
    mock.moduleIds.add('secrets');
    mock.wiringController.add(const {});
    await tester.pumpAndSettle();
    expect(find.text('secrets.key_input · 离线'), findsNothing);
    expect(find.text('secrets.key_input'), findsOneWidget);
  });

  testWidgets('配对推送：成员变化 -> 模块列表实时刷新（勿轮询）', (tester) async {
    final mock = _demoOutbound();
    await tester.pumpWidget(UiShell(outbound: mock, store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('notes'), findsNothing);
    mock.moduleIds.add('notes');
    mock.responses['notes.ui.contribution'] = const UiContribution(
      components: [
        UiComponent('note_pad', UiKind.textOutput, label: '便签', bind: 'notes.read'),
      ],
    ).toJson();
    mock.wiringController.add(const {});
    await tester.pumpAndSettle();
    expect(find.text('notes'), findsOneWidget);
  });

  test('导入导出：JSON 往返 + 画布 id 重生成', () {
    final engine = LayoutEngine();
    final doc = LayoutDoc();
    final placeholder = engine.newCanvas(doc, '占位'); // 抬高源序号
    engine.newCanvas(doc, 'a');
    final owned = engine.newCanvas(doc, 'b', ownerModuleId: 'secrets');
    owned.placements.add(Placement('secrets', 'key_input'));
    doc.canvases.remove(placeholder); // 导出前移除占位
    final json = jsonEncode(doc.toJson());

    final importEngine = LayoutEngine();
    final restored = importEngine.importFrom(
        Map<String, Object?>.from(jsonDecode(json) as Map));
    expect(restored.canvases, hasLength(2));
    expect(restored.canvases[0].title, 'a');
    expect(restored.canvases[1].title, 'b');
    expect(restored.canvases[1].ownerModuleId, 'secrets');
    expect(restored.canvases[1].placements.single.componentId, 'key_input');
    expect(restored.canvases[1].placements.single.moduleId, 'secrets');
    // 画布 id 与源不同（重生成），且导入后再新建不撞 id
    expect(restored.canvases[0].id, isNot(doc.canvases[0].id));
    expect(restored.canvases[1].id, isNot(doc.canvases[1].id));
    importEngine.newCanvas(restored, 'c');
    expect(restored.canvases.map((c) => c.id).toSet().length,
        restored.canvases.length);
  });
}
