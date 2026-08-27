/// 探针：直接测假服务器 SSE 流式响应。
library;

import 'dart:async';
import 'dart:convert';
import 'package:http/http.dart' as http;

Future<void> main() async {
  final req = http.Request('POST', Uri.parse('http://127.0.0.1:9123/v1/chat/completions'))
    ..headers['Content-Type'] = 'application/json'
    ..body = jsonEncode({
      'model': 'fake-model',
      'stream': true,
      'messages': [
        {'role': 'user', 'content': '你好'}
      ]
    });
  final resp = await http.Client().send(req);
  print('HTTP ' + resp.statusCode.toString() + ' type=' + resp.headers['content-type'].toString());
  await for (final line in resp.stream.transform(utf8.decoder).transform(const LineSplitter())) {
    print('LINE: ' + line);
    if (line.contains('[DONE]')) break;
  }
}
