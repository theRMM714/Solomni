// 一次性 e2e 假供应商：按提示词内容决定回哪种信封（内置工具 / 外部工具 / 讨论 / 回报 / 推荐 / 代拟 / 验收）。
// 路径模型：核心会把"真实根目录"渲染进系统提示词，所以这里**从提示词里解析出根**再用（不在夹具里写死机器路径）。
// 支持流式（j.stream）与非流式。
const http = require('http');
http.createServer((req, res) => {
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
    let content;
    if (user.includes('== 编排模式 ==')) {
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
    } else if (sys.includes('内置文件工具') && !user.includes('[工具结果]')) {
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
      } else {
        // 内置 write：路径用**提示词里给出的真实沙箱根**（不写死机器路径）
        const target = (sandboxRoot || 'sandbox-root') + '/mock-note.txt';
        content = JSON.stringify({ type: 'tool', name: 'write', args: { path: target, content: '来自内置工具' } });
      }
    } else {
      content = JSON.stringify({ type: 'say', text: '收到' });
    }
    if (j.stream) {
      res.setHeader('Content-Type', 'text/event-stream');
      res.write('data: ' + JSON.stringify({ choices: [{ delta: { content } }] }) + '\n\n');
      res.write('data: [DONE]\n\n');
      res.end();
      return;
    }
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({ choices: [{ message: { role: 'assistant', content } }] }));
  });
}).listen(8397, '127.0.0.1', () => console.log('MOCK-UP'));
