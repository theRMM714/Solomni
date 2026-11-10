/* 目的：设置中心的存储空间页：占用报告、单项 / 全部回收、访客虚拟根还原。
 * 管：报告渲染与回收动作，轮询还原进度。
 * 不管：通用字节格式化（core/format.js 的 fmtBytes）、弹层与提示（ui 层）。
 * 联动：由 pages.js 的 settingsForm 调用；字节显示用 app.fmtBytes。
 */
let g, doc, api, ui, app;
let btn;
/** 挂载：把本功能的入口登记到 app；跨功能调用一律经 app 晚绑定。 */
export function mount(root, deps) {
  ({ g, doc, api, ui, app } = deps);
  ({ btn } = ui);
  Object.assign(root, { buildStoragePage, renderStorage, cleanStorage, resetGuest });
}
/// 5 页：存储空间（盘点 + 回收 + 显式清理 + 策略）。
function buildStoragePage(c, f) {
  c.body.appendChild(f.stPeriodic.wrap);
  c.body.appendChild(f.stInterval.wrap);
  c.body.appendChild(f.stCap.wrap);
  c.body.appendChild(f.stKeep.wrap);
  c.body.appendChild(f.stVolume.wrap);
  c.body.appendChild(f.stFree.wrap);
  c.body.appendChild(f.stLog.wrap);
  c.body.appendChild(app.cfgHint('上限只约束可回收缓存（构建缓存 / 下载缓存 / 过期可写层 / 日志），不约束会话、虚拟根与模块产物。到上限先回收一次，不拒绝构建；全局回收默认且至少 15 分钟一次，关闭自动回收后只剩手动。', 'warn'));
  c.body.appendChild(app.cfgHint('还原虚拟根会删除 guest 内未落宿主的数据，并用清单重建；不备份、不可撤销。', 'err'));
  const box = doc.createElement('div');
  box.className = 'storage-report';
  box.textContent = '读取中…';
  c.body.appendChild(box);
  const refresh = async () => {
    try {
      const r = await api('GET', '/api/storage');
      renderStorage(box, r.report, refresh);
    } catch (e) {
      box.textContent = '读不到：' + e.message;
    }
  };
  refresh();
}

function renderStorage(box, report, refresh) {
  box.innerHTML = '';
  box.className = 'storage-report';
  const head = doc.createElement('div');
  head.className = 'storage-summary' + (report.overCap ? ' err' : '');
  head.textContent = '缓存占用 ' + app.fmtBytes(report.managedBytes) + ' / 上限 ' + app.fmtBytes(report.capBytes) +
    '；宿主剩余 ' + app.fmtBytes(report.freeBytes) + (report.overCap ? '（已超限）' : '');
  box.appendChild(head);
  const note = doc.createElement('div');
  note.className = 'cfg-hint';
  note.textContent = '按策略回收只处理已超上限或超过保留天数的项；未到条件时不会删任何东西。要手动清理，用下方单项或“清空全部缓存”。';
  box.appendChild(note);
  const bar = doc.createElement('div');
  bar.className = 'storage-actions';
  const gc = btn('按策略回收', 'btn btn-primary');
  gc.onclick = async () => {
    try {
      const preview = await api('POST', '/api/storage/gc', { apply: false });
      const lines = (preview.report.actions || []).map((a) => a.label + ' · ' + app.fmtBytes(a.bytes) + ' · ' + a.reason);
      if (!lines.length) {
        ui.toast('warn', '当前没有达到策略条件的项：未超上限，也没有超过保留天数的缓存；可用单项清理或“清空全部缓存”。');
        return;
      }
      if (!(await app.confirmBox('回收预览', lines.join('\n'), '回收'))) return;
      const done = await api('POST', '/api/storage/gc', { apply: true });
      ui.toast('ok', '已回收 ' + app.fmtBytes(done.report.freedBytes));
      await refresh();
    } catch (e) { ui.toast('err', '回收失败：' + e.message); }
  };
  bar.appendChild(gc);
  const clear = btn('清空全部缓存', 'btn');
  clear.onclick = async () => {
    if (!(await app.confirmBox('清空全部缓存', '会清空构建缓存卷、下载缓存、过期可写层与可清理日志；会话与虚拟根不动。', '清空'))) return;
    try {
      for (const kind of ['build-cache', 'download-cache', 'overlays', 'logs']) {
        await api('POST', '/api/storage/clean', { kind, confirm: true });
      }
      ui.toast('ok', '已清空可回收缓存');
      await refresh();
    } catch (e) { ui.toast('err', '清空失败：' + e.message); }
  };
  bar.appendChild(clear);
  box.appendChild(bar);
  for (const cat of report.categories || []) {
    const catEl = doc.createElement('div');
    catEl.className = 'storage-cat';
    const catHead = doc.createElement('div');
    catHead.className = 'storage-cat-head';
    const title = doc.createElement('span');
    title.className = 'storage-cat-title';
    title.textContent = cat.label;
    const size = doc.createElement('span');
    size.className = 'storage-size';
    size.textContent = app.fmtBytes(cat.bytes);
    const note = doc.createElement('span');
    note.className = 'storage-cat-note';
    note.textContent = (cat.autoEvictable ? '自动回收' : '不自动回收') + ' · ' + cat.note;
    catHead.appendChild(title);
    catHead.appendChild(size);
    catHead.appendChild(note);
    if (['download-cache', 'overlays', 'logs'].includes(cat.id) && (cat.items || []).length) {
      const clearOne = btn('清理整类', 'link-btn danger');
      clearOne.onclick = () => cleanStorage(cat.id, null, null, refresh);
      catHead.appendChild(clearOne);
    }
    catEl.appendChild(catHead);
    if (cat.id === 'logs') {
      note.textContent += ' · 共 ' + (cat.items || []).length + ' 份';
      box.appendChild(catEl);
      continue;
    }
    const limit = (cat.items || []).slice(0, 20);
    for (const item of limit) {
      const itemEl = doc.createElement('div');
      itemEl.className = 'storage-item';
      const main = doc.createElement('div');
      main.className = 'storage-item-main';
      main.textContent = item.label + ' · ' + app.fmtBytes(item.bytes) +
        (item.active ? ' · 受保护' : '') + ' · ' + item.detail;
      itemEl.appendChild(main);
      const acts = doc.createElement('div');
      acts.className = 'storage-item-actions';
      if (cat.id === 'build-cache') {
        const opt = btn('优化', 'link-btn');
        opt.onclick = () => cleanStorage('compact', item.id, null, refresh);
        acts.appendChild(opt);
        const del = btn('清理整卷', 'link-btn danger');
        del.onclick = () => cleanStorage('build-cache', item.id, null, refresh);
        acts.appendChild(del);
        for (const mod of item.modules || []) {
          const mb = btn('排队清理 ' + mod, 'link-btn');
          mb.onclick = () => cleanStorage('build-cache', item.id, mod, refresh);
          acts.appendChild(mb);
        }
      } else if (cat.id === 'guest-bases') {
        const reset = btn('还原到初始状态', 'link-btn danger');
        reset.onclick = () => resetGuest(item.id, refresh);
        acts.appendChild(reset);
      }
      if (acts.children.length) itemEl.appendChild(acts);
      catEl.appendChild(itemEl);
    }
    if ((cat.items || []).length > limit.length) {
      const more = doc.createElement('div');
      more.className = 'storage-item-more';
      more.textContent = '还有 ' + (cat.items.length - limit.length) + ' 项未显示';
      catEl.appendChild(more);
    }
    box.appendChild(catEl);
  }
}

async function cleanStorage(kind, id, module, refresh) {
  const names = { 'build-cache': '构建缓存', 'download-cache': '下载缓存', overlays: '可写层', logs: '日志', compact: '压缩' };
  if (!(await app.confirmBox('清理' + (names[kind] || kind), '确认清理：' + (id || '整类') + '？', '清理'))) return;
  try {
    const r = await api('POST', '/api/storage/clean', { kind, id: id || null, module: module || null, confirm: true });
    if (r.result && r.result.started) {
      ui.toast('ok', '已开始，进度见下方');
    } else if (module) {
      ui.toast('ok', '已排队：下次构建前回收 ' + module);
    } else {
      ui.toast('ok', '完成；释放 ' + app.fmtBytes((r.result && r.result.report && r.result.report.freedBytes) || 0));
    }
    await refresh();
  } catch (e) { ui.toast('err', '清理失败：' + e.message); }
}

async function resetGuest(guest, refresh) {
  if (!(await app.confirmBox('还原虚拟根', '会删除 guest 内未落宿主的数据，并按清单重建；不备份、不可撤销。', '继续', true))) return;
  if (!(await app.confirmBox('再次确认', '真的要还原「' + guest + '」吗？', '还原', true))) return;
  try {
    const r = await api('POST', '/api/storage/clean', { kind: 'guest-reset', id: guest, confirm: true });
    ui.toast('ok', '已开始还原：' + guest);
    const poll = async () => {
      const s = await api('GET', '/api/storage');
      const j = (s.report && s.report.job) || {};
      if (j.running) { g.setTimeout(poll, 1000); return; }
      await refresh();
      if (j.error) ui.toast('err', '还原失败：' + j.error);
      else ui.toast('ok', '已还原：' + guest);
    };
    g.setTimeout(poll, 1000);
  } catch (e) { ui.toast('err', '还原失败：' + e.message); }
}