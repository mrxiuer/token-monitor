'use strict';

const crypto = require('node:crypto');
const duo = require('./duo');
const { refreshQuotaCycles } = require('./duoCycles');

function createDuoHub(store, persist, now = Date.now) {
  store.duo ||= { members: [], bindings: {}, cycles: [], source: null };
  const state = store.duo;
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
  function refresh(records) { refreshQuotaCycles(state, records, now()); }
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
        || Object.keys(payload).length !== 1 || ['rename', 'assign'].filter((key) => Object.hasOwn(payload, key)).length !== 1) throw new Error('额度周期由 Pro 自动识别；请选择有效的成员设置');
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
      } else throw new Error('设置内容无效');
      persist();
    } catch (error) { const previous = JSON.parse(before); for (const key of Object.keys(state)) delete state[key]; Object.assign(state, previous); throw error; }
    return snapshot();
  }
  function snapshot() {
    return JSON.parse(JSON.stringify({ version: 2, members: state.members, bindings: state.bindings, cycles: state.cycles,
      cycle: state.cycle || null, source: state.source, manual: false,
      uncertainRanges: state.uncertainRanges || [], cycleStatus: state.pendingReset ? 'checking-reset'
        : state.uncertainRanges?.some((range) => range.status === 'pending' && (range.afterStart === state.cycle?.start || range.start === state.cycle?.end)) ? 'boundary-pending' : state.cycle ? 'confirmed' : 'waiting' }));
  }
  refresh(Object.values(store.devices));
  return { decorate, ingest, snapshot, configure };
}

module.exports = { createDuoHub };
