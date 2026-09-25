// e2e 驱动（L4，由 orchestrator.js 调起）：隔离根内跑，绝不动真实 .home/ 与 session/。
// 覆盖：agent 登记处 → 推荐复用 → 单 agent（1 个 agent 带多模块）+ 内置 write 落私沙箱
//       → 协作（非代拟）跑完交付 → 代拟（复用+组装）确认后名单写回 meta 并建出沙箱
//       → 外部工具 cwd / 绝对路径 / 自由格式补丁 / 正文+信封 / 原生多调用（协议形状由假供应商核对）。
const BASE = process.env.E2E_BASE || 'http://127.0.0.1:3099';
// 假供应商端口由编排器指定：本机可能残留上一次的进程，固定端口会让驱动打到旧的那个。
const MOCK_BASE = process.env.E2E_MOCK_BASE || 'http://127.0.0.1:8397';
const fs = require('fs');
const path = require('path');
// 夹具根：本目录下的 root/（隔离根：prompts/、.home、modules 都在里面）。
const ROOT = path.join(__dirname, 'root');

async function api(method, p, body) {
  const r = await fetch(BASE + p, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch {}
  return { status: r.status, json, text };
}
let failed = 0;
function assert(cond, label, extra) {
  if (!cond) failed++;
  console.log((cond ? 'PASS ' : 'FAIL ') + label + (cond || extra === undefined ? '' : ' :: ' + extra));
}
const dir = (name) => path.join(ROOT, 'session', name);
/** 假供应商那一侧看到的最后一条请求（它记下了消息形状）：原生通道的协议形状只能在这里验。 */
async function mockSeen() {
  try {
    const r = await fetch(MOCK_BASE + '/__seen');
    return await r.json();
  } catch {
    return null;
  }
}
/** 该工作的转录行（从落盘流水取，已应用回档截断）。 */
async function lines(sid) {
  const r = await api('GET', '/api/history/' + encodeURIComponent(sid));
  const out = [];
  for (const ev of (r.json && r.json.events) || []) {
    if (ev.type === 'transcript') for (const l of ev.lines || []) out.push(l);
  }
  return out;
}

(async () => {
  for (let i = 0; i < 60; i++) {
    try { const s = await api('GET', '/api/state'); if (s.status === 200) break; } catch {}
    await new Promise((r) => setTimeout(r, 250));
  }
  const st = await api('GET', '/api/state');
  assert(st.status === 200 && Array.isArray(st.json.agents), 'GET /api/state 带 agents', st.text.slice(0, 120));

  // 登记处：一个供应商 + 两个模型
  assert((await api('POST', '/api/providers', { id: 'mock', base_url: MOCK_BASE + '/v1', api_key: 'k' })).status === 200, '登记供应商');
  assert((await api('POST', '/api/models', { id: 'm1', name: 'M1', api_model: 'm1', provider: 'mock', note: '' })).status === 200, '登记模型 m1');
  assert((await api('POST', '/api/models', { id: 'm2', name: 'M2', api_model: 'm2', provider: 'mock', note: '' })).status === 200, '登记模型 m2');
  assert((await api('POST', '/api/models/m1/core')).status === 200, '设核心默认 m1');
  assert((await api('POST', '/api/agents', { name: '单兵', modules: ['summarizer'], model: 'm1', note: 'e2e' })).status === 200, '登记 agent 单兵');
  assert((await api('POST', '/api/agents', { name: '双子', modules: ['research', 'reviewer'], model: 'm2', note: 'e2e 多模块' })).status === 200, '登记 agent 双子（2 模块 / m2）');

  // 推荐：核心优先复用登记处里的 agent（不代拟模型）
  const sug = await api('POST', '/api/suggest-models', { task: '调研并总结', mode: 'single' });
  assert(sug.status === 200, 'POST /api/suggest-models(mode=single)', sug.text.slice(0, 160));
  const rec = (sug.json && sug.json.agents && sug.json.agents[0]) || {};
  assert(rec.reuse === true, '复用项带 reuse=true', JSON.stringify(rec));
  assert(rec.name === '双子' && rec.model === 'm2', '复用沿用登记处的模型（不代拟）', JSON.stringify(rec));
  assert((rec.modules || []).join() === 'research,reviewer', '复用带全部模块（不截断）', JSON.stringify(rec.modules));

  // 单 agent（模块无外部工具）：内置 write 落在该 agent 私沙箱
  const name1 = 'e2e-single-' + Date.now();
  const c1 = await api('POST', '/api/sessions', {
    name: name1, mode: 'single',
    agents: [{ name: '专属', transient: true, modules: ['research'], model: 'm1' }],
  });
  assert(c1.status === 200, '建单 agent 工作', c1.text.slice(0, 200));
  const meta1 = fs.readFileSync(path.join(dir(name1), 'meta.yaml'), 'utf8');
  assert(/mode: single/.test(meta1), 'meta.mode = single', meta1.split('\n').slice(0, 6).join(' | '));

  const said = await api('POST', '/api/sessions/' + encodeURIComponent(name1) + '/say', { text: '记一笔' });
  assert(said.status === 200, '单 agent 发言', said.text.slice(0, 200));
  const note = path.join(dir(name1), '专属', 'mock-note.txt');
  assert(fs.existsSync(note) && fs.readFileSync(note, 'utf8') === '来自内置工具', '内置 write 落在该 agent 私沙箱', note);

  // 转录结构：一轮模型调用 = 一行，工具调用自成一条 tool 行
  const l1 = await lines(name1);
  assert(l1.map((x) => x.id).join() === '0,1,2', '一次工具循环 = 用户行 + 工具行 + 文本行（id 连续）', JSON.stringify(l1.map((x) => x.id)));
  const t1 = (l1[1] || {}).tool || {};
  assert(t1.name === 'write' && t1.ok === true && t1.module === '', '工具调用是一条 tool 行（内置工具 module 为空）', JSON.stringify(t1).slice(0, 200));
  assert(l1[2] && !l1[2].tool, '最后一行是纯文本行', JSON.stringify(l1[2] || {}).slice(0, 160));
  // 回档：删除该行及其后（点第一行 → 转录清空）
  assert((await api('POST', '/api/sessions/' + encodeURIComponent(name1) + '/rewind', { id: 0 })).status === 200, '回档到第一行');
  const l1r = await lines(name1);
  assert(l1r.length === 0, '删除第一行后转录清空', JSON.stringify(l1r).slice(0, 160));

  // 单 agent 带 2 个模块，且外部工具属于「第二个」模块：必须在它自己模块的目录里跑
  fs.mkdirSync(path.join(ROOT, 'modules', 'toolbox', 'userdata'), { recursive: true });
  fs.writeFileSync(path.join(ROOT, 'modules', 'toolbox', 'userdata', 'e2e.txt'), '第一行内容\n第二行内容\n');
  const name1b = 'e2e-multi-' + Date.now();
  const c1b = await api('POST', '/api/sessions', {
    name: name1b, mode: 'single',
    agents: [{ name: '双子形', transient: true, modules: ['research', 'toolbox'], model: 'm1' }],
  });
  assert(c1b.status === 200, '建单 agent 工作（1 个 agent 带 2 个模块）', c1b.text.slice(0, 200));
  const meta1b = fs.readFileSync(path.join(dir(name1b), 'meta.yaml'), 'utf8');
  assert((meta1b.match(/- research|- toolbox/g) || []).length >= 2, 'meta 记录了该 agent 的两个模块');
  const said1b = await api('POST', '/api/sessions/' + encodeURIComponent(name1b) + '/say', { text: '读一下' });
  assert(said1b.status === 200, '多模块 agent 发言', said1b.text.slice(0, 200));
  const lb = await lines(name1b);
  const tb = (lb[1] || {}).tool || {};
  assert(tb.module === 'toolbox' && tb.name === 'read_txt' && tb.ok === true, '外部工具用它自己模块的目录当 cwd 并成功（落成 tool 行）', JSON.stringify(tb).slice(0, 240));
  assert(String(tb.output || '').includes('第二行内容'), '工具结果原文进了转录（重启/回档后仍可回放）', String(tb.output).slice(0, 160));

  // 多模块 agent 漏写 module：核心如实报错并列出可用的 模块.工具（不猜）
  const said1c = await api('POST', '/api/sessions/' + encodeURIComponent(name1b) + '/say', { text: '不要写模块，直接读' });
  assert(said1c.status === 200, '漏写 module 时仍能发言（不崩）', said1c.text.slice(0, 200));
  const lc = await lines(name1b);
  const fail = lc.filter((x) => x.tool).pop() || {};
  const ft = fail.tool || {};
  assert(ft.ok === false && /module/i.test(String(ft.output || '')), '漏写 module → tool 行记为失败并如实说明', JSON.stringify(ft).slice(0, 240));
  assert(String(ft.output || '').includes('toolbox.read_txt'), '报错里列出可用的 模块.工具', String(ft.output).slice(0, 240));

  // 上传（同名不覆盖）＋ 文件清单 ＋ @ 引用改写
  const b64 = Buffer.from('用户投喂').toString('base64');
  const up1 = await api('POST', '/api/sessions/' + encodeURIComponent(name1) + '/upload', { name: 'f.txt', data_base64: b64 });
  const up2 = await api('POST', '/api/sessions/' + encodeURIComponent(name1) + '/upload', { name: 'f.txt', data_base64: b64 });
  assert(up1.status === 200 && up2.status === 409, '上传同名返回 409（不静默覆盖）', up1.status + '/' + up2.status);
  assert(fs.existsSync(path.join(dir(name1), 'work', 'f.txt')), '投喂落在 work/');
  const files = await api('GET', '/api/sessions/' + encodeURIComponent(name1) + '/files');
  assert(files.status === 200 && (files.json.work || []).includes('f.txt'), 'GET /files 列出共享区文件', files.text.slice(0, 200));
  const af = (files.json.agents || [])[0] || {};
  assert((af.files || []).includes('mock-note.txt'), '/files 列出该 agent 沙箱里的成品', JSON.stringify(af).slice(0, 200));
  const rr = files.json.roots || {};
  assert(typeof rr.work === 'string' && /\/work$/.test(rr.work), '/files 返回共享区真实根（/ 分隔）', JSON.stringify(rr).slice(0, 220));
  const ra = (rr.agents || [])[0] || {};
  assert(String(ra.name || '') !== '' && String(ra.root || '').endsWith('/' + ra.name), '/files 返回 agent 沙箱真实根（与 agents 同序同名）', JSON.stringify(ra).slice(0, 220));
  const saidRef = await api('POST', '/api/sessions/' + encodeURIComponent(name1) + '/say', { text: '@work:f.txt 看一下' });
  assert(saidRef.status === 200, '@ 引用发言', saidRef.text.slice(0, 200));
  const lr = await lines(name1);
  const ul = lr.filter((x) => String(x.line).startsWith('[用户')).pop() || {};
  assert(!/@work:/.test(String(ul.line)) && /work[\\/]+f\.txt/.test(String(ul.line)), '核心把 @work:… 改写成真实绝对路径', String(ul.line).slice(0, 200));

  // 句读标点不进路径：@work:f.txt，看一下 → work:/f.txt，看一下
  const saidPunct = await api('POST', '/api/sessions/' + encodeURIComponent(name1) + '/say', { text: '@work:f.txt，看一下' });
  assert(saidPunct.status === 200, '带句读的引用发言', saidPunct.text.slice(0, 200));
  const lp = await lines(name1);
  const upl = lp.filter((x) => String(x.line).startsWith('[用户')).pop() || {};
  assert(/work[\\/]+f\.txt，看一下/.test(String(upl.line)), '句读标点不入路径（留在正文里）', String(upl.line).slice(0, 200));

  // 文件名含空格：引用用引号包住路径 → 仍能改写成正确路径
  const spaced = '项目 说明.md';
  const upSpace = await api('POST', '/api/sessions/' + encodeURIComponent(name1) + '/upload', { name: spaced, data_base64: b64 });
  assert(upSpace.status === 200, '上传含空格文件名的文件', upSpace.text.slice(0, 160));
  const saidSpace = await api('POST', '/api/sessions/' + encodeURIComponent(name1) + '/say', { text: '@work:"' + spaced + '" 看一下' });
  assert(saidSpace.status === 200, '引用含空格文件名', saidSpace.text.slice(0, 200));
  const ls = await lines(name1);
  const usl = ls.filter((x) => String(x.line).startsWith('[用户')).pop() || {};
  assert(new RegExp('项目 说明\\.md').test(String(usl.line)) && !/@work:/.test(String(usl.line)), '引号内的空格路径被完整改写（真实路径）', String(usl.line).slice(0, 240));

  // 非法工具信封：必须记成一条失败的工具行，且 JSON 绝不上屏（真实 bug 的回归）
  const name1f = 'e2e-badenv-' + Date.now();
  const c1f = await api('POST', '/api/sessions', {
    name: name1f, mode: 'single',
    agents: [{ name: '坏信封手', transient: true, modules: ['toolbox'], model: 'm1' }],
  });
  assert(c1f.status === 200, '建「坏信封」工作', c1f.text.slice(0, 200));
  const s1f = await api('POST', '/api/sessions/' + encodeURIComponent(name1f) + '/say', { text: '坏信封' });
  assert(s1f.status === 200, '非法信封发言', s1f.text.slice(0, 200));
  const lf = await lines(name1f);
  const tf = lf.filter((x) => x.tool).pop() || {};
  assert(tf.tool && tf.tool.ok === false && tf.tool.name === 'write', '非法信封 → 一条 ok=false 的工具行（名字打捞为 write）', JSON.stringify(tf.tool || {}).slice(0, 200));
  assert(lf.every((x) => !String(x.line || '').includes('"type"')), '非法信封的 JSON 没有上屏', JSON.stringify(lf.map((x) => String(x.line || '').slice(0, 40))));
  assert(!fs.existsSync(path.join(dir(name1f), '坏信封手', 'a.md')), '非法信封没有被执行（沙箱里没有 a.md）');

  // 正文在前、坏信封在后且未闭合：正文保留，坏 JSON 不上屏
  const name1g = 'e2e-unclosed-' + Date.now();
  const c1g = await api('POST', '/api/sessions', {
    name: name1g, mode: 'single',
    agents: [{ name: '半截手', transient: true, modules: ['toolbox'], model: 'm1' }],
  });
  assert(c1g.status === 200, '建「半截信封」工作', c1g.text.slice(0, 200));
  const s1g = await api('POST', '/api/sessions/' + encodeURIComponent(name1g) + '/say', { text: '半截信封' });
  assert(s1g.status === 200, '半截信封发言', s1g.text.slice(0, 200));
  const lu = await lines(name1g);
  const tu = lu.filter((x) => x.tool).pop() || {};
  assert(tu.tool && tu.tool.ok === false && tu.tool.name === 'write', '未闭合信封也记成失败的工具行', JSON.stringify(tu.tool || {}).slice(0, 200));
  assert(lu.some((x) => String(x.line || '').includes('好的。') && !String(x.line || '').includes('{')), '正文被保留下来（未闭合的坏 JSON 没有混进去）', JSON.stringify(lu.map((x) => String(x.line || '').slice(0, 40))));
  assert(lu.every((x) => !String(x.line || '').includes('"type"')), '未闭合信封的 JSON 没有上屏', JSON.stringify(lu.map((x) => String(x.line || '').slice(0, 40))));

  // 外部工具 + 真实绝对路径读用户投喂的文件（最初那个"read_txt 不认识路径"的场景，现在必须是绿的）
  const name1h = 'e2e-abspath-' + Date.now();
  const c1h = await api('POST', '/api/sessions', {
    name: name1h, mode: 'single',
    agents: [{ name: '工具手', transient: true, modules: ['toolbox'], model: 'm1' }],
  });
  assert(c1h.status === 200, '建「绝对路径」工作', c1h.text.slice(0, 200));
  const upH = await api('POST', '/api/sessions/' + encodeURIComponent(name1h) + '/upload', {
    name: 'g.txt', data_base64: Buffer.from('绝对路径能读到').toString('base64'),
  });
  assert(upH.status === 200, '投喂 g.txt', upH.text.slice(0, 160));
  const s1h = await api('POST', '/api/sessions/' + encodeURIComponent(name1h) + '/say', { text: '绝对路径读一下' });
  assert(s1h.status === 200, '绝对路径发言', s1h.text.slice(0, 200));
  const lh = await lines(name1h);
  const th = lh.filter((x) => x.tool).pop() || {};
  assert(th.tool && th.tool.module === 'toolbox' && th.tool.name === 'read_txt' && th.tool.ok === true, '模块的外部工具用真实绝对路径读到了用户投喂的文件', JSON.stringify(th.tool || {}).slice(0, 260));
  assert(String((th.tool || {}).output || '').includes('绝对路径能读到'), '工具真的读到了内容（cwd 与路径都对）', String((th.tool || {}).output).slice(0, 200));

  // 自由格式补丁：信封 + 之后原样跟补丁正文（不转义）——这条新路径必须在真实二进制上通
  const name1p = 'e2e-patch-' + Date.now();
  const c1p = await api('POST', '/api/sessions', {
    name: name1p, mode: 'single',
    agents: [{ name: '补丁手', transient: true, modules: ['research'], model: 'm1' }],
  });
  assert(c1p.status === 200, '建「补丁」工作', c1p.text.slice(0, 200));
  const s1p = await api('POST', '/api/sessions/' + encodeURIComponent(name1p) + '/say', { text: '打补丁' });
  assert(s1p.status === 200, '补丁发言', s1p.text.slice(0, 200));
  const lpatch = await lines(name1p);
  const tpatch = lpatch.filter((x) => x.tool).pop() || {};
  assert(tpatch.tool && tpatch.tool.name === 'patch' && tpatch.tool.ok === true, '自由格式补丁落成一条成功的工具行', JSON.stringify(tpatch.tool || {}).slice(0, 260));
  const patched = path.join(dir(name1p), '补丁手', 'mock-patch.txt');
  assert(fs.existsSync(patched), '补丁真的写下了文件');
  assert(
    fs.readFileSync(patched, 'utf8') === '补丁第一行\n补丁第二行「引号、换行、冒号：都不用转义」',
    '补丁内容原样落盘（引号/换行/中文都没被转义）',
    JSON.stringify(fs.readFileSync(patched, 'utf8').slice(0, 200)),
  );
  assert(
    lpatch.every((x) => !String(x.line || '').includes('*** Add File') && !String(x.line || '').includes('补丁已经给出')),
    '补丁正文与它后面的散话都没有上屏（它是工具输入，不是发言）',
    JSON.stringify(lpatch.map((x) => String(x.line || '').slice(0, 40))),
  );

  // 协作里引用某个 agent 的私沙：如实说明只有那个 agent 能读（speaker = None）
  const name1e = 'e2e-refcollab-' + Date.now();
  const c1e = await api('POST', '/api/sessions', {
    name: name1e, mode: 'collab', task: '@sandbox:甲/秘密.md 只看这个',
    agents: [{ name: '甲', transient: true, modules: ['summarizer'], model: 'm1' }],
  });
  assert(c1e.status === 200, '建引用私沙的协作工作', c1e.text.slice(0, 200));
  const le = await lines(name1e);
  const demand = le.filter((x) => String(x.line).includes('需求')).pop() || {};
  assert(String(demand.line).includes('秘密.md') && String(demand.line).includes('能读') && !/@sandbox:/.test(String(demand.line)), '协作里引用私沙 → 改写并如实说明只有该 agent 能读', String(demand.line).slice(0, 200));
  assert(!String(demand.line).includes('）.md') && !String(demand.line).includes(') .md'), '改写后不残留路径尾巴（扩展名没被句读切断）', String(demand.line).slice(0, 200));

  // 同一轮"先写正文再发工具信封"：正文进转录，信封 JSON 不出现在任何行里
  const name1d = 'e2e-prose-' + Date.now();
  const c1d = await api('POST', '/api/sessions', {
    name: name1d, mode: 'single',
    agents: [{ name: '双子形2', transient: true, modules: ['research', 'toolbox'], model: 'm1' }],
  });
  assert(c1d.status === 200, '建「正文+工具」工作', c1d.text.slice(0, 200));
  const sd = await api('POST', '/api/sessions/' + encodeURIComponent(name1d) + '/say', { text: '顺便读一下' });
  assert(sd.status === 200, '「正文+工具」发言', sd.text.slice(0, 200));
  const ld = await lines(name1d);
  assert(ld.map((x) => x.id).join() === '0,1,2,3', '正文行 + tool 行 + 答复行（4 条，id 连续）', JSON.stringify(ld.map((x) => x.id)));
  assert(ld[1] && String(ld[1].line).includes('我先看一下这个文件。') && !String(ld[1].line).includes('{'), '工具轮的正文进了转录且不含 JSON', JSON.stringify(ld[1] || {}).slice(0, 200));
  assert(ld[2] && ld[2].tool && ld[2].tool.name === 'read_txt', '正文之后紧跟工具卡片', JSON.stringify(ld[2] || {}).slice(0, 200));
  assert(ld.every((x) => !String(x.line || '').includes('"type"')), '整条转录里没有信封 JSON', JSON.stringify(ld.map((x) => String(x.line || '').slice(0, 40))));

  // 协作（非代拟）：两个 agent 跑完五阶段并交付
  const name2 = 'e2e-collab-' + Date.now();
  const c2 = await api('POST', '/api/sessions', {
    name: name2, mode: 'collab', task: '一起把事情做完',
    agents: [
      { name: '甲', transient: true, modules: ['summarizer'], model: 'm1' },
      { name: '乙', transient: true, modules: ['reviewer'], model: 'm1' },
    ],
  });
  assert(c2.status === 200, '建协作工作（2 个 agent）', c2.text.slice(0, 200));
  const begin = await api('POST', '/api/sessions/' + encodeURIComponent(name2) + '/begin', { text: 'yes,allow' });
  assert(begin.status === 200, '确认开始讨论', begin.text.slice(0, 200));
  // 子会话的**事件台**要有它自己的权威行与运行态（不能只落在盘上）：否则打开它的标签页，
  // 流式块永远等不到替换它的那一行——光标一直挂着、按钮永远停在「停止」（真机反馈过）。
  const childBus = await api('GET', '/api/events?sid=' + encodeURIComponent(name2 + '--甲') + '&since=0');
  const childJson = JSON.stringify((childBus.json && childBus.json.lines) || []);
  assert(childJson.includes('[甲:say]'), '子会话事件台带它自己的权威发言行', childJson.slice(0, 200));
  assert(childJson.includes('"working"'), '子会话事件台带运行态（在跑/收尾）', childJson.slice(0, 200));
  // 整理完停在**待审**：点「同意」才开工；开工后节点在子会话里跑，交付是异步产生的。
  await approvePlan(name2);
  // 命令回包不再携带事实：协作的转录与交付从**落盘重放**取（快照）。
  const ev2 = JSON.stringify(await waitForDelivery(name2));
  assert(ev2.includes('甲') && ev2.includes('乙'), '协作转录以 agent 名为说话人', ev2.slice(0, 240));
  assert(ev2.includes('delivery') || ev2.includes('交付'), '协作跑完并交付', ev2.slice(-240));
  assert(fs.existsSync(path.join(dir(name2), '甲')) && fs.existsSync(path.join(dir(name2), '乙')), '两个 agent 各自沙箱目录已建');

  // 代拟：核心优先复用（单兵）+ 组装（新助手），确认后名单写回 meta
  const name3 = 'e2e-delegate-' + Date.now();
  const c3 = await api('POST', '/api/sessions', { name: name3, mode: 'collab', agents: [], task: '调研一下再总结', delegate: true });
  assert(c3.status === 200, '建代拟工作（无名单）', c3.text.slice(0, 200));
  // 开场事实在**事件台**上（回包只给 head）：按会话过滤取它自己的流。
  const c3Bus = JSON.stringify(
    ((await api('GET', '/api/events?sid=' + encodeURIComponent(name3) + '&since=0')).json || {}).lines || [],
  );
  assert(c3Bus.includes('代拟'), '核心已代拟名单', c3Bus.slice(0, 240));
  const slate = await api('POST', '/api/sessions/' + encodeURIComponent(name3) + '/slate', { text: 'yes' });
  assert(slate.status === 200, '确认代拟名单', slate.text.slice(0, 200));
  const meta3 = fs.readFileSync(path.join(dir(name3), 'meta.yaml'), 'utf8');
  assert(meta3.includes('单兵') && meta3.includes('新助手'), '名单写回 meta.yaml（复用项 + 组装项）', meta3.split('\n').slice(0, 20).join(' | '));
  assert(/transient: false/.test(meta3), '复用项记为非常驻（transient: false）');
  assert(/transient: true/.test(meta3), '组装项记为临时（transient: true）');
  assert(fs.existsSync(path.join(dir(name3), '单兵')) && fs.existsSync(path.join(dir(name3), '新助手')), '确认名单后按 agent 名建出沙箱目录');
  const begun3 = await api('POST', '/api/sessions/' + encodeURIComponent(name3) + '/begin', { text: 'yes,allow' });
  assert(begun3.status === 200, '代拟名单后开始讨论', begun3.text.slice(0, 200));
  await approvePlan(name3);
  const ev3 = JSON.stringify(await waitForDelivery(name3));
  assert(ev3.includes('单兵'), '代拟出来的 agent 真的在发言', ev3.slice(0, 240));


  /** 走完审查关卡：整理完停在待审，点「同意」才开工（协作的必经一步）。 */
async function approvePlan(name) {
  const r = await api('POST', '/api/sessions/' + encodeURIComponent(name) + '/approve-plan', {});
  assert(r.status === 200, '审查关卡：点「同意」开工', r.text.slice(0, 200));
  return (r.json && r.json.events) || [];
}

/* ---------- 协作状态机的六条判据（L4） ----------
   * 现有用例只断言"跑完并交付"；这里把状态机的承诺逐条钉住：
   * agree 收敛 / leave 不可逆 / 轮次上限 / 返工闭环 / 撤回同意 / ask 中止。
   * 信封走向由 mock.js 按转录事实路由（夹具不改产品行为）。
   */
  const all = (sid) => lines(sid);
  const joined = (ls) => ls.map((x) => String(x.line || '')).join('\n');
  /** 全部事件（notice / delivery / ended 这些不是转录行，得从这里看）。 */
  async function eventsOf(sid) {
    const r = await api('GET', '/api/history/' + encodeURIComponent(sid));
    return (r.json && r.json.events) || [];
  }

  /** 等链驱动跑完：节点在**自己的子会话**里跑，父会话的交付是异步产生的。 */
  async function waitForDelivery(sid, ms = 90000) {
    const until = Date.now() + ms;
    for (;;) {
      const ev = await eventsOf(sid);
      if (JSON.stringify(ev).includes('delivery')) return ev;
      if (Date.now() > until) return ev;
      await new Promise((r) => setTimeout(r, 500));
    }
  }

  // ① agree 收敛：只有一个人同意时不该收敛（要出现下一轮），两人都同意才收敛。
  const nA = 'e2e-collab-noconv-' + Date.now();
  assert((await api('POST', '/api/sessions', {
    name: nA, mode: 'collab', task: '不收敛：先各说各的',
    agents: [
      { name: '甲', transient: true, modules: ['summarizer'], model: 'm1' },
      { name: '乙', transient: true, modules: ['reviewer'], model: 'm1' },
    ],
  })).status === 200, '建「不收敛」协作工作');
  assert((await api('POST', '/api/sessions/' + encodeURIComponent(nA) + '/begin', { text: 'yes,allow' })).status === 200, '「不收敛」开始讨论');
  const tA = joined(await all(nA));
  assert(/\[轮次 2\]/.test(tA), '有人同意、有人没同意 → 不收敛，进入下一轮', tA.slice(-300));
  assert(!tA.includes('delivery'), '未收敛时不交付', tA.slice(-200));

  // ② leave 不可逆：退场后不再被询问；留下的那个人同意即收敛（退场者不算收敛门槛）。
  const nB = 'e2e-collab-leave-' + Date.now();
  assert((await api('POST', '/api/sessions', {
    name: nB, mode: 'collab', task: '退场：甲先撤',
    agents: [
      { name: '甲', transient: true, modules: ['summarizer'], model: 'm1' },
      { name: '乙', transient: true, modules: ['reviewer'], model: 'm1' },
    ],
  })).status === 200, '建「退场」协作工作');
  assert((await api('POST', '/api/sessions/' + encodeURIComponent(nB) + '/begin', { text: 'yes,allow' })).status === 200, '「退场」开始讨论');
  await approvePlan(nB);
  const tB = joined(await all(nB));
  const evB = JSON.stringify(await waitForDelivery(nB));
  assert(tB.includes('甲:leave'), '甲发了 leave', tB.slice(-300));
  // leave 之后不该再出现甲的发言：取 leave 之后那一段来断言。
  const afterLeave = tB.slice(tB.indexOf('甲:leave'));
  assert(!/\[甲:(say|agree|ask)/.test(afterLeave), 'leave 不可逆：退场之后甲不再发言', afterLeave.slice(0, 300));
  assert(evB.includes('delivery'), '退场者不算门槛：剩下的人同意即收敛并交付', evB.slice(-240));

  // ③ 轮次上限：谁都不同意 → 触上限并交用户裁决。
  const nC = 'e2e-collab-cap-' + Date.now();
  assert((await api('POST', '/api/sessions', {
    name: nC, mode: 'collab', task: '上限：一直议下去',
    agents: [
      { name: '甲', transient: true, modules: ['summarizer'], model: 'm1' },
      { name: '乙', transient: true, modules: ['reviewer'], model: 'm1' },
    ],
  })).status === 200, '建「上限」协作工作');
  assert((await api('POST', '/api/sessions/' + encodeURIComponent(nC) + '/begin', { text: 'yes,allow' })).status === 200, '「上限」开始讨论');
  const evC = JSON.stringify(await eventsOf(nC));
  assert(evC.includes('讨论轮次超限'), '触上限要如实说明并交用户裁决', evC.slice(-300));

  // ④ 返工闭环：验收先 fail → 定向返工 → 重验 → 交付。
  const nD = 'e2e-collab-rework-' + Date.now();
  assert((await api('POST', '/api/sessions', {
    name: nD, mode: 'collab', task: '返工：第一次验收不过',
    agents: [
      { name: '甲', transient: true, modules: ['summarizer'], model: 'm1' },
      { name: '乙', transient: true, modules: ['reviewer'], model: 'm1' },
    ],
  })).status === 200, '建「返工」协作工作');
  assert((await api('POST', '/api/sessions/' + encodeURIComponent(nD) + '/begin', { text: 'yes,allow' })).status === 200, '「返工」开始讨论');
  await approvePlan(nD);
  const evD = JSON.stringify(await waitForDelivery(nD));
  assert(evD.includes('返工'), '验收 fail → 触发返工', evD.slice(-400));
  assert(evD.includes('delivery'), '返工后重验通过并交付', evD.slice(-240));

  // ⑤ 撤回同意：撤回行进转录，讨论重新打开并继续轮转。
  const nE = 'e2e-collab-withdraw-' + Date.now();
  assert((await api('POST', '/api/sessions', {
    name: nE, mode: 'collab', task: '不收敛：撤回后重议',
    agents: [
      { name: '甲', transient: true, modules: ['summarizer'], model: 'm1' },
      { name: '乙', transient: true, modules: ['reviewer'], model: 'm1' },
    ],
  })).status === 200, '建「撤回」协作工作');
  assert((await api('POST', '/api/sessions/' + encodeURIComponent(nE) + '/begin', { text: 'yes,allow' })).status === 200, '「撤回」开始讨论');
  const w = await api('POST', '/api/sessions/' + encodeURIComponent(nE) + '/withdraw', { agent: '乙' });
  assert(w.status === 200, '撤回乙的同意', w.text.slice(0, 200));
  const wBus = JSON.stringify(
    ((await api('GET', '/api/events?sid=' + encodeURIComponent(nE) + '&since=0')).json || {}).lines || [],
  );
  assert(wBus.includes('[用户:撤回] 乙'), '撤回如实进转录（事实在事件台）', wBus.slice(0, 240));

  // ⑥ ask 中止：agent 提问 → 轮转中止并呈给用户；回答后继续。
  const nF = 'e2e-collab-ask-' + Date.now();
  assert((await api('POST', '/api/sessions', {
    name: nF, mode: 'collab', task: '提问：需要用户决定',
    agents: [
      { name: '甲', transient: true, modules: ['summarizer'], model: 'm1' },
      { name: '乙', transient: true, modules: ['reviewer'], model: 'm1' },
    ],
  })).status === 200, '建「提问」协作工作');
  // 不给 allow：授权自裁（yes,allow）会让 ask 留档不中止——这正是 allow 的语义分界，所以这里要验"不授权"那一侧。
  const beginF = await api('POST', '/api/sessions/' + encodeURIComponent(nF) + '/begin', { text: 'yes' });
  assert(beginF.status === 200, '「提问」开始讨论（不授权自裁）', beginF.text.slice(0, 200));
  const pendF = await api('POST', '/api/sessions/' + encodeURIComponent(nF) + '/pending', {});
  assert(pendF.status === 200 && pendF.json && pendF.json.pending && pendF.json.pending.type === 'ask', 'ask 中止轮转并把问题呈给用户', pendF.text.slice(0, 300));
  const ansF = await api('POST', '/api/sessions/' + encodeURIComponent(nF) + '/answer', { text: '用第一个方案' });
  assert(ansF.status === 200, '回答 ask 后继续', ansF.text.slice(0, 200));
  const tF = joined(await all(nF));
  assert(tF.includes('用第一个方案'), '用户回答并入转录（进上下文）', tF.slice(-300));


  /* ---------- 真工具链路（L4）：三个真模块里的两个，真的在真进程里跑 ----------
   * harvest（python）扫夹具共享区 → corpus.jsonl；indexer（C++，CI 里现编）建索引 → 检索有命中。
   * 这一段证明的是"三种语言的模块在真进程里真的能用"，与上面的信封/状态机验收互补。
   * 工具用真进程，所以路径必须是**提示词里给出的真实绝对路径**（相对路径会被围栏拒绝）。
   */
  const nameR = 'e2e-realtools-' + Date.now();
  const workDir = path.join(ROOT, 'session', nameR, 'work');
  const cR = await api('POST', '/api/sessions', {
    name: nameR, mode: 'single',
    agents: [{ name: '资料手', transient: true, modules: ['harvest', 'indexer'], model: 'm1' }],
  });
  assert(cR.status === 200, '建「真工具」工作（harvest + indexer）', cR.text.slice(0, 200));
  // 投喂两份材料（走产品的上传能力面，落进本次工作的共享区）。
  assert((await api('POST', '/api/sessions/' + encodeURIComponent(nameR) + '/upload', {
    name: '甲.md', data_base64: Buffer.from('# 甲\n\n本地优先的检索：索引建好之后可以离线查。\n', 'utf8').toString('base64'),
  })).status === 200, '投喂甲.md');
  assert((await api('POST', '/api/sessions/' + encodeURIComponent(nameR) + '/upload', {
    name: '乙.md', data_base64: Buffer.from('# 乙\n\n本地优先的检索：离线查更稳。\n', 'utf8').toString('base64'),
  })).status === 200, '投喂乙.md');
  const corpusPath = path.join(workDir, 'corpus.jsonl');
  const indexPath = path.join(workDir, 'index.bin');
  // 直接经产品能力面驱动一次发言：假供应商会按提示词分支发出 harvest.scan 与 indexer.build/query。
  const sayR = await api('POST', '/api/sessions/' + encodeURIComponent(nameR) + '/say', { text: '真工具链路：先抽语料，再建索引并检索' });
  assert(sayR.status === 200, '「真工具」发言', sayR.text.slice(0, 200));
  const rowsR = (await all(nameR)).filter((x) => x.tool).map((x) => x.tool);
  // 失败时把工具原文打出来：真机围栏开着时（CI）只有它能说清是哪一层拦住了。
  for (const r of rowsR) {
    console.log('   工具 ' + (r.module || '') + '.' + r.name + (r.ok ? ' 成功' : ' 失败') + ' → '
      + String(r.output || '').replace(/\s+/g, ' ').slice(0, 1500));
  }
  // 失败时要能一眼分清"目录不在"与"在但没权限"——两者的修法完全不同。
  console.log('   共享区存在吗：' + fs.existsSync(workDir) + '（' + workDir + '）');
  assert(rowsR.some((r) => r.name === 'scan'), 'harvest.scan 真的跑了（python 工具进程）', JSON.stringify(rowsR.map((r) => r.name)));
  assert(fs.existsSync(corpusPath), 'corpus.jsonl 真的落地', corpusPath);
  assert(rowsR.some((r) => r.name === 'build'), 'indexer.build 真的跑了（C++ 工具进程）', JSON.stringify(rowsR.map((r) => r.name)));
  assert(fs.existsSync(indexPath), 'index.bin 真的落地', indexPath);
  assert(rowsR.some((r) => r.name === 'query' && r.ok), 'indexer.query 真的跑了并成功', JSON.stringify(rowsR.map((r) => [r.name, r.ok])));
  // 检索结果里要有命中（不是空结果兜底）：语料里两份材料都含「检索」。
  const q = rowsR.filter((r) => r.name === 'query').pop();
  assert(q && /检索/.test(String(q.output || '')), 'query 有命中（不是空结果兜底）', String((q && q.output) || '').slice(0, 300));

  // 原生工具调用（真实二进制 + 真 HTTP）：先实测这条通道支持（探测把 tools 写回 native），
  // 再跑一次"一次回复两个调用"，并核对**发给供应商的历史就是协议形状**。
  // 放在最后：把 m1 判成 native 之后，前面那些信封场景的预期就不再成立。
  const probe = await api('POST', '/api/models/m1/probe');
  assert(probe.status === 200 && probe.json && probe.json.outcome === 'supported', '探针判这条通道支持原生工具调用', probe.text.slice(0, 200));
  assert(probe.json && probe.json.mode === 'native', '探测结论写回登记处（mode=native）', JSON.stringify(probe.json));
  // 关流式：这条断言要看"最终那一条请求的消息形状"，非流式最好断言（流式分片的形状由 T2 契约测试盯）。
  assert((await api('POST', '/api/settings', { streaming: false })).status === 200, '关流式（本场景用）');
  const name1n = 'e2e-native-' + Date.now();
  const c1n = await api('POST', '/api/sessions', {
    name: name1n, mode: 'single',
    agents: [{ name: '多调手', transient: true, modules: ['summarizer'], model: 'm1' }],
  });
  assert(c1n.status === 200, '建「原生多调用」工作', c1n.text.slice(0, 200));
  const s1n = await api('POST', '/api/sessions/' + encodeURIComponent(name1n) + '/say', { text: '原生多调用' });
  assert(s1n.status === 200, '原生多调用发言', s1n.text.slice(0, 200));
  const ln = await lines(name1n);
  const toolsN = ln.filter((x) => x.tool);
  assert(toolsN.length === 2, '一次回复里的两个调用各成一条工具行', JSON.stringify(toolsN.map((x) => x.tool && x.tool.name)));
  assert(
    toolsN[0] && toolsN[0].tool.call_id === 'call_a' && toolsN[1] && toolsN[1].tool.call_id === 'call_b',
    '工具行记下供应商给的调用 id',
    JSON.stringify(toolsN.map((x) => x.tool && x.tool.call_id)),
  );
  assert(
    toolsN[0] && toolsN[1] && toolsN[0].tool.reply === toolsN[1].tool.reply,
    '同一次回复的工具行同号（回档按它原子截断）',
    JSON.stringify(toolsN.map((x) => x.tool && x.tool.reply)),
  );
  assert(toolsN.every((x) => x.tool.ok === true), '两个调用都真的执行了', JSON.stringify(toolsN.map((x) => x.tool && x.tool.ok)));
  assert(
    fs.existsSync(path.join(dir(name1n), '多调手', 'native-a.txt')) && fs.existsSync(path.join(dir(name1n), '多调手', 'native-b.txt')),
    '两个调用各写下一个文件（顺序内的两个都真的跑了）',
  );
  assert(ln.every((x) => !String(x.line || '').includes('"type"')), '原生通道的转录里没有信封 JSON', JSON.stringify(ln.map((x) => String(x.line || '').slice(0, 40))));
  const seen = await mockSeen();
  assert(
    seen && seen.assistantWithCalls === 1 && seen.toolMsgs === 2,
    '发回去的历史是协议形状（一条助手消息带 tool_calls + 两条 role=tool）',
    JSON.stringify(seen),
  );
  assert(seen && String(seen.toolIds) === 'call_a,call_b', '结果消息用 tool_call_id 各回应自己的调用', JSON.stringify(seen));
  assert((await api('POST', '/api/settings', { streaming: true })).status === 200, '恢复流式');

  console.log((failed ? 'E2E-FAILED failed=' + failed : 'E2E-DONE exitCode=0'));
  process.exitCode = failed ? 1 : 0;
})().catch((e) => { console.log('FAIL 驱动异常 :: ' + e.message); process.exitCode = 1; });
