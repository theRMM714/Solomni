/* 目的：设置中心的分页内容——供应商、模型、思考等级、设置表单与多选一弹层。
 * 管：这些页的控件搭建、读取与提交（经 app.* 调登记处与设置动作）。
 * 不管：弹层栈本身、设置中心的分页框架（见同目录 hub.js）、保存后的全局重绘。
 * 联动：hub.js 组装分页并调用本文件；本文件经 app 调用 openModalLayer / refreshState；数据来自 state 投影。
 */
let doc, state, api, ui, app;
let icon, btn, busyButton, textInput, field, numberInput, areaInput, selectInput, syncSelect, checkbox, emptyHint;
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ doc, state, api, ui, app } = deps);
  ({ icon, btn, busyButton, textInput, field, numberInput, areaInput, selectInput, syncSelect, checkbox, emptyHint } = ui);
  Object.assign(root, { providersPage, thinkingTagsEditor, thinkingMapEditor, modelsPage, coreModelBlock, settingsForm, parseSettingLines, collectSettings, saveSettings, SETTINGS_PAGES, choiceModal });
}
/* ---------- 设置①：供应商登记（录入 / 修改 / 删除） ---------- */
function providersPage(c) {
    const listWrap = doc.createElement('div'); listWrap.className = 'reg-list';
    const idIn = textInput('id');
    const urlIn = textInput('base_url');
    const keyIn = textInput('api_key（保存后不回显）', 'password');
    const save = btn('登记 / 更新', 'btn btn-primary btn-block');

    function rebuild() {
      listWrap.innerHTML = '';
      if (!state.providers.length) { listWrap.appendChild(emptyHint('（暂无供应商）')); return; }
      for (const p of state.providers) {
        const row = doc.createElement('div'); row.className = 'reg-item';
        const main = doc.createElement('div'); main.className = 'reg-main';
        const id = doc.createElement('div'); id.className = 'reg-id'; id.textContent = p.id;
        const sub = doc.createElement('div'); sub.className = 'reg-sub'; sub.textContent = p.base_url;
        main.appendChild(id); main.appendChild(sub);
        const acts = doc.createElement('div'); acts.className = 'reg-acts';
        const edit = btn('编辑', 'link-btn');
        edit.onclick = () => {
          idIn.value = p.id; urlIn.value = p.base_url; keyIn.value = '';
          c.setMsg('编辑 ' + p.id + '：api_key 需重新填写');
        };
        const del = btn('删除', 'link-btn danger');
        del.onclick = async () => {
          try {
            await c.mutate('remove_provider', { id: p.id });
            rebuild();
            c.setMsg('已删除 ' + p.id);
          } catch (e) { c.setMsg(e.message, true); }
        };
        acts.appendChild(edit); acts.appendChild(del);
        row.appendChild(main); row.appendChild(acts);
        listWrap.appendChild(row);
      }
    }

    save.onclick = async () => {
      const id = idIn.value.trim(), url = urlIn.value.trim(), key = keyIn.value;
      if (!id || !url || !key) { c.setMsg('id / base_url / api_key 均不能为空', true); return; }
      try {
        await c.mutate('upsert_provider', { id, base_url: url, api_key: key });
        keyIn.value = '';
        rebuild();
        c.setMsg('已保存：' + id);
      } catch (e) { c.setMsg(e.message, true); }
    };

    // 供应商变了：模型页的供应商下拉 / 发现区靠这条通知同步（它们可能已经建好并缓存）。
    c.onRegistryChange(rebuild);
    rebuild();
    c.body.appendChild(listWrap);
    const form = doc.createElement('div'); form.className = 'form-col';
    form.appendChild(field('id', idIn));
    form.appendChild(field('base_url', urlIn));
    form.appendChild(field('api_key', keyIn));
    form.appendChild(save);
    c.body.appendChild(form);
}

/* ---------- 思考等级标签 / 模型档位映射 ---------- */
/** 思考等级标签编辑器：标签就是档位，顺序即界面顺序，界面显示与模型映射都用这一串。
 *  约束：非空、不重复、无首尾空白、无控制字符（与后端校验一致）；不限制字符集。 */
function thinkingTagsEditor(levels) {
  const node = doc.createElement('div');
  node.className = 'tag-editor';
  const chips = doc.createElement('div');
  chips.className = 'mod-chips';
  const addRow = doc.createElement('div');
  addRow.className = 'wf-inline';
  const input = textInput('新档位（如 high、高、xhigh）');
  input.className = 'field-input tag-input';
  const add = btn('+ 添加档位', 'mod-add');
  const hint = ui.hint('标签就是档位；模型登记按它映射。可以任意写（如 high、高、xhigh），不能为空或重复。');
  const err = ui.hint('', 'err');
  const control = /[\u0000-\u001f\u007f-\u009f]/;
  addRow.appendChild(input); addRow.appendChild(add);
  node.appendChild(chips); node.appendChild(hint); node.appendChild(err);

  const ids = [];
  for (const l of levels || []) {
    const id = String((l && l.id) || '').trim();
    if (!id || ids.indexOf(id) >= 0) continue;
    ids.push(id);
  }

  function showError(text) { err.textContent = text || ''; }
  function render() {
    chips.innerHTML = '';
    if (!ids.length) {
      const empty = doc.createElement('span');
      empty.className = 'mod-chips-empty';
      empty.textContent = '（还没有档位：至少加一个）';
      chips.appendChild(empty);
      chips.appendChild(addRow);
      return;
    }
    for (const id of ids) {
      const chip = doc.createElement('span');
      chip.className = 'mod-chip';
      const label = doc.createElement('span');
      label.className = 'mod-chip-label';
      label.textContent = id;
      chip.title = id;
      const x = btn('×', 'mod-chip-x');
      x.title = '移除 ' + id;
      x.onclick = () => {
        const i = ids.indexOf(id);
        if (i >= 0) ids.splice(i, 1);
        showError('');
        render();
      };
      chip.appendChild(label); chip.appendChild(x);
      chips.appendChild(chip);
    }
    chips.appendChild(addRow);
  }
  function addLevel() {
    const raw = String(input.value || '').trim();
    if (!raw) { showError('思考档位不能为空'); return; }
    if (ids.indexOf(raw) >= 0) { showError('思考档位已存在：' + raw); return; }
    if (control.test(raw)) { showError('思考档位不能包含控制字符'); return; }
    ids.push(raw);
    input.value = '';
    showError('');
    render();
    if (typeof input.focus === 'function') input.focus();
  }
  add.onclick = addLevel;
  input.addEventListener('keydown', (e) => {
    if (e && e.key === 'Enter') addLevel();
  });
  function validate() {
    if (!ids.length) {
      showError('思考等级至少要有一个档位');
      return '思考等级至少要有一个档位';
    }
    for (const id of ids) {
      if (control.test(id)) {
        const msg = '思考档位不能包含控制字符：' + id;
        showError(msg);
        return msg;
      }
    }
    showError('');
    return '';
  }
  function list() {
    return ids.map((id) => ({ id }));
  }
  render();
  return { node, list, validate };
}

/** 模型思考映射：一行 = 左选档位、右填请求体片段；已映射的档位在其它行不可再选。
 *  约束：每行值非空；{...} 必须是 JSON 对象；默认档只能从已映射的档里选（后端同一条）。 */
function thinkingMapEditor() {
  const mapping = doc.createElement('div');
  mapping.className = 'think-editor';
  const rowsBox = doc.createElement('div');
  rowsBox.className = 'think-rows';
  const empty = ui.hint('（还没有映射：这个模型不发思考参数）');
  const addRow = doc.createElement('div');
  addRow.className = 'wf-inline';
  const add = btn('+ 添加映射', 'mod-add');
  addRow.appendChild(add);
  const hint = ui.hint('字符串 = {"reasoning_effort": 值} 简写；{...} = 原样合并进请求体（保留键会被拒）。每行的值都不能为空。');
  const defSel = selectInput([{ value: '', label: '（不指定：随供应商默认）' }], '');
  const defaultField = field('思考默认档（只能选已映射的档）', defSel);
  const defHint = ui.hint('不指定 = 随供应商默认；会话没选档时用它。');
  mapping.appendChild(rowsBox);
  mapping.appendChild(empty);
  mapping.appendChild(addRow);
  mapping.appendChild(hint);

  const rows = [];
  const catalog = () => app.thinkingLevels();
  function findLevel(id) {
    return catalog().find((l) => l.id === id) || null;
  }
  function optionFor(id) {
    const l = findLevel(id);
    return { value: id, label: l ? id : id + '（已不在目录）' };
  }
  function usedByOthers(row) {
    const used = new Set();
    for (const r of rows) if (r !== row && r.level) used.add(r.level);
    return used;
  }
  function optionsFor(row) {
    const used = usedByOthers(row);
    const opts = [];
    if (!row.level) opts.push({ value: '', label: '（选择档位）' });
    for (const l of catalog()) {
      if (used.has(l.id) && l.id !== row.level) continue;
      if (!opts.some((o) => o.value === l.id)) opts.push({ value: l.id, label: l.id });
    }
    if (row.level && !findLevel(row.level) && !opts.some((o) => o.value === row.level)) {
      opts.push({ value: row.level, label: row.level + '（已不在目录）' });
    }
    if (!opts.length) opts.push({ value: '', label: '（没有可用档位）' });
    return opts;
  }
  function createRow(level, value) {
    const row = { level: level || '', value: value || '', sel: null, input: null, node: null };
    const node = doc.createElement('div');
    node.className = 'think-row';
    const sel = selectInput([], row.level);
    const eq = doc.createElement('span');
    eq.className = 'think-eq';
    eq.textContent = '=';
    const input = textInput('参数（如 high 或 {"thinking":{"type":"disabled"}}）');
    input.className = 'field-input think-value';
    input.value = row.value;
    input.addEventListener('input', () => { row.value = input.value; });
    const del = btn('×', 'link-btn danger');
    del.title = '移除这一行';
    del.onclick = () => removeRow(row);
    sel.onchange = () => { row.level = sel.value; syncAll(); };
    node.appendChild(sel); node.appendChild(eq); node.appendChild(input); node.appendChild(del);
    row.node = node; row.sel = sel; row.input = input;
    rowsBox.appendChild(node);
    rows.push(row);
    return row;
  }
  function removeRow(row) {
    const i = rows.indexOf(row);
    if (i >= 0) rows.splice(i, 1);
    if (row.node && row.node.parentNode) row.node.parentNode.removeChild(row.node);
    syncAll();
  }
  function syncRow(row) {
    const v = syncSelect(row.sel, optionsFor(row), row.level);
    if (v !== row.level) row.level = v;
  }
  function defaultOptions() {
    const opts = [{ value: '', label: '（不指定：随供应商默认）' }];
    for (const row of rows) {
      if (!row.level || opts.some((o) => o.value === row.level)) continue;
      opts.push(optionFor(row.level));
    }
    return opts;
  }
  function syncDefault(want) {
    return syncSelect(defSel, defaultOptions(), want === undefined ? defSel.value : want);
  }
  function syncRows() {
    for (const row of rows) syncRow(row);
    empty.style.display = rows.length ? 'none' : '';
    const cat = catalog();
    const unused = cat.filter((l) => !rows.some((r) => r.level === l.id));
    if (!cat.length) {
      add.disabled = true;
      add.textContent = '+ 添加映射';
      add.title = '先在设置页添加思考等级';
    } else if (!unused.length) {
      add.disabled = true;
      add.textContent = '（档位都已映射）';
      add.title = '所有思考等级都已映射';
    } else {
      add.disabled = false;
      add.textContent = '+ 添加映射';
      add.title = '再添加一行映射';
    }
  }
  function syncAll() {
    syncRows();
    syncDefault(defSel.value);
  }
  function addMapping() {
    const used = new Set(rows.map((r) => r.level).filter(Boolean));
    const next = catalog().find((l) => !used.has(l.id));
    if (!next) return;
    createRow(next.id, '');
    syncAll();
  }
  add.onclick = addMapping;

  function setSpec(spec) {
    rowsBox.innerHTML = '';
    rows.length = 0;
    defSel.value = '';
    const map = (spec && spec.levels) || {};
    const ids = catalog().map((l) => l.id);
    const known = ids.filter((id) => Object.prototype.hasOwnProperty.call(map, id));
    const unknown = Object.keys(map).filter((id) => ids.indexOf(id) < 0).sort();
    for (const id of known.concat(unknown)) {
      const raw = map[id];
      createRow(id, typeof raw === 'string' ? raw : JSON.stringify(raw));
    }
    syncRows();
    syncDefault((spec && spec.default) || '');
  }
  function refresh() { syncAll(); }
  function read() {
    const levels = {};
    const seen = new Set();
    for (const row of rows) {
      const level = String(row.level || '').trim();
      const raw = String(row.value || '').trim();
      if (!level) return { error: '有一行还没选思考档位' };
      if (seen.has(level)) return { error: '思考档位 ' + level + ' 重复' };
      seen.add(level);
      if (!raw) return { error: '思考档位 ' + level + ' 的映射不能为空' };
      if (raw.charAt(0) === '{') {
        try {
          const obj = JSON.parse(raw);
          if (!obj || typeof obj !== 'object' || Array.isArray(obj)) {
            return { error: '思考档位 ' + level + ' 的映射必须是 JSON 对象' };
          }
          levels[level] = obj;
        } catch (e) {
          return { error: '思考档位 ' + level + ' 的 JSON 不合法：' + e.message };
        }
      } else {
        levels[level] = raw;
      }
    }
    const want = String(defSel.value || '').trim();
    if (want && !seen.has(want)) return { error: '思考默认档 ' + want + ' 没有映射' };
    return { spec: { default: want || null, levels } };
  }

  setSpec({ default: null, levels: {} });
  return { mapping, defaultField, defaultHint: defHint, setSpec, refresh, read };
}

/* ---------- 设置②：模型登记（录入 / 修改 / 删除 + 供应商模型发现） ---------- */
function modelsPage(c, coreBody) {
    const listWrap = doc.createElement('div'); listWrap.className = 'reg-list';
    const discWrap = doc.createElement('div'); discWrap.className = 'reg-list';
    const coreWrap = coreBody || doc.createElement('div');
    const idIn = textInput('id');
    const nameIn = textInput('展示名');
    const apiIn = textInput('api_model（真正发给供应商的串）');
    const providerSel = selectInput(state.providers.map((p) => ({ value: p.id, label: p.id })), null);
    const noteIn = textInput('note（能力说明）');
    // 上下文窗口：0 / 留空 = 用户不填；自动压缩按 128K 兜底（见后端 DEFAULT_CONTEXT_TOKENS）。
    const ctxIn = numberInput('上下文窗口（tokens，0 或留空 = 不填；自动压缩按 128K 兜底）', 0, 0, 2000000);
    // 工具调用形态：原生 = 保存前实测（通道基线 + tools + 回放形状）；手写信封 = 用户显式选择，不测试。
    const toolsSel = selectInput([
      { value: 'native', label: '原生工具调用（保存前实测）' },
      { value: 'envelope', label: '手写信封（不测试）' },
    ], 'native');
    // 思考强度：一行 = 左选档位、右填映射值；默认档只能从已映射的档里选。
    const thinkEditor = thinkingMapEditor();
    const save = btn('登记 / 更新', 'btn btn-primary btn-block');

    function syncProviderSelect() {
      syncSelect(providerSel, state.providers.map((p) => ({ value: p.id, label: p.id })), providerSel.value);
    }

    function refreshCore() {
      coreWrap.innerHTML = '';
      coreModelBlock({ body: coreWrap, setMsg: c.setMsg, mutate: c.mutate }, false);
    }

    function rebuild() {
      listWrap.innerHTML = '';
      if (!state.models.length) { listWrap.appendChild(emptyHint('（暂无模型）')); return; }
      for (const m of state.models) {
        const row = doc.createElement('div'); row.className = 'reg-item';
        const main = doc.createElement('div'); main.className = 'reg-main';
        const id = doc.createElement('div'); id.className = 'reg-id'; id.textContent = m.id;
        if (m.is_core) {
          const badge = doc.createElement('span'); badge.className = 'reg-core'; badge.textContent = '核心';
          id.appendChild(badge);
        }
        const sub = doc.createElement('div'); sub.className = 'reg-sub';
        const tLevels = Object.keys((m.thinking && m.thinking.levels) || {});
        const tDesc = tLevels.length
          ? ('默认 ' + ((m.thinking && m.thinking.default) || '随供应商默认') + '（' + tLevels.join(' / ') + '）')
          : '未登记（不发思考参数）';
        sub.textContent = m.name + ' · ' + m.api_model + ' · 供应商 ' + m.provider +
          ' · 工具调用 ' + (m.tools === 'native' ? '原生' : '手写信封') +
          ' · 窗口 ' + (m.context ? m.context + ' tokens' : '未填（压缩按 128K 兜底）') +
          ' · 思考 ' + tDesc;
        const note = doc.createElement('div'); note.className = 'reg-note'; note.textContent = m.note || '';
        main.appendChild(id); main.appendChild(sub); main.appendChild(note);
        const acts = doc.createElement('div'); acts.className = 'reg-acts';
        if (!m.is_core) {
          // 登记完就有这条：不必再去上面的核心默认块里找同一个模型。
          const setCore = btn('设为核心', 'link-btn');
          setCore.onclick = async () => {
            try {
              await c.mutate('set_core_model', { id: m.id });
              c.setMsg('核心 AI 默认模型：' + m.id);
            } catch (e) { c.setMsg(e.message, true); }
          };
          acts.appendChild(setCore);
        }
        const edit = btn('编辑', 'link-btn');
        edit.onclick = () => {
          idIn.value = m.id; nameIn.value = m.name; apiIn.value = m.api_model;
          providerSel.value = m.provider; noteIn.value = m.note || '';
          ctxIn.input.value = m.context || 0;
          toolsSel.value = m.tools || 'native';
          thinkEditor.setSpec(m.thinking || { default: null, levels: {} });
          c.setMsg('编辑 ' + m.id);
        };
        const del = btn('删除', 'link-btn danger');
        del.onclick = async () => {
          try {
            await c.mutate('remove_model', { id: m.id });
            c.setMsg('已删除 ' + m.id);
          } catch (e) { c.setMsg(e.message, true); }
        };
        // 实测这条通道支不支持原生工具调用（要真实网络）：结论由后端如实回报，只写确定的结论。
        const probe = btn('测工具调用', 'link-btn');
        probe.onclick = async () => {
          probe.disabled = true;
          c.setMsg('正在实测 ' + m.id + ' 的工具调用支持（要发两条最小请求）…');
          try {
            const r = await c.mutate('probe_model_tools', { id: m.id });
            const what = r.outcome === 'supported' ? '支持原生工具调用'
              : r.outcome === 'unsupported' ? '**不支持**原生工具调用（供应商拒了带 tools 的请求）'
              : '无法判定（供应商接受了 tools，但这次没有发起调用）';
            c.setMsg(what + '｜' + (r.detail || '') + '｜登记处现在是「' +
              (r.mode === 'native' ? '原生' : '手写信封') + '」，下一次生成起生效');
          } catch (e) { c.setMsg(e.message, true); }
        };
        // 实测「工具调用历史怎么发回去」的几种写法（要真实网络；只报事实、不改登记处）。
        const probeReplay = btn('测回放形状', 'link-btn');
        probeReplay.onclick = async () => {
          probeReplay.disabled = true;
          c.setMsg('正在实测 ' + m.id + ' 的回放形状（几种写法各发一次）…');
          try {
            const r = await api('POST', app.actionUrl('probe_replay_shape'), { id: m.id });
            const lines = (r.shapes || []).map((s) => {
              const verdict = !s.accepted ? '被拒' : (s.understood ? '收+读懂' : '收未懂');
              return verdict + '  ' + s.name + '  ' + (s.detail || '');
            });
            c.setMsg('回放形状（' + m.id + '）：\n' + lines.join('\n'));
          } catch (e) { c.setMsg(e.message, true); }
        };
        acts.appendChild(probeReplay); acts.appendChild(probe); acts.appendChild(edit); acts.appendChild(del);
        row.appendChild(main); row.appendChild(acts);
        listWrap.appendChild(row);
      }
    }

    function rebuildDiscover() {
      discWrap.innerHTML = '';
      if (!state.providers.length) { discWrap.appendChild(emptyHint('（暂无供应商，先登记供应商）')); return; }
      for (const p of state.providers) {
        const row = doc.createElement('div'); row.className = 'disc-row';
        const label = doc.createElement('span'); label.className = 'disc-id'; label.textContent = p.id;
        const go = btn('获取模型列表', 'link-btn');
        const chips = doc.createElement('div'); chips.className = 'chips';
        go.onclick = async () => {
          chips.innerHTML = '';
          c.setMsg('正在拉取 ' + p.id + ' 的模型列表…');
          try {
            const r = await api('POST', app.actionUrl('discover_models'), { id: p.id });
            const list = r.models || [];
            if (!list.length) { chips.appendChild(emptyHint('（未返回模型）')); }
            for (const name of list) {
              const chip = btn(name, 'chip');
              chip.onclick = () => {
                apiIn.value = name; providerSel.value = p.id;
                c.setMsg('已填入 api_model：' + name + '（供应商 ' + p.id + '）');
              };
              chips.appendChild(chip);
            }
            c.setMsg('获取到 ' + list.length + ' 个模型，点选填入 api_model');
          } catch (e) { c.setMsg(e.message, true); }
        };
        row.appendChild(label); row.appendChild(go); row.appendChild(chips);
        discWrap.appendChild(row);
      }
    }

    save.onclick = async () => {
      const think = thinkEditor.read();
      if (think.error) { c.setMsg(think.error, true); return; }
      const body = {
        id: idIn.value.trim(), name: nameIn.value.trim(), api_model: apiIn.value.trim(),
        provider: providerSel.value, note: noteIn.value.trim(),
        context: Number(ctxIn.input.value) || 0,
        tools: toolsSel.value,
        thinking: think.spec,
      };
      if (!body.id || !body.name || !body.api_model || !body.provider) {
        c.setMsg('id / 名字 / api_model / 供应商 均不能为空', true); return;
      }
      try {
        if (body.tools === 'native') {
          c.setMsg('正在保存：先实测原生工具调用与回放形状（几条最小请求，可能要几秒）…');
        }
        await c.mutate('upsert_model', body);
        c.setMsg('已保存：' + body.id +
          (state.core ? '' : '（还没有核心默认模型：点这一行的「设为核心」即可）'));
      } catch (e) { c.setMsg(e.message, true); }
    };

    // 登记处任何变化都只同步列表 / 下拉 / 核心块，不重建表单（轮询也会走到这里）。
    c.onRegistryChange(() => {
      rebuild();
      rebuildDiscover();
      syncProviderSelect();
      refreshCore();
      thinkEditor.refresh();
    });
    rebuild(); rebuildDiscover(); syncProviderSelect(); refreshCore();
    if (!coreBody) c.body.appendChild(coreWrap);
    c.body.appendChild(listWrap);
    const form = doc.createElement('div'); form.className = 'form-col';
    form.appendChild(field('id', idIn));
    form.appendChild(field('展示名', nameIn));
    form.appendChild(field('api_model', apiIn));
    form.appendChild(field('供应商', providerSel));
    form.appendChild(field('note', noteIn));
    // numberInput 返回 { input, wrap }，而 wrap **已经带标签**——直接把 wrap 挂上去
    //（传对象给 field 会让 appendChild 抛异常，整个表单就建不起来）。
    form.appendChild(ctxIn.wrap);
    form.appendChild(field('工具调用', toolsSel));
    form.appendChild(field('思考档位映射（可选）', thinkEditor.mapping));
    form.appendChild(thinkEditor.defaultField);
    form.appendChild(thinkEditor.defaultHint);
    form.appendChild(save);
    c.body.appendChild(form);
    const discTitle = doc.createElement('div'); discTitle.className = 'wf-label'; discTitle.textContent = '从供应商获取模型（点选填入 api_model）';
    c.body.appendChild(discTitle);
    c.body.appendChild(discWrap);
}

/* ---------- 设置③：核心 AI 默认模型 ---------- */
function coreModelBlock(c, subscribe) {
    const sel = selectInput(app.modelOptions(state.core), state.core);
    const save = btn('保存', 'btn btn-primary btn-block');
    const sync = () => syncSelect(sel, app.modelOptions(state.core), state.core || sel.value);
    if (subscribe !== false && typeof c.onRegistryChange === 'function') c.onRegistryChange(sync);
    save.onclick = async () => {
      const id = sel.value;
      if (!id) { c.setMsg('请先登记模型', true); return; }
      try {
        await c.mutate('set_core_model', { id });
        c.setMsg('核心 AI 默认模型：' + id);
      } catch (e) { c.setMsg(e.message, true); }
    };
    if (!state.models.length) c.body.appendChild(emptyHint('（未登记模型，先到「模型登记」添加）'));
    c.body.appendChild(field('核心 AI 使用的模型', sel));
    c.body.appendChild(save);
}

/* ---------- 设置④：基本设置 ---------- */
/// 设置 1–4 页共用的控件：一次建全，切页只切显示——保存是全量写，不能因为没建过某页丢改动。
function settingsForm() {
  const stream = checkbox('流式传输（供应商逐片返回，边收边显示）', state.settings.streaming);
  const cot = checkbox('思维链显示（每条回答下的思维链，永远默认折叠、点击展开）', state.settings.show_reasoning);
  const to = numberInput('等待首个响应的超时（秒）', state.settings.llm_timeout_secs, 10, 3600);
  const remind = numberInput('一轮内最多提醒几次', state.settings.discuss_remind_cap, 0, 20);
  const pct = numberInput('上下文用到百分之多少就压缩', state.settings.compact_at_percent, 0, 100);
  const keep = numberInput('压缩时至少保留最近内容（占模型窗口 %）', state.settings.compact_keep_percent, 0, 100);
  const compactModels = [{ value: '', label: '默认（会话同模型）' }].concat(
    (state.models || []).map((m) => ({
      value: m.id,
      label: m.name + '（' + m.id + (m.context ? ' · ' + m.context + ' tokens' : '') + '）',
    })),
  );
  if (state.settings.compact_model && !compactModels.some((o) => o.value === state.settings.compact_model)) {
    compactModels.push({ value: state.settings.compact_model, label: state.settings.compact_model + '（登记处已无此模型）' });
  }
  const compactModel = selectInput(compactModels, state.settings.compact_model || '');
  const guests = numberInput('虚拟机档同时最多几台', state.settings.max_guests, 0, 64);
  const sb = state.settings.sandbox || {};
  const sp = sb.policy || {};
  const en = sp.enforce || {};
  const sd = sb.defaults || {};
  const lines = (v) => (Array.isArray(v) ? v.join('\n') : '');
  const area = (label, value) => {
    const t = areaInput('每行一条');
    t.value = lines(value);
    return { t, f: field(label, t) };
  };
  // 思考等级目录：标签即档位，顺序即界面顺序。模型登记只能引用这里的标签。
  const thinkLevels = thinkingTagsEditor(app.thinkingLevels());
  const network = checkbox('允许工具 / guest / 常驻服务 / 构建出站（全局）', !!sp.network);
  const writeAcl = checkbox('允许 Windows 写本机目录 ACL（只影响 Windows 的容器围栏）', !!(sp.windows && sp.windows.write_acl));
  const level = (v) => selectInput([
    { value: 'require', label: 'require：必须强制，不可用时询问是否本进程降级' },
    { value: 'best_effort', label: 'best_effort：尽力而为，不可用时降级并如实报告' },
  ], v || 'require');
  const enFs = level(en.filesystem);
  const enNet = level(en.network);
  const enTree = level(en.process_tree);
  const enEnv = level(en.env_whitelist);
  const onUnavailable = selectInput([
    { value: 'ask', label: 'ask：启动时询问是否本进程降级（不落盘）' },
    { value: 'refuse', label: 'refuse：直接拒绝' },
  ], sp.on_unavailable || 'ask');
  const hostRoots = area('主机只读根（绝对路径，每行一条）', sp.host_read_roots);
  const hardDeny = area('全局黑名单（工作区相对路径，只增不减）', sp.deny);
  const defProfile = selectInput([
    { value: 'view', label: '仅可查看' },
    { value: 'workspace', label: '工作区内修改（默认）' },
    { value: 'external', label: '外部可达（逐次确认）' },
    { value: 'all_reach', label: '全可达（自动放行）' },
  ], sd.profile || 'workspace');
  const scopeArea = (label, value) => {
    const mode = Array.isArray(value) ? 'paths' : (value === 'none' ? 'none' : 'all');
    const sel = selectInput([
      { value: 'all', label: '整棵共享区' },
      { value: 'none', label: '无权限' },
      { value: 'paths', label: '指定路径' },
    ], mode);
    const t = areaInput('每行一条（相对共享区根）');
    t.value = Array.isArray(value) ? value.join('\n') : '';
    const wrap = doc.createElement('div'); wrap.className = 'wf-field';
    wrap.appendChild(field(label + '方式', sel));
    const pathField = field(label + '路径', t);
    wrap.appendChild(pathField);
    const sync = () => { pathField.style.display = sel.value === 'paths' ? '' : 'none'; };
    sel.addEventListener('change', sync);
    sync();
    return {
      wrap,
      get: () => (sel.value === 'all' ? 'all' : sel.value === 'none' ? 'none' : parseSettingLines(t.value)),
    };
  };
  const defSharedRead = scopeArea('默认共享区读范围', (sd.shared && sd.shared.read) || 'all');
  const defSharedWrite = scopeArea('默认共享区写范围', (sd.shared && sd.shared.write) || 'all');
  const defTools = area(
    '默认逐工具决定（每行一条：工具名=allow/ask/deny）',
    Object.entries(sd.tools || {}).map(([k, v]) => k + '=' + v),
  );
  const st = state.settings.storage || {};
  const stPeriodic = checkbox('启用全局周期回收（关掉只停周期；构建前 headroom 始终启用）', st.periodic_gc !== false);
  const stInterval = numberInput('全局自动回收间隔（分钟，至少 15）', st.gc_interval_minutes == null ? 15 : st.gc_interval_minutes, 15, 10080);
  const stCap = numberInput('可回收缓存总预算（GiB；0 = 按磁盘 10%）', st.cache_cap_gib == null ? 20 : st.cache_cap_gib, 0, 4096);
  const stKeep = numberInput('超过多少天未使用可回收（0 = 不按龄）', st.cache_keep_days == null ? 30 : st.cache_keep_days, 0, 3650);
  const stVolume = numberInput('新构建缓存卷大小（GiB）', st.build_volume_gib == null ? 16 : st.build_volume_gib, 1, 1024);
  const stFree = numberInput('宿主最低剩余空间（GiB）', st.min_free_gib == null ? 2 : st.min_free_gib, 0, 1024);
  const stLog = numberInput('日志保留天数（0 = 不按龄）', st.log_keep_days == null ? 14 : st.log_keep_days, 0, 3650);
  return { stream, cot, to, remind, pct, keep, compactModel, guests, thinkLevels, network, writeAcl, enFs, enNet, enTree, enEnv, onUnavailable, hostRoots, hardDeny, defProfile, defSharedRead, defSharedWrite, defTools, stPeriodic, stInterval, stCap, stKeep, stVolume, stFree, stLog };
}

function parseSettingLines(text) {
  return String(text || '').split(/[\n,]/).map((s) => s.trim()).filter((s) => s.length > 0);
}

/// 1–4 页共用一份提交体（set_settings 是部分更新，未提交字段保留现值）。
function collectSettings(f) {
  return {
    streaming: f.stream.box.checked,
    show_reasoning: f.cot.box.checked,
    thinking: { levels: f.thinkLevels.list() },
    llm_timeout_secs: Number(f.to.input.value) || 300,
    discuss_remind_cap: Number(f.remind.input.value) || 0,
    compact_at_percent: Number(f.pct.input.value) || 0,
    compact_keep_percent: Number(f.keep.input.value) || 0,
    compact_model: f.compactModel.value || '',
    max_guests: Number(f.guests.input.value) || 0,
    storage: {
      periodic_gc: f.stPeriodic.box.checked,
      gc_interval_minutes: Number(f.stInterval.input.value) || 15,
      cache_cap_gib: Number(f.stCap.input.value) || 0,
      cache_keep_days: Number(f.stKeep.input.value) || 0,
      build_volume_gib: Number(f.stVolume.input.value) || 16,
      min_free_gib: Number(f.stFree.input.value) || 0,
      log_keep_days: Number(f.stLog.input.value) || 0,
    },
    sandbox: {
      policy: {
        network: f.network.box.checked,
        enforce: {
          filesystem: f.enFs.value,
          network: f.enNet.value,
          process_tree: f.enTree.value,
          env_whitelist: f.enEnv.value,
        },
        on_unavailable: f.onUnavailable.value,
        windows: { write_acl: f.writeAcl.box.checked },
        host_read_roots: parseSettingLines(f.hostRoots.t.value),
        deny: parseSettingLines(f.hardDeny.t.value),
      },
      defaults: {
        profile: f.defProfile.value,
        shared: {
          read: f.defSharedRead.get(),
          write: f.defSharedWrite.get(),
          deny: [],
        },
        tools: (() => {
          const out = {};
          for (const line of parseSettingLines(f.defTools.t.value)) {
            const i = line.indexOf('=');
            if (i < 0) continue;
            const k = line.slice(0, i).trim();
            const v = line.slice(i + 1).trim();
            if (k && ['allow', 'ask', 'deny'].includes(v)) out[k] = v;
          }
          return out;
        })(),
      },
    },
  };
}

async function saveSettings(c, f, saveBtn) {
  const bad = f.thinkLevels.validate();
  if (bad) { c.setMsg(bad, true); return; }
  const reset = busyButton(saveBtn, '保存中…');
  try {
    const saved = await api('POST', app.actionUrl('set_settings'), collectSettings(f));
    await app.refreshState();
    c.setMsg(saved && saved.restart ? '已保存：沙箱策略变更，正在重启…' : '已保存');
    ui.toast(saved && saved.restart ? 'warn' : 'ok', saved && saved.restart ? '已保存，正在重启…' : '设置已保存');
  } catch (e) {
    c.setMsg(e.message, true);
  } finally {
    reset();
  }
}

/* ---------- 设置中心：左侧分页、右侧单页、底栏统一保存 ----------
 * 1–4 页共享同一份 AppSettings（set_settings 是部分更新，未提交字段保留现值）：控件一次建全，切页只切显示。
 * 5–8 页是即时的登记 / 启停操作，不显示保存栏。 */
const SETTINGS_PAGES = [
  { id: 'generate', group: '设置', label: '生成与上下文', icon: 'cpu' },
  { id: 'sandbox', group: '设置', label: '沙箱围栏', icon: 'shield' },
  { id: 'permissions', group: '设置', label: '权限默认值', icon: 'gear' },
  { id: 'resources', group: '设置', label: '资源与进程', icon: 'dot' },
  { id: 'storage', group: '设置', label: '存储空间', icon: 'folder' },
  { id: 'providers', group: '登记', label: '供应商', icon: 'box' },
  { id: 'models', group: '登记', label: '模型与核心默认', icon: 'dot' },
  { id: 'agents', group: '登记', label: 'agent 管理', icon: 'users' },
  { id: 'modules', group: '登记', label: '模块与服务', icon: 'folder' },
];

/* ---------- 通用：多选一弹层（顶替原生 confirm 的三选） ---------- */
function choiceModal(title, question, actions) {
  // 叠在现有弹层之上：从向导里弹出来时，选完能回到向导（不丢已填内容）。
  app.openModalLayer(title, (c) => {
    const q = doc.createElement('div'); q.className = 'wf-label'; q.textContent = question;
    c.body.appendChild(q);
    const row = doc.createElement('div'); row.className = 'wf-inline';
    for (const a of actions) {
      const b = btn(a[0], a[1] || 'btn');
      b.onclick = () => { c.close(); a[2](); };
      row.appendChild(b);
    }
    c.body.appendChild(row);
  });
}