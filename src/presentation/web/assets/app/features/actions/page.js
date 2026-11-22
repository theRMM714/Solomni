/* 目的：动作与工具页：一份目录（GET /api/actions）渲染成可跑的表单，调用一律走 POST /api/actions/{id}。
 * 管：按动作分组、按参数声明现造控件（缺省值 / 上下界 / 枚举 / 位置序号）、运行与回执。
 * 不管：目录里有什么（核心按 callers 授权）、参数校验（核心按同一份声明校验）、模块启停（模块与服务页）。
 * 联动：settings/hub.js 调用 actionsPage；跨功能调用一股经 app 晚绑定。
 */
let doc, api, ui, app;
let btn, textInput, field, emptyHint;
const $ = (sel) => doc.querySelector(sel);

/** 挂载：把本功能的入口登记到 app；跳功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ doc, api, ui, app } = deps);
  ({ btn, textInput, field, emptyHint } = ui);
  Object.assign(root, { actionsPage, actionRunner });
}

/** 分页：按 id 前缀分组（module.<id>.<tool> 归到「模块 <id>」，其余是核心动作）。 */
function actionsPage(c) {
  c.body.innerHTML = '';
  c.setMsg('加载中…');
  api('GET', '/api/actions').then((data) => {
    const acts = (data.actions || []).slice();
    c.setMsg('');
    if (!acts.length) {
      c.body.appendChild(emptyHint('（这个调用者此刻没有可跑的动作）'));
      return;
    }
    const groups = new Map();
    for (const a of acts) {
      const m = /^module\.([^.]+)\./.exec(a.id);
      const key = m ? '模块 ' + m[1] : '核心动作';
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(a);
    }
    for (const [title, list] of groups) {
      const sec = doc.createElement('div');
      sec.className = 'svc-sec';
      const h = doc.createElement('div');
      h.className = 'svc-title';
      h.textContent = title + '（' + list.length + '）';
      sec.appendChild(h);
      for (const a of list) sec.appendChild(actionRow(a));
      c.body.appendChild(sec);
    }
  }).catch((e) => c.setMsg('加载失败：' + e.message, true));
}

/** 一行动作：名字 + 说明 + 展开后的参数表单（表单只在展开时建，页面不会被十几份表单撑开）。 */
function actionRow(action) {
  const row = doc.createElement('div');
  row.className = 'svc-sec';
  const head = doc.createElement('div');
  head.className = 'svc-row';
  const name = doc.createElement('span');
  name.className = 'svc-name';
  name.textContent = action.id + (action.desc ? ' — ' + action.desc : '');
  head.appendChild(name);
  if (!action.available) {
    const why = doc.createElement('span');
    why.className = 'cfg-hint warn';
    why.textContent = action.reason || '此刻不可用';
    head.appendChild(why);
    row.appendChild(head);
    return row;
  }
  const run = btn('展开', 'btn');
  head.appendChild(run);
  row.appendChild(head);
  let body = null;
  run.onclick = () => {
    if (body) {
      body.classList.toggle('hidden');
      run.textContent = body.classList.contains('hidden') ? '展开' : '收起';
      return;
    }
    body = actionRunner(action);
    run.textContent = '收起';
    row.appendChild(body);
  };
  return row;
}

/** 参数控件：声明里有什么就渲染什么（枚举 → 下拉，布尔 → 勾选，数字 → 带上下界，其余 → 文本）。 */
function paramControl(p) {
  const label = p.name + (p.required ? '（必填）' : '') +
    (p.positional ? '（位置 ' + p.positional + '）' : '');
  const note = p.desc || '';
  if (p.values && p.values.length) {
    const opts = p.values.map((v) => ({ value: v, label: v }));
    const sel = ui.selectInput(opts, p.default != null ? String(p.default) : undefined);
    return { wrap: field(label, sel, note ? { lead: note } : undefined), read: () => sel.value, empty: () => false };
  }
  if (p.ty === 'boolean') {
    const box = ui.checkbox(label, p.default === true, note);
    return { wrap: box.wrap, read: () => (box.box.checked ? true : undefined), empty: () => !box.box.checked };
  }
  if (p.ty === 'integer' || p.ty === 'number') {
    const n = ui.numberInput(label, p.default != null ? p.default : '', p.min != null ? p.min : '', p.max != null ? p.max : '');
    return { wrap: n.wrap, read: () => (n.input.value === '' ? undefined : Number(n.input.value)), empty: () => n.input.value === '' };
  }
  if (p.ty === 'array' || p.ty === 'object') {
    const t = ui.areaInput('JSON');
    if (p.default != null) t.value = JSON.stringify(p.default);
    return { wrap: field(label, t, note ? { lead: note } : undefined), read: () => (t.value.trim() === '' ? undefined : JSON.parse(t.value)), empty: () => t.value.trim() === '' };
  }
  const i = textInput('');
  if (p.default != null) i.value = String(p.default);
  return { wrap: field(label, i, note ? { lead: note } : undefined), read: () => (i.value === '' ? undefined : i.value), empty: () => i.value === '' };
}

/** 一条动作的运行块：参数按声明现渲染，运行走 /api/actions/{id}，回执打在下面。 */
function actionRunner(action) {
  const box = doc.createElement('div');
  box.className = 'svc-run';
  const controls = [];
  for (const p of action.params || []) {
    const ctl = paramControl(p);
    controls.push({ p, ctl });
    box.appendChild(ctl.wrap);
  }
  const out = doc.createElement('pre');
  out.className = 'svc-out';
  const run = btn('运行', 'btn btn-primary');
  run.onclick = async () => {
    const args = {};
    try {
      for (const { p, ctl } of controls) {
        const v = ctl.read();
        if (v === undefined || ctl.empty()) {
          if (p.required) { out.textContent = '缺少必填参数：' + p.name; return; }
          continue;
        }
        args[p.name] = v;
      }
    } catch (e) {
      out.textContent = '参数不是合法 JSON：' + e.message;
      return;
    }
    out.textContent = '运行中…';
    try {
      const r = await api('POST', actionUrl(action.id), args);
      out.textContent = JSON.stringify(r, null, 2);
    } catch (e) {
      out.textContent = '失败：' + e.message;
    }
  };
  box.appendChild(run);
  box.appendChild(out);
  return box;
}

/** 动作入口只有一个：/api/actions/{id}（声明、授权、执行与审计都在核心那一处）。 */
function actionUrl(id) { return '/api/actions/' + id; }
