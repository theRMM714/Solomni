/// 单机入口：无核心、无共享，全内建。
library;

import 'dart:io';
import 'package:conversation/conversation.dart';

Future<void> main() async {
  final conv = ConversationModule(DirectLlm());
  await for (final token in conv.send('你好')) {
    stdout.write(token);
  }
  stdout.writeln();
}
