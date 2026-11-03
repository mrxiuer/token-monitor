'use strict';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const state = { period: localStorage.getItem('period') || 'cycle', group: localStorage.getItem('group') || 'member', cycle: '', settings: null, settingsOpen: false, dirty: false, changedFields: new Set(), revision: 0, expanded: new Set(), view: null, status: null };
const count = (value) => Number(value || 0).toLocaleString('en-US');
const cost = (value) => new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(value || 0);
const time = (value) => value && Number.isFinite(Date.parse(value)) ? new Date(value).toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false }) : '—';
function node(tag, className, text) { const item = document.createElement(tag); if (className) item.className = className; if (text !== undefined) item.textContent = text; return item; }
function coverage(usage) { return usage.status === 'unavailable' ? '等待上传周期统计' : usage.status === 'partial' ? '部分数据可用' : '已读取可用日志'; }
function showMessage(text) { $('message').textContent = text; $('message').hidden = !text; }
function applyAppearance(settings) {
  document.documentElement.classList.toggle('light', settings.theme === 'light' || settings.theme === 'system' && matchMedia('(prefers-color-scheme:light)').matches);
  document.documentElement.style.setProperty('--glass-alpha', settings.opacity);
}
async function render() {
  const revision = ++state.revision;
  try {
    const result = await invoke('get_view', { period: state.period, group: state.group, cycle: state.cycle });
    if (revision !== state.revision) return;
    if (state.cycle && !result.view.cycles.some((cycle) => cycle.id === state.cycle)) { state.cycle = ''; void render(); return; }
    const { view, status } = result; state.view = view; state.status = status;
    $('liveDot').classList.toggle('online', status.connected);
    $('syncStatus').textContent = status.collecting ? '采集中…' : status.pending ? '待同步' : status.connected ? '已同步' : '离线缓存';
    $('syncStatus').title = `${status.syncMessage}\n${status.collectionMessage}\n最近采集 ${time(status.lastCollectedAt)}\n最近上传 ${time(status.lastUploadedAt)}`;
    $('connectionSummary').textContent = status.connected ? '已连接' : status.syncMessage;
    $('totalTokens').textContent = view.total.status === 'unavailable' ? '—' : count(view.total.totalTokens);
    $('totalCost').textContent = view.total.status === 'unavailable' ? status.collectionMessage : `${view.total.unpricedTokens && !view.total.costUsd ? '费用待计价' : cost(view.total.costUsd)}${view.total.unpricedTokens ? ` + ${count(view.total.unpricedTokens)} 未计价 Token` : ''}`;
    document.querySelectorAll('[data-period]').forEach((button) => button.classList.toggle('active', button.dataset.period === state.period));
    $('periodLabel').textContent = state.period === 'cycle' ? view.cycle ? `${time(view.cycle.start)} 至 ${time(view.cycle.end)}` : '等待共享 Pro 周额度信息' : { today: '当天 Codex 用量', month: '当月 Codex 用量', allTime: '全部 Codex 用量' }[state.period];
    $('periodNote').textContent = state.period === 'cycle' && !view.cycle ? '连接 Hub 后，将根据共享 Pro 的额度信息自动识别。' : view.expired ? '本轮已结束，等待 Pro 更新重置时间。' : `${view.cycleStatus === 'checking-reset' ? '正在确认额度变更 · ' : ''}费用为 API 估算${view.total.status === 'partial' ? ' · 仅汇总可用数据' : ''}`;
    $('cycleSelect').hidden = state.period !== 'cycle' || !view.cycles.length;
    if (document.activeElement !== $('cycleSelect')) {
      $('cycleSelect').replaceChildren(new Option('当前额度周期', ''));
      for (const cycle of [...view.cycles].reverse()) if (cycle.id !== view.cycle?.id || state.cycle) $('cycleSelect').append(new Option(`${time(cycle.start)} — ${time(cycle.end)}`, cycle.id));
      $('cycleSelect').value = state.cycle;
    }
    $('pendingPanel').hidden = !view.pendingRanges?.length;
    if (view.pendingRanges?.length) { $('pendingSummary').textContent = `重置边界待确认 · ${view.pendingTotal.status === 'unavailable' ? '等待设备上传' : `${count(view.pendingTotal.totalTokens)} Token · ${cost(view.pendingTotal.costUsd)}${view.pendingTotal.status === 'partial' ? ' · 部分设备待上传' : ''}${view.pendingTotal.unpricedTokens ? ' + 未计价' : ''}`}`; $('pendingIntervals').textContent = view.pendingRanges.map((range) => `${time(range.start)} 至 ${time(range.end)}`).join('；'); }
    renderRows(view);
    if (state.settingsOpen) renderSharedSettings(view);
    if (status.warning && !$('message').textContent) showMessage(status.warning);
  } catch (error) { showMessage(String(error)); }
}
function renderRows(view) {
  $('rows').hidden = state.group === 'quota'; $('quotaPanel').hidden = state.group !== 'quota';
  if (state.group === 'quota') {
    const source = view.proSources.find((source) => source.deviceId === view.source?.deviceId && source.accountKey === view.source?.accountKey);
    $('quotaPanel').replaceChildren();
    if (!source) { $('quotaPanel').append(node('p', 'empty', '等待读取共享 Pro 的额度信息。设备用量来自各自的本地日志。')); return; }
    const card = node('section', 'quota-card'); card.append(node('h3', '', source.label));
    for (const window of source.provider.windows || []) if (!window.additional) {
      card.append(node('div', 'note', window.kind === 'weekly' ? '共享账号周额度' : '共享账号短期额度'), node('div', 'quota-metric', `剩余 ${Math.max(0, 100 - window.usedPercent).toFixed(0)}%`), node('div', 'note', `重置时间 ${time(window.resetsAt)}`));
    }
    card.append(node('p', 'note', `额度读取于 ${time(source.provider.updatedAt)}`), node('p', 'note', '这里是账号共享的官方额度读数，不按个人分配。')); $('quotaPanel').append(card); return;
  }
  $('rows').replaceChildren();
  for (const row of view.rows) {
    const details = node('details', 'usage-row'); details.open = state.expanded.has(row.id);
    details.addEventListener('toggle', () => { if (!details.isConnected) return; if (details.open) state.expanded.add(row.id); else state.expanded.delete(row.id); });
    const summary = node('summary'); const title = node('div', 'row-title');
    title.append(node('span', 'row-name', row.name), node('div', 'note row-subtitle', `${state.group === 'member' ? `${row.devices.length} 台设备` : row.devices[0].memberName} · ${coverage(row.usage)}`));
    const metric = node('div', 'row-metric');
    metric.append(node('strong', '', row.usage.status === 'unavailable' ? '—' : count(row.usage.totalTokens)), node('span', '', row.usage.status === 'unavailable' ? '—' : `${cost(row.usage.costUsd)}${row.usage.unpricedTokens ? ' + 未计价' : ''}`));
    const bar = node('div', 'usage-bar'); const fill = node('span'); fill.style.width = `${view.total.totalTokens ? row.usage.totalTokens / view.total.totalTokens * 100 : 0}%`; bar.append(fill);
    summary.append(title, metric, bar); const contents = node('div', 'row-detail');
    if (view.pendingRanges?.length) contents.append(node('p', 'note pending-caption', `待确认区间：${row.pendingUsage.status === 'unavailable' ? '等待设备上传' : `${count(row.pendingUsage.totalTokens)} Token · ${cost(row.pendingUsage.costUsd)}`}`));
    for (const device of row.devices) {
      if (state.group === 'device') { contents.append(deviceContent(device)); continue; }
      const detail = node('details', 'device-detail'), heading = node('summary', 'device-heading');
      detail.open = state.expanded.has(`device:${device.id}`);
      detail.addEventListener('toggle', () => { if (!detail.isConnected) return; if (detail.open) state.expanded.add(`device:${device.id}`); else state.expanded.delete(`device:${device.id}`); });
      heading.append(node('span', '', device.name), node('span', '', device.usage.status === 'unavailable' ? '等待上传' : `${count(device.usage.totalTokens)} · ${cost(device.usage.costUsd)}`));
      detail.append(heading, deviceContent(device)); contents.append(detail);
    }
    details.append(summary, contents); $('rows').append(details);
  }
  if (!view.rows.length) $('rows').append(node('p', 'empty', '本机用量将显示在这里。连接 Hub 并填写用户名后，可以查看成员和设备用量。'));
}
function deviceContent(device) {
  const contents = node('div', 'device-content');
  const system = device.osName || (device.platform.startsWith('darwin') || device.platform.startsWith('mac') ? 'macOS' : device.platform.startsWith('win') ? 'Windows' : device.platform);
  contents.append(node('p', 'note', `${system}${device.osVersion ? ` ${device.osVersion}` : ''}${device.agentVersion ? ` · 客户端 ${device.agentVersion}` : ''}`), node('p', 'note', `${device.stale ? '离线 · ' : ''}最近采集 ${time(device.usage.updatedAt || device.updatedAt)} · 最近同步 ${time(device.receivedAt)}`));
  if (device.usage.componentsAvailable) contents.append(node('p', 'note', `输入 ${count(device.usage.inputTokens)} / 输出 ${count(device.usage.outputTokens)} / 缓存 ${count(device.usage.cacheReadTokens)}`));
  else contents.append(node('p', 'note', 'Token 分类明细尚未上传'));
  appendModels(contents, device, device.usage, 'known');
  if (state.view.pendingRanges?.length && device.pendingUsage.status !== 'unavailable') {
    const pending = node('details', 'pending-caption'), heading = node('summary');
    heading.textContent = `待确认区间 ${count(device.pendingUsage.totalTokens)} Token · ${cost(device.pendingUsage.costUsd)}`;
    pending.append(heading, node('p', 'note', '这段用量暂不计入相邻周期。'));
    appendModels(pending, device, device.pendingUsage, 'pending'); contents.append(pending);
  }
  return contents;
}
function appendModels(contents, device, source, scope) {
  const models = Object.entries(source.models || {}).sort((a, b) => b[1].totalTokens - a[1].totalTokens);
  if (!models.length) contents.append(node('p', 'note model-empty', source.status === 'complete' && !source.totalTokens ? '本周期暂无模型用量' : '模型明细尚未上传'));
  for (const [name, usage] of models) {
    const model = node('details', 'model-row'), head = node('summary', 'model-heading');
    const key = `model:${device.id}:${name}:${scope}:${state.period}:${state.cycle}`; model.open = state.expanded.has(key);
    model.addEventListener('toggle', () => { if (!model.isConnected) return; if (model.open) state.expanded.add(key); else state.expanded.delete(key); });
    head.append(node('span', 'model-name', name), node('span', 'model-count', count(usage.totalTokens)));
    const detail = node('div', 'model-details');
    detail.append(node('p', 'note', usage.priceAvailable === false ? '估算费用未提供' : `估算费用 ${cost(usage.costUsd)}${usage.unpricedTokens ? ` · ${count(usage.unpricedTokens)} Token 未计价` : ''}`));
    detail.append(node('p', 'note', usage.componentsAvailable ? `输入 ${count(usage.inputTokens)} / 输出 ${count(usage.outputTokens)} / 缓存 ${count(usage.cacheReadTokens)}` : 'Token 分类明细尚未上传'));
    model.append(head, detail); contents.append(model);
  }
}

const fields = ['username', 'hubUrl', 'deviceName', 'deviceId', 'systemGlass', 'glassStyle', 'theme', 'opacity', 'zoom', 'alwaysOnTop', 'startAtLogin', 'shortcut', 'codexHome'];
function fillSettings(bootstrap) {
  state.settings = bootstrap.settings;
  for (const id of fields) if ($(id).type === 'checkbox') $(id).checked = state.settings[id]; else $(id).value = state.settings[id];
  $('extraRoots').value = state.settings.extraRoots.join('\n'); $('secret').placeholder = bootstrap.secretConfigured ? '已保存，留空保持不变' : '共享 Hub 密钥';
  $('version').textContent = `Token Monitor Duo ${bootstrap.version} · Rust + Tauri`;
  applyAppearance(state.settings);
}
function renderSharedSettings(view) {
  const source = view.proSources.find((item) => item.deviceId === view.source?.deviceId && item.accountKey === view.source?.accountKey);
  $('sourceStatus').textContent = view.cycle ? `${source?.label || '共享 Pro'} · ${time(view.cycle.start)} 至 ${time(view.cycle.end)}` : '等待读取共享 Pro 的周额度信息。';
  $('quotaReadStatus').textContent = source?.provider.updatedAt ? `额度信息更新于 ${time(source.provider.updatedAt)}` : state.status?.quotaMessage || '等待设备上报额度信息。';
}
async function setSettingsOpen(open) {
  state.settingsOpen = open; $('dashboard').hidden = open; $('settingsPanel').hidden = !open; $('periodTabs').hidden = open;
  if (open) { if (!state.dirty) fillSettings(await invoke('bootstrap')); if (state.view) renderSharedSettings(state.view); }
  else { state.dirty = false; state.changedFields.clear(); showMessage(''); await render(); }
}
document.querySelectorAll('[data-period]').forEach((button) => button.addEventListener('click', () => { state.period = button.dataset.period; state.cycle = ''; localStorage.setItem('period', state.period); void render(); }));
$('groupSelect').value = state.group;
$('groupSelect').addEventListener('change', () => { state.group = $('groupSelect').value; localStorage.setItem('group', state.group); void render(); });
$('cycleSelect').addEventListener('change', () => { state.cycle = $('cycleSelect').value; void render(); });
$('closeButton').addEventListener('click', () => void invoke('window_action', { action: 'hide' }));
$('minimizeButton').addEventListener('click', () => void invoke('window_action', { action: 'minimize' }));
$('refreshButton').addEventListener('click', () => void invoke('refresh'));
$('settingsButton').addEventListener('click', () => void setSettingsOpen(!state.settingsOpen));
$('cancelSettings').addEventListener('click', () => void setSettingsOpen(false));
$('titlebar').addEventListener('mousedown', (event) => { if (event.button === 0 && !event.target.closest('button,select,input')) void invoke('window_action', { action: 'drag' }); });
$('settingsPanel').addEventListener('input', (event) => { state.dirty = true; state.changedFields.add(event.target.id); });
$('saveSettings').addEventListener('click', async () => {
  const settings = { ...(await invoke('bootstrap')).settings };
  for (const id of fields) if (state.changedFields.has(id)) settings[id] = $(id).type === 'checkbox' ? $(id).checked : ['opacity', 'zoom'].includes(id) ? Number($(id).value) : $(id).value;
  if (state.changedFields.has('extraRoots')) settings.extraRoots = $('extraRoots').value.split('\n').map((value) => value.trim()).filter(Boolean);
  if (settings.hubUrl && !settings.username.trim()) { showMessage('请填写用户名，同名电脑会自动归组'); $('connectionSection').open = true; $('username').focus(); return; }
  $('saveSettings').disabled = true;
  try { fillSettings(await invoke('save_settings', { settings, secret: $('secret').value || null })); $('secret').value = ''; state.dirty = false; state.changedFields.clear(); showMessage('已保存'); await render(); }
  catch (error) { showMessage(String(error)); } finally { $('saveSettings').disabled = false; }
});
document.querySelectorAll('[data-link]').forEach((button) => button.addEventListener('click', () => void invoke('open_project_link', { kind: button.dataset.link })));
matchMedia('(prefers-color-scheme:light)').addEventListener('change', () => { if (state.settings) applyAppearance(state.settings); });
void (async () => { fillSettings(await invoke('bootstrap')); await listen('duo-changed', () => void render()); await render(); if (!state.settings.username) await setSettingsOpen(true); })().catch((error) => showMessage(String(error)));
