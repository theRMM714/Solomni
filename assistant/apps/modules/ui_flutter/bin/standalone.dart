/// 单机入口：无核心，空调色板（UI 模块自治运行不崩溃）。
/// 运行：flutter run -t bin/standalone.dart -d windows
library;

import 'package:flutter/material.dart';
import 'package:ui_flutter/widgets/shell.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const UiShell());
}
