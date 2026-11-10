/* 目的：模块与服务页：模块清单、启停、密钥、服务进程与工具调用实测。
 * 管：该页的清单渲染与动作（经 /api/actions 的 available 决定可做什么）。
 * 不管：设置中心分页框架（settings/hub.js）、弹层栈（ui/modal.js）。
 * 联动：settings/hub.js 调用 modulesPage；本文件经 app 调用 refreshState / pollLoop / openWizard。
 */
let doc, state, api, ui, app;
let btn, textInput, field, emptyHint;
const $ = (sel) => doc.querySelector(sel);
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ doc, state, api, ui, app } = deps);
  ({ btn, textInput, field, emptyHint } = ui);
  Object.assign(root, { modulesPage, renderModulesPanel, useRow, openUseCard, fire, panelSection, actionRunner });
}
function modulesPage(c) {
  const load = async () => {
    c.body.innerHTML = '';
    c.setMsg('加载中…');
    let mods = [];
    let services = [];
    let actions = [];
    try {
      const st = await api('GET', '/api/state');
      mods = st.modules || [];
      services = st.services || [];
      const ac = await api('GET', '/api/actions');
      // 隐秘字段的设置面另有入口：值不在这一块面板里回显。
      actions = (ac.actions || []).filter((a) => a.id !== 'set_secret' && a.id !== 'clear_secret');
    } catch (e) {
      c.setMsg('加载失败：' + e.message, true);
      return;
    }
    c.setMsg('');
    renderModulesPanel(c, mods, services, actions, load);
  };
  load();
}

/** 纯渲染（数据注入）：模块 → 服务（开关 + 运行态）→ 功能（「使用」按钮）。 */
function renderModulesPanel(ctx, mods, services, actions, reload) {
  if (!mods.length) {
    ctx.body.appendChild(emptyHint('（modules/ 下没有模块）'));
    return;
  }
  for (const m of mods) {
    const sec = panelSection(m.id + (m.brief ? ' · ' + m.brief : ''));
    // 模块开关：停用会连带停掉它的常驻服务（核心侧级联）。
    const head = doc.createElement('div');
    head.className = 'svc-row';
    const state = doc.createElement('span');
    state.className = 'svc-state';
    state.textContent = m.enabled ? '整体启用' : '整体停用';
    head.appendChild(state);
    const toggle = btn(m.enabled ? '停用模块' : '启用模块', 'btn');
    toggle.onclick = () => fire(ctx, 'set_module_enabled', { module: m.id, enabled: !m.enabled }, reload);
    head.appendChild(toggle);
    sec.appendChild(head);

    // 服务：模块声明的常驻服务（声明在 modules[].services，运行态在 services）。
    sec.appendChild(ui.div('cfg-sec-title', '服务'));
    const declared = m.services || [];
    if (!declared.length) {
      sec.appendChild(emptyHint('（没有声明常驻服务）'));
    } else {
      for (const s of declared) {
        const rt = services.find((x) => x.module === m.id && x.name === s.name) || {};
        const row = doc.createElement('div');
        row.className = 'svc-row';
        const name = doc.createElement('span');
        name.className = 'svc-name';
        name.textContent = s.name + (rt.adapter ? ' · ' + rt.adapter : '') +
          ' · ' + (rt.state || (s.enabled ? '未启动' : '已禁用'));
        row.appendChild(name);
        const sw = btn(s.enabled ? '停用服务' : '启用服务', 'btn');
        sw.onclick = () => fire(ctx, 'control_resident',
          { module: m.id, name: s.name, action: s.enabled ? 'disable' : 'enable' }, reload);
        row.appendChild(sw);
        sec.appendChild(row);
      }
    }

    // 功能：本模块的工具与服务操作（每个一行；参数与运行在子卡片里）。
    sec.appendChild(ui.div('cfg-sec-title', '功能'));
    const acts = actions.filter((a) => a.id.indexOf('module.' + m.id + '.') === 0);
    if (!acts.length) {
      sec.appendChild(emptyHint('（只能经 AI 使用：没有可直跑的 tools / services）'));
    } else {
      for (const a of acts) sec.appendChild(useRow(a));
    }
    ctx.body.appendChild(sec);
  }
}

/** 功能一行：只显示名字与「使用」；参数与运行在子卡片里，列表不再平铺表单。 */
function useRow(action) {
  const row = doc.createElement('div');
  row.className = 'svc-row';
  const name = doc.createElement('span');
  name.className = 'svc-name';
  const short = action.id.replace(/^module\.[^.]+\./, '');
  name.textContent = short + (action.desc ? ' — ' + action.desc : '');
  row.appendChild(name);
  const use = btn('使用', 'btn');
  use.onclick = () => openUseCard(action);
  row.appendChild(use);
  return row;
}

/** 子卡片：一个功能的参数与运行（回执打在卡片里）。 */
function openUseCard(action) {
  const ctx = ui.openModalLayer(action.id, () => {});
  if (action.desc) ctx.setMsg(action.desc);
  ctx.body.appendChild(actionRunner(action));
}

/** 跑一条动作后再重绘面板：失败如实显示，成功按新状态重绘。 */
async function fire(ctx, id, args, reload) {
  try {
    await api('POST', app.actionUrl(id), args);
  } catch (e) {
    ctx.setMsg('操作失败：' + e.message, true);
    return;
  }
  await reload();
}

function panelSection(title) {
  const sec = doc.createElement('div');
  sec.className = 'svc-sec';
  const h = doc.createElement('div');
  h.className = 'svc-title';
  h.textContent = title;
  sec.appendChild(h);
  return sec;
}

/** 一条动作的运行块：参数按声明现渲染，运行走 /api/actions/{id}，回执打在下面。 */
function actionRunner(action) {
  const box = doc.createElement('div');
  box.className = 'svc-run';
  const name = doc.createElement('div');
  name.className = 'svc-name';
  name.textContent = action.id + (action.desc ? ' — ' + action.desc : '');
  box.appendChild(name);
  const inputs = {};
  for (const p of action.params || []) {
    const i = textInput(p.name + (p.required ? '（必填）' : ''));
    inputs[p.name] = { p, i };
    box.appendChild(field(p.name, i));
  }
  const out = doc.createElement('pre');
  out.className = 'svc-out';
  const run = btn('运行', 'btn');
  run.onclick = async () => {
    const args = {};
    for (const k of Object.keys(inputs)) {
      const { p, i } = inputs[k];
      if (i.value === '' && !p.required) continue;
      if (p.ty === 'integer') args[k] = Number(i.value);
      else if (p.ty === 'boolean') args[k] = i.value === 'true';
      else args[k] = i.value;
    }
    out.textContent = '运行中…';
    try {
      const r = await api('POST', app.actionUrl(action.id), args);
      out.textContent = JSON.stringify(r, null, 2);
    } catch (e) {
      out.textContent = '失败：' + e.message;
    }
  };
  box.appendChild(run);
  box.appendChild(out);
  return box;
}
