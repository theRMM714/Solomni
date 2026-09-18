// e2e 驱动（L4，由 orchestrator.js 调起）：隔离根内跑，绝不动真实 .home/ 与 session/。
// 覆盖：agent 登记处 → 推荐复用 → 单 agent（1 个 agent 带多模块）+ 内置 write 落私沙箱
//       → 协作（非代拟）跑完交付 → 代拟（复用+组装）确认后名单写回 meta 并建出沙箱。
const BASE = process.env.E2E_BASE || 'http://127.0.0.1:3099';
const fs = require('fs');
const path = require('path');
// 夹具根：本目录下的 root/（隔离根：prompts.yaml、.home、modules 都在里面）。
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
  assert((await api('POST', '/api/providers', { id: 'mock', base_url: 'http://127.0.0.1:8397/v1', api_key: 'k' })).status === 200, '登记供应商');
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
  const ev2 = JSON.stringify((begin.json && begin.json.events) || []);
  assert(ev2.includes('甲') && ev2.includes('乙'), '协作转录以 agent 名为说话人', ev2.slice(0, 240));
  assert(ev2.includes('delivery') || ev2.includes('交付'), '协作跑完并交付', ev2.slice(-240));
  assert(fs.existsSync(path.join(dir(name2), '甲')) && fs.existsSync(path.join(dir(name2), '乙')), '两个 agent 各自沙箱目录已建');

  // 代拟：核心优先复用（单兵）+ 组装（新助手），确认后名单写回 meta
  const name3 = 'e2e-delegate-' + Date.now();
  const c3 = await api('POST', '/api/sessions', { name: name3, mode: 'collab', agents: [], task: '调研一下再总结', delegate: true });
  assert(c3.status === 200, '建代拟工作（无名单）', c3.text.slice(0, 200));
  assert(JSON.stringify((c3.json && c3.json.events) || []).includes('代拟'), '核心已代拟名单', JSON.stringify((c3.json && c3.json.events) || []).slice(0, 240));
  const slate = await api('POST', '/api/sessions/' + encodeURIComponent(name3) + '/slate', { text: 'yes' });
  assert(slate.status === 200, '确认代拟名单', slate.text.slice(0, 200));
  const meta3 = fs.readFileSync(path.join(dir(name3), 'meta.yaml'), 'utf8');
  assert(meta3.includes('单兵') && meta3.includes('新助手'), '名单写回 meta.yaml（复用项 + 组装项）', meta3.split('\n').slice(0, 20).join(' | '));
  assert(/transient: false/.test(meta3), '复用项记为非常驻（transient: false）');
  assert(/transient: true/.test(meta3), '组装项记为临时（transient: true）');
  assert(fs.existsSync(path.join(dir(name3), '单兵')) && fs.existsSync(path.join(dir(name3), '新助手')), '确认名单后按 agent 名建出沙箱目录');
  const begun3 = await api('POST', '/api/sessions/' + encodeURIComponent(name3) + '/begin', { text: 'yes,allow' });
  assert(begun3.status === 200, '代拟名单后开始讨论', begun3.text.slice(0, 200));
  const ev3 = JSON.stringify((begun3.json && begun3.json.events) || []);
  assert(ev3.includes('单兵'), '代拟出来的 agent 真的在发言', ev3.slice(0, 240));

  console.log((failed ? 'E2E-FAILED failed=' + failed : 'E2E-DONE exitCode=0'));
  process.exitCode = failed ? 1 : 0;
})().catch((e) => { console.log('FAIL 驱动异常 :: ' + e.message); process.exitCode = 1; });
