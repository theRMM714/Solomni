/* 目的：agent 的展示与编辑：模块选择弹层、agent 卡片、临时 / 登记处 agent 的编辑入口。
 * 管：agent 表单与模块勾选交互，结果交给调用方的回调。
 * 不管：agent 的落盘（走调用方传入的回调 / app 动作）、设置中心分页框架（settings/hub.js）。
 * 联动：settings/pages.js 与向导都用这里的表单与卡片。
 */
let g, doc, state, ui, app;
let btn, textInput, field, areaInput, selectInput, syncSelect, emptyHint;
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ g, doc, state, ui, app } = deps);
  ({ btn, textInput, field, areaInput, selectInput, syncSelect, emptyHint } = ui);
  Object.assign(root, { moduleById, moduleMatches, moduleChips, openModulePicker, cloneAgent, agentPreview, agentForm, openAgentEditor, openTempAgentSheet, openRegistryAgentPicker, agentCard });
}
function moduleById(id) {
  return state.modules.find((m) => m.id === id);
}

/** 只匹配模块名字（id）与简介（brief）；runtimes / services / system 等字段刻意不查。 */
function moduleMatches(m, q) {
  const terms = String(q || '').toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return true;
  const id = String(m.id || '').toLowerCase();
  const brief = String(m.brief || '').toLowerCase();
  return terms.every((t) => id.indexOf(t) >= 0 || brief.indexOf(t) >= 0);
}

/** 已选模块的 chips；返回 { node, render }。disabled = 只读（登记处 agent 的模块由登记处决定）。 */
function moduleChips(opts) {
  const o = opts || {};
  const box = doc.createElement('div');
  box.className = 'mod-chips' + (o.disabled ? ' readonly' : '');
  function current() { return (o.selected ? o.selected() : []).slice(); }
  function commit(ids) {
    if (o.onChange) o.onChange(ids.slice());
    render();
  }
  function render() {
    box.innerHTML = '';
    const ids = current();
    if (!ids.length) {
      const empty = doc.createElement('span');
      empty.className = 'mod-chips-empty';
      empty.textContent = o.emptyText || '（还没有模块：零模块 agent 只用内置文件工具）';
      box.appendChild(empty);
    }
    for (const id of ids) {
      const m = moduleById(id);
      const chip = doc.createElement('span');
      chip.className = 'mod-chip' + (m ? '' : ' is-orphan');
      const label = doc.createElement('span');
      label.className = 'mod-chip-label';
      label.textContent = id + (m ? '' : '（已不在清单）');
      if (m && m.brief) chip.title = m.brief;
      chip.appendChild(label);
      if (!o.disabled) {
        const x = doc.createElement('button');
        x.type = 'button'; x.className = 'mod-chip-x'; x.textContent = '×';
        x.title = '移除 ' + id;
        x.onclick = () => commit(ids.filter((xid) => xid !== id));
        chip.appendChild(x);
      }
      box.appendChild(chip);
    }
    if (!o.disabled) {
      const add = doc.createElement('button');
      add.type = 'button'; add.className = 'mod-add';
      add.textContent = '+ 添加模块';
      add.onclick = () => openModulePicker({ selected: ids, title: '添加模块', onDone: commit });
      box.appendChild(add);
    }
  }
  render();
  return { node: box, render };
}

/** 模块选择弹层：上方「已加入」始终可见且不参与搜索；下方「可选」可搜索；点卡片切换。 */
function openModulePicker(opts) {
  const o = opts || {};
  const picked = new Set(o.selected || []);
  app.openModalLayer(o.title || '添加模块', (c) => {
    const search = textInput('关键字（只匹配名字与简介）');
    const addedTitle = doc.createElement('div'); addedTitle.className = 'mod-pick-title';
    const availTitle = doc.createElement('div'); availTitle.className = 'mod-pick-title';
    const addedList = doc.createElement('div'); addedList.className = 'mod-grid';
    const availList = doc.createElement('div'); availList.className = 'mod-grid';
    function ordered() {
      const out = state.modules.map((m) => m.id).filter((id) => picked.has(id));
      for (const id of picked) if (out.indexOf(id) < 0) out.push(id);
      return out;
    }
    function moduleOf(id) {
      return state.modules.find((m) => m.id === id) || { id, brief: '（已不在清单）', missing: true };
    }
    function tile(m, isAdded) {
      const b = doc.createElement('button');
      b.type = 'button';
      b.className = 'mod-tile' + (isAdded ? ' is-added' : '') + (m.missing ? ' is-orphan' : '');
      b.setAttribute('aria-pressed', isAdded ? 'true' : 'false');
      b.title = (isAdded ? '点击移除：' : '点击加入：') + m.id + (m.brief ? '——' + m.brief : '');
      const head = doc.createElement('span'); head.className = 'mod-tile-head';
      const id = doc.createElement('span'); id.className = 'mod-tile-id'; id.textContent = m.id;
      const mark = doc.createElement('span'); mark.className = 'mod-tile-mark';
      mark.textContent = isAdded ? '✓' : '+';
      head.appendChild(id); head.appendChild(mark);
      const brief = doc.createElement('span'); brief.className = 'mod-tile-brief'; brief.textContent = m.brief || '';
      b.appendChild(head); b.appendChild(brief);
      b.onclick = () => {
        if (isAdded) picked.delete(m.id); else picked.add(m.id);
        render();
      };
      return b;
    }
    function render() {
      const chosen = ordered();
      const chosenSet = new Set(chosen);
      addedTitle.textContent = '已加入（' + chosen.length + '）';
      addedList.innerHTML = '';
      if (!chosen.length) addedList.appendChild(emptyHint('（还没有加入模块）'));
      else for (const id of chosen) addedList.appendChild(tile(moduleOf(id), true));

      const shown = state.modules.filter((m) => !chosenSet.has(m.id) && moduleMatches(m, search.value));
      availTitle.textContent = '可选（' + shown.length + '）';
      availList.innerHTML = '';
      if (!shown.length) {
        const q = String(search.value || '').trim();
        availList.appendChild(emptyHint(!state.modules.length
          ? '（modules/ 下没有模块）'
          : (!q && chosenSet.size >= state.modules.length
            ? '（模块都已加入）'
            : '（没有匹配的可选模块）')));
      } else {
        for (const m of shown) availList.appendChild(tile(m, false));
      }
    }
    search.addEventListener('input', render);
    const addedSec = doc.createElement('div'); addedSec.className = 'mod-pick-section';
    addedSec.appendChild(addedTitle); addedSec.appendChild(addedList);
    const availSec = doc.createElement('div'); availSec.className = 'mod-pick-section';
    availSec.appendChild(availTitle); availSec.appendChild(availList);
    c.body.appendChild(addedSec);
    c.body.appendChild(field('搜索可选模块', search));
    c.body.appendChild(availSec);
    const actions = doc.createElement('div'); actions.className = 'wf-inline';
    const cancel = btn('取消', 'btn'); cancel.onclick = c.close;
    const ok = btn('确定', 'btn btn-primary');
    ok.onclick = () => { const ids = ordered(); c.close(); if (o.onDone) o.onDone(ids); };
    actions.appendChild(cancel); actions.appendChild(ok);
    c.body.appendChild(actions);
    render();
    if (typeof search.focus === 'function') search.focus();
  }, true);
}

/* ---------- agent 编辑：设置页 / 向导临时 agent 共用 ---------- */
function cloneAgent(a) {
  const x = a || {};
  return {
    name: x.name || '',
    modules: (x.modules || []).slice(),
    model: x.model || '',
    chara: x.chara || '',
    note: x.note || '',
    transient: !!x.transient,
    why: x.why || null,
    reuse: !!x.reuse,
  };
}

/** 人设 / 说明的一行预览（卡片与选择器用）。 */
function agentPreview(a) {
  if (a && String(a.chara || '').trim()) return '人设：' + app.oneLine(a.chara, 96);
  if (a && String(a.note || '').trim()) return '说明：' + app.oneLine(a.note, 96);
  return '';
}

/** agent 表单：名字 / 人设 / 模块 chips / 模型 / 说明。锁定项用于“登记处 agent 只改本会话模型”。 */
function agentForm(opts) {
  const o = opts || {};
  const draft = o.draft;
  const box = doc.createElement('div'); box.className = 'form-col';
  let nameTouched = !!(draft.name || '').trim();
  const nameIn = textInput('agent 名字（会当沙箱目录名）');
  nameIn.value = draft.name || '';
  nameIn.disabled = !!o.lockedName;
  nameIn.addEventListener('input', () => { nameTouched = true; draft.name = nameIn.value; });
  const charaIn = areaInput('人设 / 身份（给模型看：角色、语气、职责边界；别写本次任务）');
  charaIn.value = draft.chara || '';
  charaIn.disabled = !!o.lockedChara;
  charaIn.addEventListener('input', () => { draft.chara = charaIn.value; });
  const chips = moduleChips({
    selected: () => draft.modules,
    disabled: !!o.lockedModules,
    onChange: (ids) => {
      draft.modules = ids;
      if (!o.lockedName && !nameTouched && !String(draft.name || '').trim() && ids.length) {
        draft.name = ids[0];
        nameIn.value = ids[0];
      }
    },
  });
  const modelSel = selectInput(app.modelOptions(draft.model || '', true), draft.model || '');
  modelSel.addEventListener('change', () => { draft.model = modelSel.value; });
  const noteIn = textInput('说明（给人看的一句话，可选）');
  noteIn.value = draft.note || '';
  noteIn.addEventListener('input', () => { draft.note = noteIn.value; });
  const syncModels = () => syncSelect(modelSel, app.modelOptions(draft.model || '', true), draft.model || '');
  box.appendChild(field(o.lockedName ? '名字（只读）' : '名字', nameIn));
  box.appendChild(field('人设 / 身份', charaIn));
  box.appendChild(field('模块（能力）', chips.node));
  box.appendChild(field('默认模型（空 = 核心默认）', modelSel));
  box.appendChild(field('说明', noteIn));
  return { node: box, draft, syncModels, nameIn, charaIn, chipNode: chips.node };
}

/** 设置页的 agent 新建 / 编辑（写登记处）。 */
function openAgentEditor(opts) {
  const o = opts || {};
  const editing = o.agent ? o.agent.name : null;
  const draft = cloneAgent(o.agent);
  app.openModalLayer(editing ? '编辑 agent' : '新建 agent', (c) => {
    const form = agentForm({ draft });
    c.body.appendChild(form.node);
    const save = btn('保存 agent', 'btn btn-primary btn-block');
    save.onclick = async () => {
      const name = draft.name.trim();
      if (!name) { c.setMsg('agent 名字不能为空', true); return; }
      if (state.agents.some((x) => x.name === name && x.name !== editing)) {
        c.setMsg('已有同名 agent：' + name + '（换一个名字，或先删除旧的）', true);
        return;
      }
      try {
        if (editing && editing !== name) {
          const ok = await app.confirmBox('agent 改名', '改名 = 先删旧「' + editing + '」再建新「' + name + '」。继续？', '继续');
          if (!ok) return;
          await c.mutate('remove_agent', { name: editing });
        }
        await c.mutate('upsert_agent', {
          name,
          modules: draft.modules.slice(),
          model: draft.model || undefined,
          note: draft.note.trim(),
          chara: draft.chara.trim(),
        });
        if (o.onSaved) o.onSaved(name);
        c.close();
      } catch (e) { c.setMsg(e.message, true); }
    };
    const cancel = btn('取消', 'btn btn-block'); cancel.onclick = c.close;
    c.body.appendChild(save); c.body.appendChild(cancel);
    c.onRegistryChange(form.syncModels);
  }, true);
}

/** 向导里的临时 agent 新建 / 编辑（不写登记处；可顺手保存为 agent）。 */
function openTempAgentSheet(opts) {
  const o = opts || {};
  const editing = !!o.agent;
  const draft = cloneAgent(o.agent);
  if (!editing) draft.transient = true;
  const title = o.title || (editing ? '编辑临时 agent' : '新建临时 agent');
  app.openModalLayer(title, (c) => {
    const form = agentForm({
      draft,
      lockedName: !!o.lockedName,
      lockedModules: !!o.lockedModules,
      lockedChara: !!o.lockedChara,
    });
    c.body.appendChild(form.node);
    const add = btn(editing ? '保存' : '加入本次（临时）', 'btn btn-primary btn-block');
    add.onclick = () => {
      if (!draft.name.trim()) { c.setMsg('agent 名字不能为空', true); return; }
      if (o.onAdd) o.onAdd(cloneAgent(draft));
      c.close();
    };
    c.body.appendChild(add);
    if (o.allowSaveAsAgent !== false) {
      const keep = btn('保存为 agent 并加入', 'btn btn-block');
      keep.onclick = async () => {
        const name = draft.name.trim();
        if (!name) { c.setMsg('agent 名字不能为空', true); return; }
        try {
          if (state.agents.some((x) => x.name === name)) {
            const ok = await app.confirmBox('同名 agent', '登记处已有「' + name + '」，保存会更新它。继续？', '更新');
            if (!ok) return;
          }
          await c.mutate('upsert_agent', {
            name,
            modules: draft.modules.slice(),
            model: draft.model || undefined,
            note: draft.note.trim(),
            chara: draft.chara.trim(),
          });
          if (o.onSaveAsAgent) {
            const saved = cloneAgent(draft);
            saved.transient = false;
            saved.reuse = true;
            o.onSaveAsAgent(saved);
          }
          c.close();
        } catch (e) { c.setMsg(e.message, true); }
      };
      c.body.appendChild(keep);
    }
    const cancel = btn('取消', 'btn btn-block'); cancel.onclick = c.close;
    c.body.appendChild(cancel);
    c.onRegistryChange(form.syncModels);
  }, true);
}

/** 从登记处挑一个 agent（协作加入 / 单 agent 选择）。 */
function openRegistryAgentPicker(opts) {
  const o = opts || {};
  const taken = o.taken || [];
  app.openModalLayer(o.title || (o.single ? '选择 agent' : '从登记处加入'), (c) => {
    const search = textInput('搜索 agent（名字 / 说明 / 人设）');
    const list = doc.createElement('div'); list.className = 'reg-list';
    function matches(a, q) {
      const terms = String(q || '').toLowerCase().split(/\s+/).filter(Boolean);
      if (!terms.length) return true;
      const hay = [a.name, a.note, a.chara].map((x) => String(x || '').toLowerCase()).join(' ');
      return terms.every((t) => hay.indexOf(t) >= 0);
    }
    function render() {
      list.innerHTML = '';
      const shown = state.agents.filter((a) => matches(a, search.value));
      if (!shown.length) { list.appendChild(emptyHint('（没有匹配的登记处 agent）')); return; }
      for (const a of shown) {
        const row = doc.createElement('div'); row.className = 'reg-item';
        const main = doc.createElement('div'); main.className = 'reg-main';
        const nm = doc.createElement('div'); nm.className = 'reg-id';
        nm.textContent = a.name + (taken.indexOf(a.name) >= 0 ? '（已在本次名单）' : '');
        const sub = doc.createElement('div'); sub.className = 'reg-sub';
        sub.textContent = (a.modules || []).join(' + ') + ' · 模型 ' + (a.model || '（核心默认）');
        const line = doc.createElement('div'); line.className = 'reg-note';
        line.textContent = agentPreview(a);
        main.appendChild(nm); main.appendChild(sub); if (line.textContent) main.appendChild(line);
        const acts = doc.createElement('div'); acts.className = 'reg-acts';
        const pick = btn(o.single ? '选择' : '加入', 'btn');
        pick.disabled = taken.indexOf(a.name) >= 0;
        pick.onclick = () => {
          if (pick.disabled) return;
          if (o.onPick) o.onPick(cloneAgent({
            name: a.name,
            modules: (a.modules || []).slice(),
            model: a.model || '',
            chara: a.chara || '',
            note: a.note || '',
            transient: false,
            reuse: true,
          }));
          c.close();
        };
        acts.appendChild(pick);
        row.appendChild(main); row.appendChild(acts);
        list.appendChild(row);
      }
    }
    search.addEventListener('input', render);
    c.body.appendChild(field('搜索', search));
    c.body.appendChild(list);
    const cancel = btn('关闭', 'btn btn-block'); cancel.onclick = c.close;
    c.body.appendChild(cancel);
    render();
  }, true);
}

/** 向导 / 设置列表里的一个 agent 卡片。 */
function agentCard(a, handlers) {
  const h = handlers || {};
  const row = doc.createElement('div'); row.className = 'agent-item';
  const main = doc.createElement('div'); main.className = 'reg-main';
  const head = doc.createElement('div'); head.className = 'agent-card-head';
  const nm = doc.createElement('div'); nm.className = 'agent-name'; nm.textContent = a.name;
  head.appendChild(nm);
  const badge = doc.createElement('span');
  badge.className = 'agent-badge' + (a.transient ? ' is-transient' : '');
  badge.textContent = a.transient ? '临时' : '登记处';
  head.appendChild(badge);
  main.appendChild(head);
  const preview = agentPreview(a);
  if (preview) { const p = doc.createElement('div'); p.className = 'agent-preview'; p.textContent = preview; main.appendChild(p); }
  const mods = doc.createElement('div'); mods.className = 'agent-mods';
  mods.textContent = (a.modules && a.modules.length ? a.modules.join(' + ') : '（无模块）') + ' · 模型 ' + (a.model || '（核心默认）');
  main.appendChild(mods);
  if (a.why) { const why = doc.createElement('div'); why.className = 'wf-why'; why.textContent = a.why; main.appendChild(why); }
  const acts = doc.createElement('div'); acts.className = 'wf-inline';
  if (a.transient && h.onSave) {
    const keep = btn('存为 agent', 'link-btn');
    keep.onclick = () => h.onSave(a);
    acts.appendChild(keep);
  }
  if (h.onEdit) {
    const edit = btn('编辑', 'link-btn');
    edit.onclick = () => h.onEdit(a);
    acts.appendChild(edit);
  }
  if (h.onRemove) {
    const del = btn('移除', 'link-btn danger');
    del.onclick = () => h.onRemove(a);
    acts.appendChild(del);
  }
  row.appendChild(main); row.appendChild(acts);
  return row;
}