/* 目的：模块与服务页：模块清单、启停、服务进程（工具调用不在这里）。
 * 管：该页的清单渲染与启停动作（经 /api/actions/{id}）。
 * 不管：设置中心分页框架（settings/hub.js）、弹层栈（ui/modal.js）、工具调用（features/actions/page.js）。
 * 联动：settings/hub.js 调用 modulesPage；本文件经 app 调用 actionUrl。
 */
let doc, state, api, ui, app;
let btn, emptyHint;
const $ = (sel) => doc.querySelector(sel);
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ doc, state, api, ui, app } = deps);
  ({ btn, emptyHint } = ui);
  Object.assign(root, { modulesPage, renderModulesPanel, fire, panelSection });
}
function modulesPage(c) {
  const load = async () => {
    c.body.innerHTML = '';
    c.setMsg('加载中…');
    let mods = [];
    let services = [];
    try {
      const st = await api('GET', '/api/state');
      mods = st.modules || [];
      services = st.services || [];
    } catch (e) {
      c.setMsg('加载失败：' + e.message, true);
      return;
    }
    c.setMsg('');
    renderModulesPanel(c, mods, services, load);
  };
  load();
}

/** 纯渲染（数据注入）：模块（状态 + 说明）→ 服务（开关 + 运行态）→ 用它的入口。 */
function renderModulesPanel(ctx, mods, services, reload) {
  if (!mods.length) {
    ctx.body.appendChild(emptyHint('（modules/ 下没有模块）'));
    return;
  }
  for (const m of mods) {
    const sec = panelSection(m.id + (m.version ? ' ' + m.version : '') + (m.brief ? ' · ' + m.brief : ''));
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

    // 调用是另一页的事：这里只说去哪儿，不把工具表单塞进启停设置里。
    sec.appendChild(ui.div('cfg-sec-title', '调用'));
    sec.appendChild(emptyHint('工具与服务的调用在「登记 → 动作与工具」：参数按声明现渲染，跑完的回执就在那里。'));
    ctx.body.appendChild(sec);
  }
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

