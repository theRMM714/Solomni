/// 探针：验证真实 OpenAI 兼容端点连通性（无 key，预期 401）。
library;

import 'dart:io';
import 'package:http/http.dart' as http;

Future<void> main(List<String> args) async {
  final url = args.isNotEmpty ? args[0] : 'https://api.deepseek.com/chat/completions';
  try {
    final resp = await http.post(Uri.parse(url),
        headers: {'Authorization': 'Bearer probe-no-key'},
        body: '{"model":"deepseek-chat","messages":[]}');
    print('连通 OK，HTTP ' + resp.statusCode.toString() + '（401=认证拒绝，符合预期）');
  } catch (e) {
    print('连通失败: ' + e.toString());
  }
  exit(0);
}
