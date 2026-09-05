/// 协作入口：Flutter UI 模块连到核心（--port=9100），自己一个窗口。
/// 运行：flutter run -t bin/coordinated.dart -d windows
/// 模块目录由 dev/dev.bat 注入（--module-dir=<模块根>）：Flutter 运行时
/// 没有 package_config（Isolate.resolvePackageUriSync 返回 null），
/// 私有数据路径必须显式传入，不能靠包根反推。
library;

import 'dart:io';
import 'package:flutter/material.dart';import 'package:transport/transport.dart';
import 'package:ui_flutter/layout_store.dart';
import 'package:ui_flutter/ui_flutter.dart';
import 'package:ui_flutter/widgets/shell.dart';

Future<void> main(List<String> args) async {
  // 参数双形态：--port=N 与 --port N（Windows 的 cmd 批参数以 = 分隔，
  // dev.bat 侧会把 --port=N 拆成两个参数送进来）
  var port = 9100;
  var moduleDir = '';
  for (var i = 0; i < args.length; i++) {
    if (args[i] == '--port' && i + 1 < args.length) {
      port = int.parse(args[i + 1]);
    } else if (args[i].startsWith('--port=')) {
      port = int.parse(args[i].substring(7));
    } else if (args[i] == '--module-dir' && i + 1 < args.length) {
      moduleDir = args[i + 1];
    } else if (args[i].startsWith('--module-dir=')) {
      moduleDir = args[i].substring(13);
    }
  }
  WidgetsFlutterBinding.ensureInitialized();
  final program = UiFlutterProgram();
  await ModuleClient.connect(InternetAddress.loopbackIPv4, port, program);
  // 布局存储：dev 契约注入模块目录 -> <模块根>/userdata/layout.json（确定路径）；
  // 未注入时走默认（退化到运行目录，仅影响落盘位置不影响功能）
  final store = moduleDir.isEmpty
      ? null
      : LayoutStore(file: File(moduleDir + '/userdata/layout.json'));
  runApp(UiShell(outbound: program.outbound, store: store));
}
