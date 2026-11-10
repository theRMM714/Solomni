/* 目的：设置中心的分页框架（左侧分页、右侧单页、底栏统一保存）与各入口。
 * 管：分页切换、共享设置表单的挂载、底栏保存；agent 管理页的列表骨架。
 * 不管：各页内容（pages.js / runtime.js / storage.js）、弹层栈（ui/modal.js）。
 * 联动：侧栏与向导调用 openSettingsHub；本文件按页调用各页构建函数。
 */
let g, doc, state, ui, app;
let btn, emptyHint;
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ g, doc, state, ui, app } = deps);
  ({ btn, emptyHint } = ui);
  Object.assign(root, { openSettingsHub, agentsPage });
}
function openSettingsHub(page) {
  ui.openModal('设置', (c) => {
    const hub = doc.createElement('div');
    hub.className = 'settings-hub';
    const nav = doc.createElement('nav');
    nav.className = 'settings-nav';
    nav.setAttribute('aria-label', '设置分页');
    const panes = doc.createElement('div');
    panes.className = 'settings-panes';
    const foot = doc.createElement('div');
    foot.className = 'settings-foot';
    const form = app.settingsForm();
    const save = btn('保存', 'btn btn-primary');
    save.onclick = () => app.saveSettings(c, form, save);
    const hint = doc.createElement('span');
    hint.className = 'settings-foot-hint';
    hint.textContent = '设置页共用同一份保存（全量写入）';
    foot.appendChild(hint);
    foot.appendChild(save);
    const main = doc.createElement('div');
    main.className = 'settings-main';
    main.appendChild(panes);
    main.appendChild(foot);
    hub.appendChild(nav);
    hub.appendChild(main);

    const built = new Map();
    // 分页也拿得到弹窗级的登记处订阅与写动作入口（否则分页刷新只能各写一套）。
    const ctxOf = (pane) => ({
      body: pane,
      setMsg: c.setMsg,
      onRegistryChange: c.onRegistryChange,
      mutate: c.mutate,
    });
    const builders = {
      generate: (pane, ctx) => app.buildGeneratePage(ctx, form),
      sandbox: (pane, ctx) => app.buildSandboxPage(ctx, form),
      permissions: (pane, ctx) => app.buildPermissionsPage(ctx, form),
      resources: (pane, ctx) => app.buildResourcesPage(ctx, form),
      storage: (pane, ctx) => app.buildStoragePage(ctx, form),
      providers: (pane, ctx) => app.providersPage(ctx),
      models: (pane, ctx) => {
        // 核心默认块放在模型页顶部：登记 / 删除 / 设核心后由 modelsPage 自己的订阅统一刷新。
        const core = doc.createElement('div');
        core.className = 'settings-core';
        pane.appendChild(core);
        app.modelsPage(ctx, core);
      },
      agents: (pane, ctx) => agentsPage(ctx),
      modules: (pane, ctx) => app.modulesPage(ctx),
    };
    const settingsIds = new Set(['generate', 'sandbox', 'permissions', 'resources', 'storage']);

    let group = '';
    for (const p of app.SETTINGS_PAGES) {
      if (p.group !== group) {
        group = p.group;
        const g = doc.createElement('div');
        g.className = 'settings-nav-group';
        g.textContent = group;
        nav.appendChild(g);
      }
      const b = doc.createElement('button');
      b.type = 'button';
      b.className = 'settings-nav-item';
      b.dataset.id = p.id;
      b.appendChild(ui.icon(p.icon));
      const label = doc.createElement('span');
      label.textContent = p.label;
      b.appendChild(label);
      b.onclick = () => show(p.id);
      nav.appendChild(b);
    }

    function show(id) {
      if (!builders[id]) id = 'generate';
      for (const b of nav.children) {
        if (!b.dataset || !b.dataset.id) continue;
        b.className = 'settings-nav-item' + (b.dataset.id === id ? ' active' : '');
      }
      let pane = built.get(id);
      if (!pane) {
        pane = doc.createElement('section');
        pane.className = 'settings-page';
        pane.dataset.id = id;
        panes.appendChild(pane);
        builders[id](pane, ctxOf(pane));
        built.set(id, pane);
      }
      for (const ch of panes.children) {
        ch.className = 'settings-page' + (ch.dataset.id === id ? '' : ' hidden');
      }
      foot.className = settingsIds.has(id) ? 'settings-foot' : 'settings-foot hidden';
      c.setMsg('');
    }

    c.body.appendChild(hub);
    show(page || 'generate');
  }, true);
}

/* ---------- 设置⑤：agent 管理（列表 / 新建 / 编辑 / 删除） ---------- */
function agentsPage(c) {
    const listWrap = doc.createElement('div'); listWrap.className = 'reg-list';
    const toolbar = doc.createElement('div'); toolbar.className = 'wf-inline';
    const add = btn('+ 新建 agent', 'btn btn-primary');
    add.onclick = () => app.openAgentEditor({ onSaved: (name) => c.setMsg('已保存 agent：' + name) });
    toolbar.appendChild(add);
    function renderList() {
      listWrap.innerHTML = '';
      if (!state.agents.length) { listWrap.appendChild(emptyHint('（还没有 agent：点「+ 新建 agent」）')); return; }
      for (const a of state.agents) {
        listWrap.appendChild(app.agentCard(a, {
          onEdit: () => app.openAgentEditor({
            agent: a,
            onSaved: (name) => c.setMsg('已保存 agent：' + name),
          }),
          onRemove: async () => {
            try {
              await c.mutate('remove_agent', { name: a.name });
              c.setMsg('已删除 ' + a.name);
            } catch (e) { c.setMsg(e.message, true); }
          },
        }));
      }
    }
    c.onRegistryChange(renderList);
    renderList();
    c.body.appendChild(toolbar);
    c.body.appendChild(listWrap);
}