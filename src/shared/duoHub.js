'use strict';

const crypto = require('node:crypto');
const duo = require('./duo');

function createDuoHub(store, persist, now = Date.now) {
  store.duo ||= { members: [], bindings: {}, cycles: [], source: null };
  const state = store.duo;
  function remember(current) {
    state.cycle = current;
    if (!state.cycles.some((entry) => entry.id === current.id)) state.cycles.push(current);
    state.cycles = state.cycles.slice(-104);
  }
  function register(deviceId, name) {
    const clean = duo.username(name);
    if (!clean) return;
    if (!deviceId || clean.includes('\0')) throw new Error('用户名或设备 ID 无效');
    let member = state.members.find((entry) => duo.usernameKey(entry.name) === duo.usernameKey(clean));
    if (!member) { member = { id: crypto.randomUUID(), name: clean }; state.members.push(member); }
    Object.defineProperty(state.bindings, deviceId, { value: member.id, enumerable: true, writable: true, configurable: true });
  }
  function decorate(record) {
    const { memberId: _id, memberName: _name, ...device } = record;
    const id = Object.hasOwn(state.bindings, record.deviceId) ? state.bindings[record.deviceId] : null;
    const member = state.members.find((entry) => entry.id === id);
    return member ? { ...device, memberId: member.id, memberName: member.name } : device;
  }
  function refresh(records) {
    if (state.manual) return;
    const candidates = [];
    for (const record of records) for (const provider of record.limits?.providers || []) {
      if (provider.provider !== 'codex' || !/pro/i.test(provider.planLabel || '')) continue;
      if (state.source && ((provider.accountKey || '') !== state.source.accountKey || record.deviceId !== state.source.deviceId)) continue;
      if (provider.status !== 'ok') continue;
      for (const window of provider.windows || []) {
        if (window.kind !== 'weekly' || window.additional || !window.resetsAt) continue;
        const duration = Number(window.windowMinutes);
        if (!(duration > 0)) continue;
        const end = Date.parse(window.resetsAt);
        if (!Number.isFinite(end)) continue;
        const current = duo.cycle({ start: new Date(end - duration * 60000).toISOString(), end: window.resetsAt });
        if (current && end > now() && Date.parse(current.start) <= now()) candidates.push({ current, provider, record });
      }
    }
    candidates.sort((a, b) => Date.parse(b.provider.updatedAt || b.record.updatedAt) - Date.parse(a.provider.updatedAt || a.record.updatedAt));
    const found = candidates[0];
    if (!found) return;
    if (!state.source) state.source = { accountKey: found.provider.accountKey || '', deviceId: found.record.deviceId };
    if (state.cycle?.id === found.current.id) return;
    remember(found.current);
  }
  function ingest(record, payload) {
    const before = JSON.stringify(state);
    try { register(record.deviceId, payload.username); refresh(Object.values(store.devices)); }
    catch (error) { const old = JSON.parse(before); for (const key of Object.keys(state)) delete state[key]; Object.assign(state, old); throw error; }
    return decorate(record);
  }
  function configure(payload) {
    const before = JSON.stringify(state);
    try {
      if (!payload || typeof payload !== 'object' || Array.isArray(payload)
        || ['rename', 'assign', 'manual', 'source'].filter((key) => Object.hasOwn(payload, key)).length !== 1) throw new Error('请选择一项设置');
      if (payload.rename) {
        const member = state.members.find((entry) => entry.id === payload.rename.id);
        const name = duo.username(payload.rename.name);
        if (!member || !name) throw new Error('成员不存在或名称为空');
        if (state.members.some((entry) => entry.id !== member.id && duo.usernameKey(entry.name) === duo.usernameKey(name))) throw new Error('该用户名已存在');
        member.name = name;
      } else if (payload.assign) {
        if (!Object.hasOwn(store.devices, payload.assign.deviceId)) throw new Error('设备不存在');
        if (!duo.username(payload.assign.username)) throw new Error('用户名不能为空');
        register(payload.assign.deviceId, payload.assign.username);
      } else if (payload.manual) {
        const current = duo.cycle(payload.manual);
        if (!current) throw new Error('周期起止时间无效');
        state.manual = true; remember(current);
      } else if (payload.source) {
        const source = payload.source;
        const record = Object.hasOwn(store.devices, source.deviceId) && store.devices[source.deviceId];
        const provider = record?.limits?.providers?.find((entry) => entry.provider === 'codex'
          && /pro/i.test(entry.planLabel || '') && (entry.accountKey || '') === source.accountKey);
        if (!provider) throw new Error('请选择已经上报的 Pro 额度来源');
        state.source = { deviceId: String(source.deviceId), accountKey: source.accountKey };
        state.manual = false;
        refresh(Object.values(store.devices));
      } else throw new Error('设置内容无效');
      persist();
    } catch (error) { const previous = JSON.parse(before); for (const key of Object.keys(state)) delete state[key]; Object.assign(state, previous); throw error; }
    return snapshot();
  }
  function snapshot() {
    return JSON.parse(JSON.stringify({ version: 1, members: state.members, bindings: state.bindings, cycles: state.cycles,
      cycle: state.cycle || null, source: state.source, manual: state.manual === true }));
  }
  refresh(Object.values(store.devices));
  return { decorate, ingest, snapshot, configure };
}

module.exports = { createDuoHub };
