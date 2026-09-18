// 一次性 e2e 假供应商：按提示词内容决定回哪种信封（内置工具 / 外部工具 / 讨论 / 回报 / 推荐 / 代拟 / 验收）。
// 路径模型：核心会把"真实根目录"渲染进系统提示词，所以这里**从提示词里解析出根**再用（不在夹具里写死机器路径）。
// 支持流式（j.stream）与非流式。
const http = require('http');
/** 供应商这一侧看到的最后一条请求的消息形状：驱动据此断言"发回去的历史是不是协议形状"。 */
let lastSeen = null;
http.createServer((req, res) => {
  if (req.url.includes('/__seen')) {
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify(lastSeen || {}));
    return;
  }
  if (req.url.includes('/models')) {
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({ data: [{ id: 'm1' }, { id: 'm2' }] }));
    return;
  }
  let body = '';
  req.on('data', (d) => (body += d));
  req.on('end', () => {
    let j = null;
    try {
      j = JSON.parse(body);
    } catch (e) {
      res.statusCode = 400;
      res.end('{}');
      return;
    }
    const msgs = j.messages || [];
    const sys = (msgs.find((m) => m.role === 'system') || {}).content || '';
    const user = ((msgs.filter((m) => m.role === 'user').pop()) || {}).content || '';
    // 从系统提示词里取真实根目录（sys_tools 里固定有两行：共享区 / 沙箱）
    const workRoot = (sys.match(/本次工作的共享区：([^\n]+)/) || [])[1];
    const sandboxRoot = (sys.match(/你私有的沙箱：([^\n]+)/) || [])[1];
    // 已经跑过工具（手写信封走用户消息，原生通道走 role=tool 的结果消息）。
    const sawToolResult = msgs.some((m) => m.role === 'tool') || user.includes('[工具结果]');
    // 带工具声明、且声明的是探针工具 = 工具调用支持探测（不带声明的那条走普通分支，所以这里只在有 tools 时成立）。
    const wantsPing = (j.tools || []).some((t) => t && t.function && t.function.name === 'solomni_ping');
    let content;
    /** 原生工具调用：给了它就用结构化槽位回（而不是 content）。 */
    let calls = null;
    if (wantsPing) {
      calls = [{ id: 'call_probe', type: 'function', function: { name: 'solomni_ping', arguments: '{}' } }];
    } else if (sys.includes('内置文件工具') && user.includes('原生多调用') && !sawToolResult) {
      // 原生通道：一次回复里给**两个**调用（各写一个文件）——验证协议形状与"一条助手消息 + N 条结果"。
      const a = (sandboxRoot || 'sandbox-root') + '/native-a.txt';
      const b = (sandboxRoot || 'sandbox-root') + '/native-b.txt';
      calls = [
        { id: 'call_a', type: 'function', function: { name: 'write', arguments: JSON.stringify({ path: a, content: '第一个' }) } },
        { id: 'call_b', type: 'function', function: { name: 'write', arguments: JSON.stringify({ path: b, content: '第二个' }) } },
      ];
    } else if (user.includes('== 编排模式 ==')) {
      content = JSON.stringify({ agents: [{ agent: '双子', why: '它正合适' }] });
    } else if (user.includes('== 已存 agent') && user.includes('== 需求 ==')) {
      content = JSON.stringify({
        picks: [
          { agent: '单兵', why: '单人够用' },
          { name: '新助手', modules: ['research'], model: 'm1', why: '补上调研' },
        ],
      });
    } else if (user.includes('== 讨论至今 ==')) {
      content = JSON.stringify({ type: 'agree', text: '同意' });
    } else if (user.includes('== 用户需求 ==')) {
      content = JSON.stringify({ type: 'say', text: '我建议直接动手' });
    } else if (user.includes('== 你的任务 ==')) {
      content = JSON.stringify({ summary: '做完了', changes: '无外部影响', open: '' });
    } else if (sys.includes('核心验收者') || user.includes('== 方案 ==')) {
      content = JSON.stringify([{ item: '方案条目', status: 'pass', evidence: '回报' }]);
    } else if (sys.includes('总结讨论')) {
      content = '方案：一次把事情做完';
    } else if (sys.includes('内置文件工具') && !sawToolResult) {
      if (/read_txt/.test(sys)) {
        // 该 agent 的某个模块声明了外部工具（夹具 toolbox）：用**相对路径**调用，专门验证 cwd = 它自己的模块目录。
        const env = { type: 'tool', module: 'toolbox', name: 'read_txt', args: { path: 'userdata/e2e.txt' } };
        if (user.includes('不要写模块')) {
          delete env.module; // 故意漏 module：验证核心如实报错而不猜
          content = JSON.stringify(env);
        } else if (user.includes('半截信封')) {
          content = '好的。{"type":"tool","name":"write","args":{"path":"a"}';
        } else if (user.includes('坏信封')) {
          content = '{"type":"tool","name":"write","args":{"path":"a.md","content":"abc"}]}';
        } else if (user.includes('绝对路径')) {
          // 外部工具 + **真实绝对路径**（用户投喂的文件）：这正是最初 read_txt 收到 work:/ 直接报错的那个场景
          content = JSON.stringify({ type: 'tool', module: 'toolbox', name: 'read_txt', args: { path: (workRoot || '') + '/g.txt' } });
        } else if (user.includes('顺便')) {
          content = '我先看一下这个文件。' + JSON.stringify(env);
        } else {
          content = JSON.stringify(env);
        }
      } else if (user.includes('打补丁')) {
        // 自由格式补丁：信封之后**原样**跟补丁正文（不转义、不引号）——验证这条路径在真实二进制上通
        const target = (sandboxRoot || 'sandbox-root') + '/mock-patch.txt';
        content = '{"type":"tool","name":"patch"}\n'
          + '*** Add File: ' + target + '\n'
          + '补丁第一行\n'
          + '补丁第二行「引号、换行、冒号：都不用转义」\n'
          + '*** End File\n'
          + '补丁已经给出。';
      } else {
        // 内置 write：路径用**提示词里给出的真实沙箱根**（不写死机器路径）
        const target = (sandboxRoot || 'sandbox-root') + '/mock-note.txt';
        content = JSON.stringify({ type: 'tool', name: 'write', args: { path: target, content: '来自内置工具' } });
      }
    } else {
      content = JSON.stringify({ type: 'say', text: '收到' });
    }
    // 记下这一轮请求的**消息形状**（驱动要断言协议形状真的发出去了）。
    lastSeen = {
      assistantWithCalls: msgs.filter((m) => m.role === 'assistant' && Array.isArray(m.tool_calls) && m.tool_calls.length > 0).length,
      toolMsgs: msgs.filter((m) => m.role === 'tool').length,
      toolIds: msgs.filter((m) => m.role === 'tool').map((m) => m.tool_call_id),
    };
    if (j.stream) {
      res.setHeader('Content-Type', 'text/event-stream');
      if (calls) {
        // 分片按 index 来（与真实供应商同形）：name/id 在第一片，arguments 一片给全。
        calls.forEach((c, i) => {
          res.write('data: ' + JSON.stringify({
            choices: [{ delta: { tool_calls: [{ index: i, id: c.id, type: 'function', function: { name: c.function.name, arguments: c.function.arguments } }] } }],
          }) + '\n\n');
        });
        res.write('data: ' + JSON.stringify({ choices: [{ delta: {}, finish_reason: 'tool_calls' }] }) + '\n\n');
      } else {
        res.write('data: ' + JSON.stringify({ choices: [{ delta: { content } }] }) + '\n\n');
      }
      res.write('data: [DONE]\n\n');
      res.end();
      return;
    }
    res.setHeader('Content-Type', 'application/json');
    if (calls) {
      res.end(JSON.stringify({ choices: [{ message: { role: 'assistant', content: '', tool_calls: calls }, finish_reason: 'tool_calls' }] }));
      return;
    }
    res.end(JSON.stringify({ choices: [{ message: { role: 'assistant', content } }] }));
  });
}).listen(8397, '127.0.0.1', () => console.log('MOCK-UP'));
