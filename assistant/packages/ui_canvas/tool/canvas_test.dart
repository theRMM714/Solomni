/// 画布模型逻辑测试：规则与持久化（不依赖 Flutter，直接 VM 跑）。
/// 覆盖 DESIGN.md：空白起步、模块/自定义画布、放置规则、导入导出。
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

  final keyInput = PaletteEntry('secrets',
      const UiComponent('key_input', UiKind.formField, scope: UiScope.private));
  final chatInput = PaletteEntry(
      'conversation', const UiComponent('chat_input', UiKind.textInput));

  // 1. 空白起步：新文档零画布，画布由用户创建
  final doc = LayoutDoc();
  check('空白起步：零画布', doc.canvases.isEmpty);

  // 2. 画布创建：自定义画布（owner=null）与模块画布（owner=模块 id）
  final custom = eng.newCanvas(doc, '我的画布');
  final owned = eng.newCanvas(doc, 'secrets', ownerModuleId: 'secrets');
  check('自定义画布 owner 为空', custom.ownerModuleId == null);
  check('模块画布 owner 为模块 id', owned.ownerModuleId == 'secrets');

  // 3. 放置规则：private 仅所属模块画布；public 任意画布
  check('private 组件不能放进自定义画布', !eng.canPlace(custom, keyInput));
  check('private 组件能放进所属模块画布', eng.canPlace(owned, keyInput));
  check('public 组件任意画布可放',
      eng.canPlace(custom, chatInput) && eng.canPlace(owned, chatInput));
  check('place 拒绝违规放置', !eng.place(custom, keyInput));
  check('place 接受合规放置', eng.place(owned, keyInput, x: 300, y: 300));
  check('place 接受公共放置', eng.place(custom, chatInput));

  // 4. 移动与缩放
  final p = owned.placements.first;
  eng.move(p, 40, -10);
  check('move 生效', p.x > 40 && p.y >= 0);
  final w0 = p.w;
  eng.resize(p, -1000, 0);
  check('resize 下限保护', p.w >= 120 && w0 > 0);

  // 5. 持久化往返
  final rt = LayoutDoc.fromJson(
      Map<String, Object?>.from(doc.toJson() as Map));
  check('画布数量往返一致', rt.canvases.length == doc.canvases.length);
  check('放置数量往返一致',
      rt.canvases[0].placements.length == doc.canvases[0].placements.length);
  check('owner 往返一致', rt.canvases[1].ownerModuleId == 'secrets');

  // 6. 导入：整体替换 + 画布 id 重生成 + 后续新建不撞 id
  final json = doc.toJson();
  final eng2 = LayoutEngine();
  final restored = eng2.importFrom(Map<String, Object?>.from(json));
  check('导入：画布数量一致', restored.canvases.length == 2);
  check('导入：owner 保留', restored.canvases[1].ownerModuleId == 'secrets');
  check('导入：id 重生成',
      restored.canvases[0].id != doc.canvases[0].id &&
          restored.canvases[1].id != doc.canvases[1].id);
  final after = eng2.newCanvas(restored, '再来一张');
  check('导入后新建不撞 id',
      restored.canvases.map((c) => c.id).toSet().contains(after.id));

  print('');
  print('全部通过: ' + _passed.toString() + ' 项');
}
