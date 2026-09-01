/// 探针：直接测假服务器 SSE 流式响应。
library;

import 'dart:convert';
import 'dart:io';

Future<void> main() async {
  final client = HttpClient();
  final req =
      await client.postUrl(Uri.parse('http://127.0.0.1:9123/v1/chat/completions'));
  req.headers.set('Content-Type', 'application/json');
  req.write(jsonEncode({
    'model': 'fake-model',
    'stream': true,
    'messages': [
      {'role': 'user', 'content': '你好'}
    ]
  }));
  final resp = await req.close();
  print('HTTP ' +
      resp.statusCode.toString() +
      ' type=' +
      (resp.headers.value('content-type') ?? '').toString());
  await for (final line
      in resp.transform(utf8.decoder).transform(const LineSplitter())) {
    print('LINE: ' + line);
    if (line.contains('[DONE]')) break;
  }
  client.close(force: true);
}
