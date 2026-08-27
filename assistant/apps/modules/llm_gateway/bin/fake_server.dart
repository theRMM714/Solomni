/// 假 OpenAI 服务器：开发验证用。支持普通与 SSE 流式两种模式。
/// 流式模式分块延时输出，验证 token 增量到达。
library;

import 'dart:convert';
import 'dart:io';

Future<void> main(List<String> args) async {
  var port = 9123;
  for (final a in args) {
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, port);
  print('fake openai server: http://127.0.0.1:' + port.toString() + '/v1/chat/completions');
  await for (final req in server) {
    if (req.method == 'POST' && req.uri.path == '/v1/chat/completions') {
      final body = jsonDecode(await utf8.decoder.bind(req).join()) as Map;
      final messages = body['messages'] as List;
      final last = (messages.last as Map)['content'] as String;
      final content = '假AI回复「' + last + '」（带 ' +
          messages.length.toString() + ' 条历史，模型 ' +
          (body['model']?.toString() ?? '?') + '）';
      if (body['stream'] == true) {
        req.response.statusCode = 200;
        req.response.headers.contentType =
            ContentType('text', 'event-stream', charset: 'utf-8');
        req.response.bufferOutput = false;
        // 切 5 块，每块间隔 120ms，验证消费端逐块收到
        final per = (content.length / 5).ceil();
        for (var i = 0; i < content.length; i += per) {
          final piece = content.substring(i, (i + per).clamp(0, content.length));
          req.response.write('data: ' + jsonEncode({
                'choices': [
                  {'delta': {'content': piece}}
                ]
              }) + '\n\n');
          await req.response.flush();
          await Future.delayed(const Duration(milliseconds: 120));
        }
        req.response.write('data: [DONE]\n\n');
        await req.response.flush();
        await req.response.close();
      } else {
        req.response.statusCode = 200;
        req.response.headers.contentType = ContentType.json;
        req.response.write(jsonEncode({
          'choices': [
            {'message': {'role': 'assistant', 'content': content}}
          ]
        }));
        await req.response.close();
      }
    } else {
      req.response.statusCode = 404;
      await req.response.close();
    }
  }
}
