'use strict';

const base = require('./electron-builder.config');

// Personal distribution keeps the native glass implementation. It has no
// upstream signing credentials, macOS Widget extension or silent updater.
module.exports = {
  ...base,
  mac: { ...base.mac, forceCodeSigning: false, identity: '-', target: ['zip'] },
  win: { ...base.win, signtoolOptions: null },
  publish: [{ provider: 'github', owner: 'mrxiuer', repo: 'token-monitor' }]
};
