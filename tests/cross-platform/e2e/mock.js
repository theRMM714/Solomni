// 一次性 e2e 假供应商：按提示词内容决定回哪种信封（内置工具 / 外部工具 / 讨论 / 回报 / 推荐 / 代拟 / 验收）。
// 路径模型：核心会把"真实根目录"渲染进系统提示词，所以这里**从提示词里解析出根**再用（不在夹具里写死机器路径）。
// 支持流式（j.stream）与非流式。
const http = require('http');
/** 供应商这一侧看到的最后一条请求的消息形状：驱动据此断言"发回去的历史是不是协议形状"。 */
let lastSeen = null;
/** 返工会话里验收被调用的次数（第一次 fail，之后 pass）。 */
let reworkReviews = 0;
// 端口可指定：本机可能残留上一次跑的假供应商占着固定端口，新进程起不来而驱动仍打到旧的。
const PORT = Number(process.env.E2E_MOCK_PORT || 8397);
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
    // 整段对话里的用户消息：工具循环的后续轮里，任务原话已不在最后一条，只有从整段里才看得见。
    const allUser = msgs.filter((m) => m.role === 'user').map((m) => m.content || '').join('\n');
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
    /** 角色工具/核心操作的信封：核心只从**工具调用**里取载荷，正文里手写 JSON 不算调用。 */
    const env = (name, args) => JSON.stringify({ type: 'tool', name, args });
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
      content = env('suggest', { agents: [{ agent: '双子', why: '它正合适' }] });
    } else if (user.includes('== 已存 agent') && user.includes('== 需求 ==')) {
      content = env('slate', {
        picks: [
          { agent: '单兵', why: '单人够用' },
          { name: '新助手', modules: ['research'], model: 'm1', why: '补上调研' },
        ],
      });
    } else if (user.includes('== 讨论至今 ==')) {
      // 讨论轮次的信封按**会话里已经出现的行**路由——状态机验收要按内容构造各种走向。
      // 判据只用转录里的事实（说话人标签 / 轮次标记 / 任务原话），不改产品行为。
      // 判断"这条请求属于哪个 agent"：系统提示词里带该 agent 的全部模块 system，用模块名认。
      // 哪个 agent 在说话：系统提示词里带该 agent 全部模块的 system，用模块特征认（甲=摘要，乙=核对）。
      // 认甲用**模块 id**（系统提示里带模块根目录那行），不要用描述性词：
      // reviewer 的 system 里也有"摘要"（它负责核对摘要），而工具说明里会出现"压缩"之类的通用词——
      // 拿它们当判据都会把乙也认成甲（退场场景于是两人都撤、名单空了）。
      const isJia = sys.includes('summarizer');
      // 到第几轮了：转录里 [轮次 N] 的条数（step 每轮开头推一条）。
      const stepNo = (user.match(/\[轮次 (\d+)\]/g) || []).length;
      const say = (t) => JSON.stringify({ type: 'say', text: t });
      const agree = () => JSON.stringify({ type: 'agree', text: '同意' });
      if (user.includes('上限')) {
        // 谁都不 agree：一路 say 到轮次上限（MAX_ROUNDS=6）。
        content = say('再议一轮');
      } else if (user.includes('退场')) {
        // 甲在第一轮 step 退场；此后它不该再被询问（再被问到 = leave 不可逆被破坏）。
        content = isJia && stepNo <= 1 ? JSON.stringify({ type: 'leave', text: '我撤了' }) : agree();
      } else if (user.includes('提问')) {
        // 甲在第一步请教用户；回答之后（stepNo >= 2）不再问，让流程能收尾。
        content = isJia && stepNo <= 1 ? JSON.stringify({ type: 'ask', text: '用哪个方案？' }) : say('等你定');
      } else if (user.includes('不收敛')) {
        // 首轮：甲说、乙同意 → 有人同意但没全票，不该收敛；次轮甲也同意才收敛。
        content = isJia ? (stepNo <= 1 ? say('我还有意见') : agree()) : agree();
      } else {
        content = agree();
      }
    } else if (user.includes('== 用户需求 ==')) {
      // 把任务原话带进首轮发言：后续轮次的 step 提示**只带转录、不带任务**，
      // 夹具要按会话构造走向，就得让关键字留在转录里（只用转录事实，不改产品行为）。
      const task = (user.split('== 用户需求 ==')[1] || '').trim().split('\n')[0].trim();
      content = JSON.stringify({ type: 'say', text: '我建议直接动手｜' + task });
    } else if (user.includes('== 你的任务 ==')) {
      // 回报本身也是一次工具调用：核心回灌结果后会再问一次，那一次用**纯正文**收尾（无信封 = 这一轮到此为止）。
      content = sawToolResult
        ? '回报已经交了。'
        : env('submit_report', { summary: '做完了', changes: '无外部影响', open: '' });
    } else if (user.includes('== 各节点 ==')) {
      // **节点级验收**：逐节点判"够不够当前目标"。夹具一律判过（要验不通过另设场景）。
      content = env('node_verdict', { verdicts: [{ node: 'n1', ok: true, note: '够用' }] });
    } else if (user.includes('== 方案 ==')) {
      // 返工会话：第一次验收给 fail（定向返工），之后给 pass——用来验"fail → 返工 → 重验 → 交付"闭环。
      if (user.includes('返工')) {
        reworkReviews += 1;
        content = reworkReviews === 1
          ? env('checklist', { items: [{ item: '方案条目', status: 'fail', evidence: '回报', reason: '还差一步（归属：甲）' }] })
          : env('checklist', { items: [{ item: '方案条目', status: 'pass', evidence: '回报' }] });
      } else {
        content = env('checklist', { items: [{ item: '方案条目', status: 'pass', evidence: '回报' }] });
      }
    } else if (user.includes('== 讨论转录 ==')) {
      // 按**用户提示词里的标记**路由，不按工具说明里的措辞：成员与核心的系统提示都会列工具说明，
      // 拿"逐节点核对/总结讨论"这类词当判据会把别的请求也认成核心操作（工具总表一加工具就撞）。
      // 核心整理的回执是**结构化任务链**（plan + nodes）：形状见 prompts/roles/planner.yaml。
  // 负责人要取**提示词里给的名单**（退场场景下甲已不在名单里，写死甲会被自洽门禁如实挡下）。
  const ulines = user.split('\n');
  const ridx = ulines.findIndex((l) => l.includes('名单'));
  const rosterLine = ridx >= 0 ? (ulines[ridx + 1] || '').trim() : '';
  const who = rosterLine.split('、').map((s) => s.trim()).filter(Boolean)[0] || '甲';

  content = user.includes('返工')
    ? env('plan', { plan: '方案：返工一次', nodes: [{ id: 'n1', title: '返工一次', objective: '把事重做一遍', assignee: who, deps: [] }] })
    : env('plan', { plan: '方案：一次把事情做完', nodes: [{ id: 'n1', title: '做完', objective: '把事做完', assignee: who, deps: [] }] });
    } else if (sys.includes('harvest') && allUser.includes('真工具链路')) {
      // 真工具链路：按**整段对话里**已经收到的工具结果条数决定下一个调用（真进程、真三语言模块）。
      // 路径用提示词里给出的真实共享区根目录（相对路径会被围栏拒绝）。
      const w = workRoot || '';
      const n = (allUser.match(/\[工具结果\]/g) || []).length;
      if (n === 0) {
        content = JSON.stringify({ type: 'tool', module: 'harvest', name: 'scan', args: { root: w, out: w + '/corpus.jsonl' } });
      } else if (n === 1) {
        content = JSON.stringify({ type: 'tool', module: 'indexer', name: 'build', args: { corpus: w + '/corpus.jsonl', out: w + '/index.bin' } });
      } else if (n === 2) {
        content = JSON.stringify({ type: 'tool', module: 'indexer', name: 'query', args: { index: w + '/index.bin', q: '检索' } });
      } else if (n === 3) {
        content = env('submit_report', { summary: '语料与索引都做好了', changes: 'corpus.jsonl 与 index.bin', open: '' });
      } else {
        content = '回报已经交了。';
      }
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
}).listen(PORT, '127.0.0.1', () => console.log('MOCK-UP ' + PORT));
