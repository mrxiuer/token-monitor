'use strict';

(function exposeDuoView(root) {
  const duo = root.TokenMonitorDuo;
  let chosenCycle = '', settingsContext = null;
  const expanded = new Set();
  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }
  function time(value) {
    const date = new Date(value);
    return Number.isFinite(date.getTime()) ? date.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false }) : '—';
  }
  function coverage(usage) {
    if (usage.status === 'unavailable') return '等待此设备上传周期统计';
    if (usage.status !== 'complete') return '部分数据可用';
    return '已读取可用日志';
  }
  function render(options) {
    const { state, els, formatCost, formatNumber, setTotalCost, renderViewSwitcher, signalContentReady } = options;
    const panel = document.getElementById('duoPanel');
    const active = ['member', 'device'].includes(state.breakdown) && ['today', 'cycle', 'month', 'allTime'].includes(state.period);
    panel.hidden = !active;
    if (!active) return false;
    const data = duo.snapshot(state.stats, state.period, state.period === 'cycle' ? chosenCycle : '');
    els.shell.classList.remove('home-mode', 'session-mode');
    for (const id of ['homePanel', 'breakdown', 'limitsPanel', 'trendsPanel', 'serviceStatusPanel', 'sessionDetail', 'sessionDetailHead', 'sessionPagerHost', 'toolDetailFooter', 'projectBreakdownFooter', 'fixedPeriodMessage', 'viewBackRow']) els[id]?.classList.add('hidden');
    els.totalTokens.textContent = data.total.status === 'unavailable' ? '—' : formatNumber(data.total.totalTokens);
    els.totalTokensCompact.classList.add('hidden');
    setTotalCost(data.total.costUsd, data.total.unpricedTokens, data.total.status !== 'unavailable');
    state.currentTotal = data.total.totalTokens;
    const signature = JSON.stringify([state.period, state.breakdown, state.stats.duo, data.devices, state.settings?.duoUsername]);
    if (panel.dataset.signature === signature) { signalContentReady(); return true; }
    panel.dataset.signature = signature;
    panel.replaceChildren();
    const header = element('div', 'duo-period-info');
    header.append(element('div', 'duo-period-label', state.period === 'cycle'
      ? data.cycle ? `${time(data.cycle.start)} 至 ${time(data.cycle.end)}` : '等待共享 Pro 的周额度信息'
      : { today: '当天 Codex 用量', month: '当月 Codex 用量', allTime: '全部 Codex 用量' }[state.period]));
    if (state.period === 'cycle') {
      const status = !data.cycle ? '在设置中选择 Pro 额度来源，也可以手动校正周期。'
        : data.expired ? '本轮已结束，等待 Pro 更新重置时间；暂不推算下一轮。'
        : `${state.stats.duo?.manual ? '手动周期' : '按共享 Pro 周额度周期统计'} · 费用为 API 价格估算`;
      header.append(element('div', 'duo-note', status));
      if (data.total.status === 'partial') header.append(element('div', 'duo-note', '当前仅汇总可用数据，请展开查看设备完整性。'));
      const controls = element('div', 'duo-period-actions');
      const select = element('select');
      select.setAttribute('aria-label', '选择额度周期');
      select.append(new Option('当前周期', ''));
      for (const item of [...(state.stats.duo?.cycles || [])].reverse()) {
        if (item.id !== state.stats.duo?.cycle?.id) select.append(new Option(`${time(item.start)} — ${time(item.end)}`, item.id));
      }
      select.value = chosenCycle;
      select.addEventListener('change', () => { chosenCycle = select.value; panel.dataset.signature = ''; settingsContext.render(); });
      const exportButton = element('button', '', '导出 CSV');
      exportButton.disabled = !data.cycle;
      exportButton.addEventListener('click', () => {
        const rows = [['成员', '设备', '周期开始', '周期结束', 'Token', '估算费用_USD', '未计价Token', '覆盖状态'],
          ...data.devices.map((device) => [device.memberName, device.displayName || device.hostname || device.deviceId,
            data.cycle.start, data.cycle.end, ...['totalTokens', 'costUsd', 'unpricedTokens'].map((field) => device.usage.status === 'unavailable' ? '' : device.usage[field]), coverage(device.usage)])];
        const csv = '\ufeff' + rows.map((row) => row.map(duo.csvCell).join(',')).join('\r\n');
        const link = element('a'); const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' }));
        link.href = url; link.download = `codex-cycle-${data.cycle.start.slice(0, 10)}.csv`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
      });
      controls.append(select, exportButton); header.append(controls);
    }
    panel.append(header);
    const list = element('div', 'duo-rows');
    const rows = state.breakdown === 'device' ? data.devices.map((device) => ({ id: device.deviceId,
      name: device.displayName || device.hostname || device.deviceId, usage: device.usage, devices: [device], memberName: device.memberName }))
      : data.members;
    for (const row of rows.sort((a, b) => b.usage.totalTokens - a.usage.totalTokens)) {
      const detail = element('details', 'duo-row');
      detail.open = expanded.has(row.id);
      detail.addEventListener('toggle', () => { if (detail.open) expanded.add(row.id); else expanded.delete(row.id); });
      const head = element('summary', 'duo-row-head');
      const title = element('div', 'duo-row-title');
      title.append(element('span', 'duo-row-name', row.name), element('span', 'duo-note',
        `${state.breakdown === 'device' ? row.memberName : `${row.devices.length} 台设备`} · ${coverage(row.usage)}`));
      const metric = element('div', 'duo-row-values');
      metric.append(element('strong', '', row.usage.status === 'unavailable' ? '—' : formatNumber(row.usage.totalTokens)),
        element('span', 'duo-row-cost', row.usage.status === 'unavailable' ? '—' : row.usage.unpricedTokens > 0 ? `${formatCost(row.usage.costUsd)} + 未计价` : formatCost(row.usage.costUsd)));
      head.append(title, metric);
      const bar = element('div', 'duo-row-bar'); const fill = element('span');
      fill.style.width = `${data.total.totalTokens ? row.usage.totalTokens / data.total.totalTokens * 100 : 0}%`; bar.append(fill);
      const contents = element('div', 'duo-row-detail');
      contents.append(element('div', 'duo-note', `${coverage(row.usage)} · 输入 ${formatNumber(row.usage.inputTokens || 0)} / 输出 ${formatNumber(row.usage.outputTokens || 0)} / 缓存 ${formatNumber(row.usage.cacheReadTokens || 0)}`));
      for (const device of row.devices) {
        const line = element('div', 'duo-device-line');
        const updated = device.usage.updatedAt || device.updatedAt;
        line.append(element('span', '', device.displayName || device.hostname || device.deviceId),
          element('span', '', device.usage.status === 'unavailable' ? '等待上传' : `${formatNumber(device.usage.totalTokens)} · ${formatCost(device.usage.costUsd, device.usage.unpricedTokens)}`));
        contents.append(line, element('div', 'duo-note', `${device.stale ? '离线 · ' : ''}最近采集 ${time(updated)} · ${coverage(device.usage)}`));
      }
      head.append(bar);
      detail.append(head, contents); list.append(detail);
    }
    if (!rows.length) list.append(element('p', 'duo-note', '连接 Hub 并填写用户名后，会显示成员与设备。'));
    panel.append(list);
    renderViewSwitcher(); signalContentReady();
    return true;
  }

  function installSettings(context) {
    settingsContext = context;
    const { state, refreshStats } = context;
    document.getElementById('automaticAppUpdatesRow').hidden = true;
    document.querySelector('.app-update-settings').append(element('p', 'settings-note', '定制版使用手动更新，下载入口指向自己的 fork。'));
    const sync = document.querySelector('.settings-sync-group');
    sync.parentElement.prepend(sync);
    const group = element('div', 'settings-group duo-cycle-settings');
    group.append(element('h3', '', '额度周期与成员'));
    const source = element('select'); source.setAttribute('aria-label', '共享 Pro 额度来源');
    const message = element('p', 'settings-note');
    const sourceStatus = element('p', 'settings-note');
    const selectSource = element('button', '', '固定此 Pro 来源');
    const manual = element('details', 'duo-advanced'); manual.append(element('summary', '', '手动校正周期'));
    const start = element('input'); start.type = 'datetime-local'; start.setAttribute('aria-label', '周期开始');
    const end = element('input'); end.type = 'datetime-local'; end.setAttribute('aria-label', '周期结束');
    const save = element('button', '', '保存周期');
    async function configure(payload) {
      try { await window.tokenMonitor.configureDuo(payload); await refreshStats(); message.textContent = '已保存'; }
      catch (error) { message.textContent = error.message || String(error); }
    }
    selectSource.addEventListener('click', () => { if (source.value) void configure({ source: JSON.parse(source.value) }); });
    save.addEventListener('click', () => {
      if (!start.value || !end.value) { message.textContent = '请填写完整的起止时间'; return; }
      void configure({ manual: { start: new Date(start.value).toISOString(), end: new Date(end.value).toISOString() } });
    });
    manual.append(start, end, save);
    const members = element('div', 'duo-member-settings');
    group.append(source, sourceStatus, selectSource, manual, members, message);
    sync.after(group);
    const advanced = element('details', 'settings-group duo-advanced'); advanced.append(element('summary', '', '高级设置'));
    const panel = document.getElementById('settingsPanel');
    panel.append(advanced);
    for (const selector of ['.settings-main-group', '.settings-tools-group', '.settings-limits-group', '.settings-appearance-section-group']) {
      const section = panel.querySelector(selector); if (section) advanced.append(section);
    }
    for (const selector of ['.settings-subscriptions-group', '.settings-plugins-group']) {
      const section = panel.querySelector(selector); if (section) section.hidden = true;
    }
    function update() {
      const options = [];
      for (const device of state.stats?.devices || []) for (const provider of device.limits?.providers || []) {
        if (provider.provider === 'codex' && /pro/i.test(provider.planLabel || '')) options.push({
          value: JSON.stringify({ deviceId: device.deviceId, accountKey: provider.accountKey || '' }),
          text: `${device.displayName || device.hostname || device.deviceId} · ${provider.planLabel}` });
      }
      const key = JSON.stringify(options);
      if (source.dataset.key !== key) {
        source.replaceChildren(new Option(options.length ? '选择共享 Pro 的额度来源' : '等待 Pro 额度数据', ''));
        options.forEach((option) => source.append(new Option(option.text, option.value))); source.dataset.key = key;
      }
      const pinned = state.stats?.duo?.source;
      const pinnedValue = pinned ? JSON.stringify(pinned) : '';
      if (document.activeElement !== source) source.value = options.some((option) => option.value === pinnedValue) ? pinnedValue : '';
      const current = state.stats?.duo?.cycle;
      sourceStatus.textContent = current ? `${state.stats.duo.manual ? '手动指定' : `已固定来源 ${pinned?.deviceId || ''}`} · ${time(current.start)} 至 ${time(current.end)}` : '等待读取共享 Pro 的周额度，或手动指定起止时间。';
      selectSource.disabled = !source.value;
      source.onchange = () => { selectSource.disabled = !source.value; };
      const devices = state.stats?.devices || [];
      const signature = JSON.stringify(devices.map((device) => [device.deviceId, device.memberId, device.memberName]));
      if (members.dataset.signature !== signature && !members.contains(document.activeElement)) {
        members.replaceChildren(element('p', 'duo-note', '设备归属（同名自动归组）'));
        for (const device of devices) {
          const row = element('label', 'duo-device-line');
          const name = element('input'); name.value = device.memberName || ''; name.placeholder = '用户名'; name.maxLength = 40;
          name.addEventListener('change', () => { if (duo.username(name.value)) void configure({ assign: { deviceId: device.deviceId, username: name.value } }); });
          row.append(element('span', '', device.displayName || device.hostname || device.deviceId), name); members.append(row);
        }
        for (const member of state.stats?.duo?.members || []) {
          const row = element('label', 'duo-device-line'); const name = element('input'); name.value = member.name; name.maxLength = 40;
          name.addEventListener('change', () => void configure({ rename: { id: member.id, name: name.value } }));
          row.append(element('span', '', '修改成员昵称'), name); members.append(row);
        }
        members.dataset.signature = signature;
      }
    }
    window.addEventListener('duo:stats', update);
    update();
    // Keep the edition's common path compact while retaining the upstream
    // appearance controls and advanced diagnostics for deliberate use.
    for (const id of ['hubModeInput', 'syncModeInput']) {
      const input = document.getElementById(id);
      if (input?.tagName === 'SELECT') for (const option of input.options) option.hidden = option.value !== 'client';
    }
  }
  root.TokenMonitorDuoView = { render, installSettings };
})(window);
