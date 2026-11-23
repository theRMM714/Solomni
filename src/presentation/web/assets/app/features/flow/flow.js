/* 目的：流程视图——任务链的节点一次列在状态栏上方，按状态上色，可展开收起。
 * 管：flowApply（事件增量）/ renderFlow（画 #flow，含展开收起与进入子会话）。
 * 不管：不造转录行、不改运行态、不落盘；事实都来自事件，节点跳转交给 app.openPath。
 * 联动：事件来自 app/features/transcript/events.js；每帧由 transcript/page.js 的 renderAll 调用。 */

let doc, ui, app;

/// 选择器简写：模块内的 DOM 查找一律走注入的 doc。
const $ = (sel) => doc.querySelector(sel);

/// 节点状态词表：与后端 NodeStatus 的序列化值一一对应。
const STATUS = ['pending', 'running', 'done', 'failed'];

/// 每个状态的字形与徽标；返工不改状态（活已经交回来了），只在徽标上带一句。
const MARK = { pending: 'clock', running: 'dot', done: 'check', failed: 'alert' };
const BADGE = { pending: '等待', running: '进行中', done: '完成', failed: '返工中' };

/// 会话的流程快照：懒建，字段只由事件写（open 是用户偏好，事件不动它）。
function flowOf(s) {
  if (!s.flow) s.flow = { nodes: [], open: true, review: null, delivery: null };
  return s.flow;
}

/// 按节点 id 找（链已经 renumber_by_stage，id 在会话内唯一）。
function nodeOf(flow, id) {
  for (const n of flow.nodes) if (n.id === id) return n;
  return null;
}

/* 事件增量：方案审查给整条链，开工 / 回报更新单个节点，验收与交付只记结论。
 * 没有链快照时不猜节点（缺 plan_review 的历史本来就没有标题可显示）。 */
function flowApply(s, ev) {
  if (!s) return;
  switch (ev.type) {
    case 'plan_review': {
      const nodes = ((ev.chain && ev.chain.nodes) || []).map((n) => ({
        id: n.id || '',
        title: n.title || '',
        assignee: n.assignee || '',
        status: STATUS.indexOf(n.status) >= 0 ? n.status : 'pending',
        sid: n.sub_session || '',
        report: n.report || '',
        rework: 0,
      }));
      if (!nodes.length) return;
      const flow = flowOf(s);
      // 重新整理换了一份链：节点整体替换，验收结论作废；展开状态跟着用户走，不重新弹开。
      flow.nodes = nodes;
      flow.review = null;
      flow.delivery = null;
      return;
    }
    case 'node_started': {
      const node = nodeOf(flowOf(s), ev.node);
      if (!node) return;
      node.status = 'running';
      if (ev.sid) node.sid = ev.sid;
      if (ev.assignee) node.assignee = ev.assignee;
      return;
    }
    case 'report': {
      const node = nodeOf(flowOf(s), ev.id);
      if (!node) return;
      node.status = 'done';
      node.report = ev.text || '';
      node.rework = ev.rework || 0;
      return;
    }
    case 'review': {
      const items = ev.items || [];
      const pass = items.filter((i) => String(i.status).toLowerCase() === 'pass').length;
      flowOf(s).review = { pass, total: items.length };
      return;
    }
    case 'delivery':
      flowOf(s).delivery = { ok: ev.ok === true };
      return;
    default:
  }
}

/// 小文本节点：流程视图里除字形外都是纯文本，不给 Markdown（这里是进度，不是正文）。
function span(cls, text) {
  const el = doc.createElement('span');
  el.className = cls;
  el.textContent = text;
  return el;
}

/// 头部总状态：交付结论优先，其次返工 / 在跑 / 全部完成。
function headStatus(flow, done, running, failed) {
  if (flow.delivery) return flow.delivery.ok && !failed ? 'ok' : 'bad';
  if (failed) return 'bad';
  if (running) return 'run';
  if (done === flow.nodes.length) return 'ok';
  return '';
}

/// 头部附注：交付与验收结论一句话说清（不占用节点行）。
function noteOf(flow) {
  if (flow.delivery) return flow.delivery.ok ? '总验收通过，已交付' : '总验收未全过，待裁决';
  if (flow.review) return '验收 ' + flow.review.pass + '/' + flow.review.total;
  return '';
}

/// 单个节点行：字形 + 标题 + id + 执行人 + 徽标；有子会话就能点进去。
function nodeRow(node) {
  const row = doc.createElement('li');
  row.className = 'flow-node ' + node.status;
  const mark = doc.createElement('span');
  mark.className = 'flow-node-mark';
  mark.appendChild(ui.icon(MARK[node.status] || 'clock'));
  row.appendChild(mark);
  row.appendChild(span('flow-node-title', node.title || '（未命名节点）'));
  row.appendChild(span('flow-node-id', node.id));
  if (node.assignee) row.appendChild(span('flow-node-who', node.assignee));
  const badge = BADGE[node.status] + (node.status === 'done' && node.rework ? ' · 返工 ' + node.rework : '');
  row.appendChild(span('flow-node-badge', badge));
  row.title = node.report ? String(node.report).split('\n')[0] : '';
  if (node.sid) {
    row.className += ' link';
    row.title = '进入 ' + (node.title || node.id) + ' 的子会话';
    row.onclick = () => app.openPath(node.sid);
  }
  return row;
}

/* 画 #flow：没有链就整块收起（单机 / 代理会话不占高度）。
 * 展开状态存在会话视图上：切会话回来仍是用户上次的选法。 */
function renderFlow(s) {
  const box = $('#flow');
  if (!box) return;
  box.innerHTML = '';
  const flow = s && s.flow;
  if (!flow || !flow.nodes.length) { box.className = 'flow hidden'; return; }
  const open = flow.open !== false;
  box.className = 'flow' + (open ? '' : ' closed');
  const done = flow.nodes.filter((n) => n.status === 'done').length;
  const running = flow.nodes.some((n) => n.status === 'running');
  const failed = flow.nodes.some((n) => n.status === 'failed');

  const head = doc.createElement('button');
  head.type = 'button';
  head.className = 'flow-head';
  head.setAttribute('aria-expanded', String(open));
  head.title = open ? '收起节点列表' : '展开节点列表';
  const mark = doc.createElement('span');
  mark.className = 'flow-mark ' + headStatus(flow, done, running, failed);
  head.appendChild(mark);
  head.appendChild(span('flow-title', '流程'));
  head.appendChild(span('flow-count', '完成 ' + done + '/' + flow.nodes.length));
  const note = noteOf(flow);
  if (note) head.appendChild(span('flow-note', note));
  const chev = doc.createElement('span');
  chev.className = 'flow-chev';
  chev.appendChild(ui.icon('chevron'));
  head.appendChild(chev);
  head.onclick = () => { flow.open = !open; renderFlow(s); };
  box.appendChild(head);
  if (!open) return;
  const list = doc.createElement('ul');
  list.className = 'flow-list';
  for (const node of flow.nodes) list.appendChild(nodeRow(node));
  box.appendChild(list);
}

export { flowApply, renderFlow };

/// 挂载：装配依赖并把入口交给组装点（事件与整页重画都走 app 上的名字）。
export function mount(root, deps) {
  ({ doc, ui, app } = deps);
  Object.assign(root, { flowApply, renderFlow });
}
