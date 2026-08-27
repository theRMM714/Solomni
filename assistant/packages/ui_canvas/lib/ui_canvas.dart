/// 画布模型：UI 模块私有的布局数据与规则。
/// 布局归 UI 模块（呈现主权）；组件 id 是模块契约，此处只引用不改名。
/// scope 规则：public 组件任意画布可放；private 仅其所属模块的画布。
library;

import 'package:ui_vocab/ui_vocab.dart';

/// 调色板条目：来自某模块贡献的一个组件
class PaletteEntry {
  final String moduleId;
  final UiComponent component;
  const PaletteEntry(this.moduleId, this.component);

  bool get isPrivate => component.scope == UiScope.private;
}

/// 一个模块的完整贡献（组件 + 命令）
class Contributed {
  final String moduleId;
  final UiContribution contribution;
  const Contributed(this.moduleId, this.contribution);
}

/// 画布上的一个放置：指向组件（模块 id + 组件 id），带位置与尺寸
class Placement {
  final String moduleId;
  final String componentId;
  double x, y, w, h;
  Placement(this.moduleId, this.componentId,
      {this.x = 0, this.y = 0, this.w = 260, this.h = 160});

  Map<String, Object?> toJson() => {
        'module': moduleId,
        'component': componentId,
        'x': x,
        'y': y,
        'w': w,
        'h': h,
      };

  factory Placement.fromJson(Map<String, Object?> j) => Placement(
        j['module'] as String,
        j['component'] as String,
        x: (j['x'] as num?)?.toDouble() ?? 0,
        y: (j['y'] as num?)?.toDouble() ?? 0,
        w: (j['w'] as num?)?.toDouble() ?? 260,
        h: (j['h'] as num?)?.toDouble() ?? 160,
      );
}

/// 画布：ownerModuleId 为 null 是公共画布，否则是模块私有画布
class CanvasDoc {
  final String id;
  String title;
  final String? ownerModuleId;
  final placements = <Placement>[];

  CanvasDoc(this.id, this.title, {this.ownerModuleId});

  Map<String, Object?> toJson() => {
        'id': id,
        'title': title,
        'owner': ownerModuleId,
        'placements': [for (final p in placements) p.toJson()],
      };

  factory CanvasDoc.fromJson(Map<String, Object?> j) {
    final c = CanvasDoc(j['id'] as String, j['title'] as String? ?? '',
        ownerModuleId: j['owner'] as String?);
    for (final p in (j['placements'] as List? ?? const [])) {
      c.placements.add(Placement.fromJson(Map<String, Object?>.from(p as Map)));
    }
    return c;
  }
}

/// 布局文档：UI 模块私有的全部画布（可持久化）
class LayoutDoc {
  final canvases = <CanvasDoc>[];

  LayoutDoc();

  Map<String, Object?> toJson() => {
        'canvases': [for (final c in canvases) c.toJson()],
      };

  factory LayoutDoc.fromJson(Map<String, Object?> j) {
    final d = LayoutDoc();
    for (final c in (j['canvases'] as List? ?? const [])) {
      d.canvases.add(CanvasDoc.fromJson(Map<String, Object?>.from(c as Map)));
    }
    return d;
  }
}

/// 布局引擎：从贡献生成初始画布 + 放置规则 + 增删改
class LayoutEngine {
  var _seq = 0;
  String _newId() => 'canvas-' + (_seq++).toString();

  /// 初始布局：public 组件进默认画布；各模块 private 组件进其私有画布
  LayoutDoc initial(List<Contributed> found) {
    final doc = LayoutDoc();
    final def = CanvasDoc(_newId(), '默认画布');
    var col = 0;
    for (final c in found) {
      for (final comp in c.contribution.components) {
        if (comp.scope == UiScope.public) {
          def.placements.add(Placement(c.moduleId, comp.id,
              x: 24.0 + (col % 2) * 300, y: 24.0 + (col ~/ 2) * 200));
          col++;
        }
      }
    }
    if (def.placements.isNotEmpty) doc.canvases.add(def);
    for (final c in found) {
      final privates = c.contribution.components
          .where((k) => k.scope == UiScope.private)
          .toList();
      if (privates.isEmpty) continue;
      final own = CanvasDoc(_newId(), c.moduleId + ' 私有画布',
          ownerModuleId: c.moduleId);
      var i = 0;
      for (final comp in privates) {
        own.placements.add(Placement(c.moduleId, comp.id,
            x: 24.0 + (i % 2) * 300, y: 24.0 + (i ~/ 2) * 200));
        i++;
      }
      doc.canvases.add(own);
    }
    return doc;
  }

  /// 放置规则：private 仅其所属模块的画布；public 任意画布
  bool canPlace(CanvasDoc canvas, PaletteEntry entry) =>
      entry.isPrivate ? canvas.ownerModuleId == entry.moduleId : true;

  /// 想用哪个组件就拖进去：校验规则后放置
  bool place(CanvasDoc canvas, PaletteEntry entry, {double x = 24, double y = 24}) {
    if (!canPlace(canvas, entry)) return false;
    canvas.placements.add(Placement(entry.moduleId, entry.component.id, x: x, y: y));
    return true;
  }

  CanvasDoc newCanvas(LayoutDoc doc, String title, {String? ownerModuleId}) {
    final c = CanvasDoc(_newId(), title, ownerModuleId: ownerModuleId);
    doc.canvases.add(c);
    return c;
  }

  void move(Placement p, double dx, double dy) {
    p.x += dx;
    p.y += dy;
  }

  void resize(Placement p, double dw, double dh) {
    p.w = (p.w + dw).clamp(120, 2000);
    p.h = (p.h + dh).clamp(60, 2000);
  }

  void remove(CanvasDoc canvas, Placement p) => canvas.placements.remove(p);
}
