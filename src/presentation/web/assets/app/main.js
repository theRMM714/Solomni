/* 目的：浏览器端的模块入口，也是唯一的组装点：建共享上下文（状态 / API / 组件库）、按顺序挂载各功能、启动接线。
 * 管：装配顺序与依赖注入；把 core/format.js 的纯函数直接放到 app 上；最后调用 shell 的启动接线。
 * 不管：任何业务、渲染与 DOM 细节；功能自己决定用到哪些注入，功能之间互不 import。
 * 联动：`index.html` 先引本文件（type="module"）；`md.js` 提供 `markdownToHtml`。
 *       冒烟（`*.smoke.cjs`）对沙箱调用 `boot`，与浏览器走同一段装配；`context` 暴露本次装配的共享对象。 */

import { createApi } from './core/http.js';
import { createEventStream } from './core/events.js';
import { createUi } from './ui/index.js';
import * as stateStore from './stores/state.js';
import * as format from './core/format.js';
import { mount as mountSessions } from './stores/sessions.js';
import { mount as mountSettings } from './stores/settings.js';
import { mount as mountDialog } from './ui/dialog.js';
import { mount as mountModal } from './ui/modal.js';
import { mount as mountPrint } from './features/status-bar/print.js';
import { mount as mountSettingsPages } from './features/settings/pages.js';
import { mount as mountSettingsRuntime } from './features/settings/runtime.js';
import { mount as mountSettingsStorage } from './features/settings/storage.js';
import { mount as mountSettingsHub } from './features/settings/hub.js';
import { mount as mountAgents } from './features/agents/agents.js';
import { mount as mountFiles } from './features/files/files.js';
import { mount as mountSetup } from './features/setup/setup.js';
import { mount as mountModules } from './features/modules/modules.js';
import { mount as mountSidebar } from './features/sidebar/sidebar.js';
import { mount as mountWizard } from './features/wizard/wizard.js';
import { mount as mountSessionConfig } from './features/session-config/session-config.js';
import { mount as mountTabs } from './features/tabs/tabs.js';
import { mount as mountStatusBar } from './features/status-bar/status-bar.js';
import { mount as mountComposer } from './features/composer/composer.js';
import { mount as mountTranscript } from './features/transcript/transcript.js';
import { mount as mountActions } from './features/actions/actions.js';
import { mount as mountWork } from './features/work/work.js';
import { mount as mountCommits } from './features/work/commits.js';
import { mount as mountShell, start } from './features/shell/shell.js';

/* 装配顺序：先是读法（stores）与通用组件（ui/dialog、ui/modal、状态栏派生），再是各功能区，
 * 最后是外壳（它要在启动时调用前面所有入口）。 */
const MOUNTS = [
  mountSessions, mountSettings,
  mountDialog, mountModal, mountPrint,
  mountSettingsPages, mountSettingsRuntime, mountSettingsStorage, mountSettingsHub,
  mountAgents, mountFiles, mountSetup, mountModules,
  mountSidebar, mountWizard, mountSessionConfig, mountTabs, mountStatusBar, mountComposer,
  mountTranscript, mountActions, mountWork, mountCommits, mountShell,
];

/** 本次装配的共享对象；冒烟用它取 state / api / ui。 */
export let context = null;

/** 装配一次：浏览器与冒烟沙箱走同一条路径。 */
export function boot(g) {
  const doc = g.document;
  const fetchImpl = (url, init) => g.fetch(url, init);
  const state = stateStore.createState();
  const api = createApi(fetchImpl);
  const ui = createUi(g);
  const app = {};
  const deps = {
    g, doc, state, stores: stateStore, api, ui, app,
    // 事件流要宿主提供的 fetch / setTimeout，由这里收口，功能自己不碰宿主。
    createEventStream: (hooks) => createEventStream({
      fetch: fetchImpl,
      setTimeout: (fn, ms) => g.setTimeout(fn, ms),
      ...hooks,
    }),
  };
  Object.assign(app, format);
  for (const mount of MOUNTS) mount(app, deps);
  start();
  context = { state, api, ui, app, deps };
  return app;
}

/* 浏览器里由本文件自启；Node（冒烟）由调用方显式 boot(沙箱)。 */
if (typeof window !== "undefined" && typeof document !== "undefined") boot(globalThis);
