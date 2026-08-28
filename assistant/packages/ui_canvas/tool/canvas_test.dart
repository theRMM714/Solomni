/// 画布模型逻辑测试：规则与持久化（不依赖 Flutter，直接 VM 跑）。
library;

import 'package:ui_canvas/ui_canvas.dart';
import 'package:ui_vocab/ui_vocab.dart';

var _passed = 0;
void check(String name, bool ok) {
  print((ok ? '  ok ' : 'FAIL ') + name);
  if (ok) _passed++;
}

void main() {
  final eng = LayoutEngine();
  final found = [
    Contributed('conversation', const UiContribution(components: [
      UiComponent('chat_floor', UiKind.textStream, label: '对话楼层'),
      UiComponent('chat_input', UiKind.textInput, label: '输入'),
    ])),
    Contributed('secrets', const UiContribution(components: [
      UiComponent('key_input', UiKind.formField, scope: UiScope.private, label: 'API Key'),
      UiComponent('key_list', UiKind.list, scope: UiScope.private, label: '已存密钥'),
    ])),
  ];

  // 1. 初始布局：public -> 默认画布；private -> 模块私有画布
  final doc = eng.initial(found);
  check('默认画布存在', doc.canvases.isNotEmpty && doc.canvases.first.ownerModuleId == null);
  check('public 组件都在默认画布',
      doc.canvases.first.placements.length == 2);
  final secretCanvas = doc.canvases.where((c) => c.ownerModuleId == 'secrets').toList();
  check('secrets 私有画布存在', secretCanvas.length == 1 && secretCanvas.first.placements.length == 2);

  // 2. 放置规则
  final def = doc.canvases.first;
  final keyInput = PaletteEntry('secrets',
      const UiComponent('key_input', UiKind.formField, scope: UiScope.private));
  check('private 组件不能放进公共画布', !eng.canPlace(def, keyInput));
  check('private 组件能放进所属模块画布', eng.canPlace(secretCanvas.first, keyInput));
  final chatInput = PaletteEntry('conversation', const UiComponent('chat_input', UiKind.textInput));
  check('public 组件任意画布可放', eng.canPlace(def, chatInput) && eng.canPlace(secretCanvas.first, chatInput));
  check('place 拒绝违规放置', !eng.place(def, keyInput));
  check('place 接受合规放置', eng.place(secretCanvas.first, keyInput, x: 300, y: 300));

  // 3. 移动与缩放
  final p = secretCanvas.first.placements.first;
  eng.move(p, 40, -10);
  check('move 生效', p.x > 40 && p.y >= 0);
  final w0 = p.w;
  eng.resize(p, -1000, 0);
  check('resize 下限保护', p.w >= 120 && w0 > 0);

  // 4. 持久化往返
  final rt = LayoutDoc.fromJson(
      Map<String, Object?>.from(doc.toJson() as Map));
  check('画布数量往返一致', rt.canvases.length == doc.canvases.length);
  check('放置数量往返一致',
      rt.canvases.first.placements.length == doc.canvases.first.placements.length);
  check('owner 往返一致', rt.canvases[1].ownerModuleId == 'secrets');

  // 5. 新建画布
  final nc = eng.newCanvas(doc, '我的画布');
  check('新建画布可放 public', eng.canPlace(nc, chatInput) && !eng.canPlace(nc, keyInput));

  print('');
  print('全部通过: ' + _passed.toString() + ' 项');
}
