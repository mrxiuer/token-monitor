'use strict';

const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const app = require('../package.json');
const config = require('../desktop/src-tauri/tauri.conf.json');
const prices = require('../desktop/pricing-catalog.json');
if (app.version !== config.version) throw new Error('Desktop version differs from package.json');
const manifest = fs.readFileSync(path.join(root, 'desktop/src-tauri/Cargo.toml'), 'utf8');
if (!manifest.includes(`version = "${app.version}"`)) throw new Error('Cargo version differs from package.json');
if (!prices.sourceSha256 || !Object.keys(prices.rates || {}).length) throw new Error('Bundled pricing catalog is missing');
for (const icon of config.bundle.icon) if (!fs.existsSync(path.join(root, 'desktop/src-tauri', icon))) throw new Error(`Missing app icon: ${icon}`);
process.stdout.write('Rust desktop sources and pricing snapshot are ready.\n');
