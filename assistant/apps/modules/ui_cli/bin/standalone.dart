/// 单机入口：无核心 -> 无模块 -> 调色板为空（UI 模块自治运行不崩溃）。
library;

import 'dart:io';

Future<void> main() async {
  print('[ui_cli] 无核心连接：调色板为空，画布为空。standalone 模式仅展示本模块可独立运行。');
  exit(0);
}
