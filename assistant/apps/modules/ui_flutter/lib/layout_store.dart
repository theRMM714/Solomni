/// 布局持久化：布局数据归 UI 模块私有，落在模块自己文件夹内 userdata/。
library;

import 'dart:convert';
import 'dart:io';
import 'package:ui_canvas/ui_canvas.dart';
import 'package:user_data/user_data.dart';

class LayoutStore {
  final File _file;

  LayoutStore({File? file})
      : _file = file ?? UserData.file('ui_flutter', 'layout.json');

  /// 导出默认路径：与布局存储同目录（导入导出是用户数据可携带性）
  String get defaultExportPath => _file.parent.path + '/layout-export.json';

  Future<LayoutDoc?> load() async {
    if (!await _file.exists()) return null;
    try {
      return LayoutDoc.fromJson(
          jsonDecode(await _file.readAsString()) as Map<String, Object?>);
    } catch (_) {
      return null; // 布局损坏则重建，不崩溃
    }
  }

  Future<void> save(LayoutDoc doc) async {
    await _file.parent.create(recursive: true);
    await _file.writeAsString(jsonEncode(doc.toJson()));
  }
}
