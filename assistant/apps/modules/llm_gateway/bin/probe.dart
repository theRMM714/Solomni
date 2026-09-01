/// 探针：验证真实 OpenAI 兼容端点连通性（无 key，预期 401）。
library;

import 'dart:io';

Future<void> main(List<String> args) async {
  final url = args.isNotEmpty ? args[0] : 'https://api.deepseek.com/chat/completions';
  final client = HttpClient();
  try {
    final req = await client.postUrl(Uri.parse(url));
    req.headers.set('Authorization', 'Bearer probe-no-key');
    req.headers.set('Content-Type', 'application/json');
    req.write('{"model":"deepseek-chat","messages":[]}');
    final resp = await req.close();
    await resp.drain<void>();
    print('连通 OK，HTTP ' + resp.statusCode.toString() + '（401=认证拒绝，符合预期）');
  } catch (e) {
    print('连通失败: ' + e.toString());
  }
  client.close(force: true);
  exit(0);
}
