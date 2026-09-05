/// ui_flutter 模块测试：mock Outbound（无核心）驱动发现/画布创建/删除/选择器/事件路由全链路。
/// LayoutStore 注入内存实现，不落盘、不依赖网络与核心。
/// 覆盖 DESIGN.md：空白起步、新建画布弹窗（模块画布/自定义画布）、"+"规则过滤、
/// 画布删除、离线冻结、命令不出现在 GUI、导入导出。
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
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
  _MemStore() : super(file: File('mem_layout.json'));
  LayoutDoc? saved;

  @override
  Future<LayoutDoc?> load() async => saved;

  @override
  Future<void> save(LayoutDoc doc) async => saved = doc;
}

/// mock 出边：rpc 应答 core.modules；定向 call 按 "模块id.方法" 查 responses 应答并记录全部调用
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

/// 模拟 conversation + secrets 两模块的贡献（与真实模块声明一致）：
/// conversation 全公共组件；secrets 全私有组件（模块画布候选）
_MockOutbound _demoOutbound() => _MockOutbound(
      moduleIds: ['conversation', 'secrets'],
      responses: {
        'conversation.ui.contribution': const UiContribution(
          components: [
            UiComponent('chat_floor', UiKind.textStream,
                label: '对话楼层', bind: Caps.chatHistory),
            UiComponent('chat_input', UiKind.textInput, label: '输入', bind: Caps.chatSend),
          ],
        ).toJson(),
        'secrets.ui.contribution': const UiContribution(
          components: [
            UiComponent('key_input', UiKind.formField,
                scope: UiScope.private,
                label: '密钥',
                bind: Caps.secretsPut,
                fields: [
                  UiField('name', label: '名字'),
                  UiField('value', label: '钥匙', secret: true),
                  UiField('remark', label: '备注'),
                ]),
            UiComponent('key_list', UiKind.list,
                scope: UiScope.private, label: '已存密钥', bind: Caps.secretsList),
          ],
        ).toJson(),
        'conversation.${Caps.chatHistory}': [
          {'role': 'user', 'content': '你好'},
          {'role': 'assistant', 'content': '在的'},
        ],
        'secrets.${Caps.secretsList}': [
          {'name': 'llm', 'remark': 'DeepSeek 主力'},
        ],
      },
    );

/// 新建画布按钮定位：空状态用 CTA（FilledButton.icon 是子类型，须 bySubtype），
/// 有画布后用画布栏 chip（同一弹窗）
Finder _newCanvasButton(WidgetTester tester) {
  final cta = find.ancestor(
      of: find.text('新建画布'), matching: find.bySubtype<FilledButton>());
  return cta.evaluate().isNotEmpty
      ? cta
      : find.widgetWithText(ActionChip, '新建画布');
}

/// 经弹窗建一张自定义画布
Future<void> _newCustom(WidgetTester tester, String title) async {
  await tester.tap(_newCanvasButton(tester));
  await tester.pumpAndSettle();
  await tester.enterText(find.byType(TextField), title);
  await tester.tap(find.text('创建'));
  await tester.pumpAndSettle();
}

/// 经弹窗建一张模块画布（.last：弹窗覆盖层在树尾，避开同名画布 chip）
Future<void> _newModule(WidgetTester tester, String moduleId) async {
  await tester.tap(_newCanvasButton(tester));
  await tester.pumpAndSettle();
  await tester.tap(find.text(moduleId).last);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('standalone：无核心 -> 空状态 + 徽标 + 弹窗仅自定义', (tester) async {
    await tester.pumpWidget(UiShell(store: _MemStore()));
    await tester.pumpAndSettle();
    expect(find.text('还没有画布'), findsOneWidget);
    expect(find.text('未连接核心（standalone）'), findsOneWidget);
    await tester.tap(find.ancestor(of: find.text("新建画布"), matching: find.bySubtype<FilledButton>()));
    await tester.pumpAndSettle();
    expect(find.text('暂无含私有组件的模块'), findsOneWidget);
    expect(find.text('自定义画布'), findsOneWidget);
    expect(find.text('secrets'), findsNothing); // 无模块候选
    await tester.tap(find.text('取消'));
  });

  testWidgets('新建弹窗：模块画布只列含私有组件的模块；命令不出现在 GUI', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    await tester.tap(find.ancestor(of: find.text("新建画布"), matching: find.bySubtype<FilledButton>()));
    await tester.pumpAndSettle();
    expect(find.text('secrets'), findsOneWidget); // 含私有组件 -> 候选
    expect(find.text('conversation'), findsNothing); // 全公共 -> 不列
    // 命令是文本交互面（REPL）的词汇，GUI 不渲染（设计回归守卫）
    expect(find.text('set-key'), findsNothing);
    expect(find.text('send'), findsNothing);
    await tester.tap(find.text('取消'));
  });

  testWidgets('模块画布创建：空白起步 + 每模块一张（已创建禁用）', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    await _newModule(tester, 'secrets');
    // 画布栏出现模块画布 chip；模块列表已不存在（左栏已删）
    expect(find.text('secrets'), findsOneWidget);
    // 空白起步：组件不自动预置，经"+"进入
    expect(find.text('secrets.key_input'), findsNothing);
    // 每模块一张：重开弹窗，候选禁用
    await tester.tap(find.widgetWithText(ActionChip, '新建画布'));
    await tester.pumpAndSettle();
    expect(find.text('已创建'), findsOneWidget);
    await tester.tap(find.text('取消'));
  });

  testWidgets('“+”选择器：自定义画布只列公共组件', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    await _newCustom(tester, '我的画布');
    expect(find.text('我的画布'), findsOneWidget);
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
    await _newModule(tester, 'secrets');
    await tester.tap(find.text('添加组件'));
    await tester.pumpAndSettle();
    expect(find.text('secrets.key_input'), findsOneWidget);
    expect(find.text('secrets.key_list'), findsOneWidget);
    expect(find.text('conversation.chat_floor'), findsNothing); // 模块画布不收公共组件
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
    await _newCustom(tester, '对话画布');
    await tester.tap(find.text('添加组件'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('conversation.chat_input'));
    await tester.pumpAndSettle();
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

  testWidgets('离线冻结：模块下线组件保留禁用，重连恢复', (tester) async {
    final mock = _demoOutbound();
    await tester.pumpWidget(UiShell(outbound: mock, store: _MemStore()));
    await tester.pumpAndSettle();
    // 建 secrets 画布并放一个私有组件
    await _newModule(tester, 'secrets');
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

  testWidgets('配对推送：含私有组件的新模块上线 -> 弹窗候选实时刷新（勿轮询）', (tester) async {
    final mock = _demoOutbound();
    await tester.pumpWidget(UiShell(outbound: mock, store: _MemStore()));
    await tester.pumpAndSettle();
    // notes 上线但只有公共组件 -> 不进模块画布候选
    mock.moduleIds.add('notes');
    mock.responses['notes.ui.contribution'] = const UiContribution(
      components: [
        UiComponent('note_pad', UiKind.textOutput, label: '便签'),
      ],
    ).toJson();
    mock.wiringController.add(const {});
    await tester.pumpAndSettle();
    await tester.tap(find.ancestor(of: find.text("新建画布"), matching: find.bySubtype<FilledButton>()));
    await tester.pumpAndSettle();
    expect(find.text('notes'), findsNothing);
    await tester.tap(find.text('取消'));
    // notes 补充私有组件 -> 候选出现
    mock.responses['notes.ui.contribution'] = const UiContribution(
      components: [
        UiComponent('note_pad', UiKind.textOutput, label: '便签'),
        UiComponent('secret_note', UiKind.formField,
            scope: UiScope.private, label: '私密便签'),
      ],
    ).toJson();
    mock.wiringController.add(const {});
    await tester.pumpAndSettle();
    await tester.tap(find.ancestor(of: find.text("新建画布"), matching: find.bySubtype<FilledButton>()));
    await tester.pumpAndSettle();
    expect(find.text('notes'), findsOneWidget);
    await tester.tap(find.text('取消'));
  });

  testWidgets('画布删除：确认后移除；删模块画布后弹窗重新可选', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    await _newCustom(tester, 'A');
    await _newModule(tester, 'secrets');
    // 两张画布各带删除图标
    expect(find.byIcon(Icons.close), findsNWidgets(2));
    // 删除当前（secrets）
    await tester.tap(find.byIcon(Icons.close).at(1));
    await tester.pumpAndSettle();
    expect(find.text('删除画布「secrets」？'), findsOneWidget);
    await tester.tap(find.text('删除'));
    await tester.pumpAndSettle();
    expect(find.text('secrets'), findsNothing); // 画布已删
    expect(find.text('A'), findsOneWidget); // 其余保留
    // 模块画布可重建：弹窗候选恢复可选
    await tester.tap(find.widgetWithText(ActionChip, '新建画布'));
    await tester.pumpAndSettle();
    expect(find.text('已创建'), findsNothing);
    await tester.tap(find.text('secrets').last);
    await tester.pumpAndSettle();
    expect(find.text('secrets'), findsOneWidget);
  });

  testWidgets('画布切换：点击画布栏 chip 切换当前画布', (tester) async {
    await tester.pumpWidget(UiShell(outbound: _demoOutbound(), store: _MemStore()));
    await tester.pumpAndSettle();
    await _newCustom(tester, '甲');
    await _newCustom(tester, '乙');
    // 新建的乙处于选中态：其“添加组件”作用于乙
    await tester.tap(find.text('添加组件'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('conversation.chat_floor'));
    await tester.pumpAndSettle();
    expect(find.text('conversation.chat_floor'), findsOneWidget);
    // 切回甲：无放置
    await tester.tap(find.text('甲'));
    await tester.pumpAndSettle();
    expect(find.text('conversation.chat_floor'), findsNothing);
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
