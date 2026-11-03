'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createHub } = require('../../src/hub/server');
const { createDuoHub } = require('../../src/shared/duoHub');

test('same-name registration groups devices, stable member renaming and old clients survive restart', async () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'duo-hub-'));
  const options = { port: 0, host: '127.0.0.1', secret: 'test-only', dataFile: path.join(home, 'data.json') };
  let hub = createHub(options);
  await hub.start();
  const origin = `http://127.0.0.1:${hub.server.address().port}`;
  const headers = { authorization: 'Bearer test-only', 'content-type': 'application/json' };
  async function configure(payload) {
    const response = await fetch(`${origin}/api/duo`, { method: 'PUT', headers, body: JSON.stringify(payload) });
    assert.equal(response.status, 200); return response.json();
  }
  try {
    hub.ingest({ deviceId: 'mac', username: ' Owner ', today: { totalTokens: 12 } });
    hub.ingest({ deviceId: 'windows', username: 'owner', today: { totalTokens: 24 } });
    hub.ingest({ deviceId: 'friend', username: 'Friend', today: { totalTokens: 4 } });
    const state = hub.getStats();
    assert.equal(state.duo.members.length, 2);
    assert.equal(state.devices.find((item) => item.deviceId === 'mac').memberId, state.devices.find((item) => item.deviceId === 'windows').memberId);
    const id = state.duo.bindings.mac;
    await configure({ rename: { id, name: 'New nickname' } });
    hub.ingest({ deviceId: 'mac', today: { totalTokens: 18 } });
    assert.equal(hub.getStats().devices.find((item) => item.deviceId === 'mac').memberName, 'New nickname');
    assert.equal((await fetch(`${origin}/api/duo`)).status, 401);
    await hub.stop();
    hub = createHub(options); await hub.start();
    assert.equal(hub.getStats().duo.bindings.mac, id);
    assert.equal(hub.getStats().periods.today.totalTokens, 46);
  } finally { await hub.stop(); fs.rmSync(home, { recursive: true, force: true }); }
});

test('Pro cycle pins account and device and ignores other Plus reset timestamps', () => {
  const now = Date.parse('2026-11-03T00:00:00Z');
  const pro = { deviceId: 'mac', updatedAt: new Date(now).toISOString(), limits: { providers: [{ provider: 'codex', status: 'ok', accountKey: 'pro-id', planLabel: 'Pro 100',
    windows: [{ kind: 'weekly', windowMinutes: 10080, resetsAt: '2026-11-08T12:00:00Z' }] }] } };
  const plus = { deviceId: 'friend', limits: { providers: [{ ...pro.limits.providers[0], planLabel: 'Plus', accountKey: 'plus-id',
    windows: [{ kind: 'weekly', windowMinutes: 10080, resetsAt: '2026-11-06T12:00:00Z' }] }] } };
  const store = { devices: { mac: pro, friend: plus } };
  const hub = createDuoHub(store, () => {}, () => now);
  assert.equal(hub.snapshot().cycle.end, '2026-11-08T12:00:00.000Z');
  hub.ingest(plus, plus);
  assert.equal(hub.snapshot().source.accountKey, 'pro-id');
  assert.equal(hub.snapshot().cycles.length, 1);
});

test('failed member/config persistence rolls back without replacing the live state object', () => {
  const store = { devices: { mac: { deviceId: 'mac' } } };
  let fail = false;
  const hub = createDuoHub(store, () => { if (fail) throw new Error('disk full'); });
  hub.configure({ assign: { deviceId: 'mac', username: 'first' } });
  const previous = JSON.stringify(hub.snapshot()); fail = true;
  assert.throws(() => hub.configure({ assign: { deviceId: 'mac', username: 'second' } }), /disk full/);
  assert.equal(JSON.stringify(hub.snapshot()), previous);
});

test('member snapshots are immutable baselines and forged record membership is ignored', () => {
  const store = { devices: { mac: { deviceId: 'mac' } } };
  const hub = createDuoHub(store, () => {});
  hub.configure({ assign: { deviceId: 'mac', username: 'first' } });
  const baseline = hub.snapshot();
  hub.configure({ rename: { id: baseline.members[0].id, name: 'new' } });
  assert.equal(baseline.members[0].name, 'first');
  assert.equal(hub.decorate({ deviceId: 'unknown', memberId: baseline.members[0].id }).memberId, undefined);
  assert.throws(() => hub.configure({ assign: { deviceId: 'mac', username: ' ' } }), /不能为空/);
  assert.throws(() => hub.configure({ source: { deviceId: 'mac', accountKey: 'not-pro' } }), /Pro/);
  assert.throws(() => hub.configure({}), /设置/);
});
