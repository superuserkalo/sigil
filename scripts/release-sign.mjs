#!/usr/bin/env node
// Sigil release manifests: build, sign and verify `sigil-update.json`.
//
// The updater in Sigil (rust/crates/desktop/src/updater.rs) trusts a release only when
// `sigil-update.json.sig` is an Ed25519 signature, by a key compiled into Sigil, over
//   "sigil-update-manifest v1\n" + <the exact bytes of sigil-update.json>
// The signature file is three lines:
//   sigil-update-signature v1
//   key <first 8 bytes of SHA-256(public key), hex>
//   sig <base64 of the 64-byte signature>
//
// Node's crypto (OpenSSL) does the Ed25519 work; this file adds no dependency.
//
//   node scripts/release-sign.mjs keygen [key-path]
//   node scripts/release-sign.mjs pubkey [key-path]
//   node scripts/release-sign.mjs manifest <manifest> --version V --platform P --zip Z [--assets]
//   node scripts/release-sign.mjs sign <manifest> [--key path] [--dir folder-with-zips]
//   node scripts/release-sign.mjs verify <manifest> [--pubkey base64]
//
// Windows and macOS entries go under `platforms`. Linux (`linux-x86_64`, `--zip` is the
// AppImage itself) goes under `extra_platforms`: Sigil 0.1.x refuses a manifest with an unknown
// key in `platforms` and never reads `extra_platforms`, so a release with Linux still updates it.
//
// The private key never leaves its file and is never printed.

import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const CONTEXT = 'sigil-update-manifest v1\n';
const HEADER = 'sigil-update-signature v1';
const DEFAULT_KEY = path.join(os.homedir(), '.config', 'sigil', 'update-signing.key');
const PLATFORMS = ['windows-x86_64', 'macos-aarch64', 'macos-x86_64'];
const EXTRA_PLATFORMS = ['linux-x86_64'];
// The prefix of an Ed25519 SubjectPublicKeyInfo in DER; the raw key is the 32 bytes after it.
const SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

function fail(message) {
  console.error(`release-sign: ${message}`);
  process.exit(1);
}

function option(args, name) {
  const at = args.indexOf(name);
  if (at === -1) return undefined;
  const value = args[at + 1];
  if (!value || value.startsWith('--')) fail(`${name} needs a value`);
  args.splice(at, 2);
  return value;
}

function flag(args, name) {
  const at = args.indexOf(name);
  if (at === -1) return false;
  args.splice(at, 1);
  return true;
}

function rawPublicKey(publicKey) {
  const der = publicKey.export({ format: 'der', type: 'spki' });
  if (der.length !== 44 || !der.subarray(0, 12).equals(SPKI_PREFIX)) fail('not an Ed25519 key');
  return der.subarray(12);
}

function keyId(raw) {
  return crypto.createHash('sha256').update(raw).digest().subarray(0, 8).toString('hex');
}

function loadPrivateKey(keyPath) {
  let stat;
  try {
    stat = fs.statSync(keyPath);
  } catch {
    fail(`no signing key at ${keyPath}. See docs/runbooks/releasing.md.`);
  }
  if (process.platform !== 'win32' && (stat.mode & 0o077) !== 0) {
    fail(`${keyPath} is readable by other users. Run: chmod 600 "${keyPath}"`);
  }
  const key = crypto.createPrivateKey(fs.readFileSync(keyPath));
  if (key.asymmetricKeyType !== 'ed25519') fail(`${keyPath} is not an Ed25519 key`);
  return key;
}

function keygen(args) {
  const keyPath = args[0] ?? DEFAULT_KEY;
  if (fs.existsSync(keyPath)) fail(`${keyPath} exists. Refusing to replace a signing key.`);
  fs.mkdirSync(path.dirname(keyPath), { recursive: true, mode: 0o700 });
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519');
  const pem = privateKey.export({ format: 'pem', type: 'pkcs8' });
  // `wx` refuses to follow or replace an existing file; the mode applies at creation.
  fs.writeFileSync(keyPath, pem, { mode: 0o600, flag: 'wx' });
  fs.chmodSync(keyPath, 0o600);
  const raw = rawPublicKey(publicKey);
  console.log(`private key: ${keyPath} (mode 600, not printed)`);
  console.log(`public key:  ${raw.toString('base64')}`);
  console.log(`key id:      ${keyId(raw)}`);
}

function pubkey(args) {
  const key = loadPrivateKey(args[0] ?? DEFAULT_KEY);
  const raw = rawPublicKey(crypto.createPublicKey(key));
  console.log(`public key: ${raw.toString('base64')}`);
  console.log(`key id:     ${keyId(raw)}`);
}

function sha256File(file) {
  const hash = crypto.createHash('sha256');
  const fd = fs.openSync(file, 'r');
  const buffer = Buffer.alloc(1 << 20);
  try {
    for (;;) {
      const n = fs.readSync(fd, buffer, 0, buffer.length, null);
      if (n === 0) break;
      hash.update(buffer.subarray(0, n));
    }
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest('hex');
}

function checkManifest(manifest) {
  if (manifest.format !== 1) fail('manifest format must be 1');
  if (manifest.product !== 'sigil') fail('manifest product must be "sigil"');
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(manifest.version ?? '')) {
    fail(`manifest version "${manifest.version}" is not x.y.z or x.y.z-pre`);
  }
  const platforms = Object.entries(manifest.platforms ?? {});
  // Older builds refuse a manifest without `platforms`: a Linux-only release cannot be signed.
  if (platforms.length === 0) fail('manifest lists no Windows or macOS platform');
  const extra = Object.entries(manifest.extra_platforms ?? {});
  const tagged = [
    ...platforms.map(([platform, entry]) => [platform, entry, PLATFORMS]),
    ...extra.map(([platform, entry]) => [platform, entry, EXTRA_PLATFORMS]),
  ];
  for (const [platform, entry, allowed] of tagged) {
    if (!allowed.includes(platform)) fail(`unknown platform ${platform}`);
    const suffix = platform.startsWith('linux') ? 'AppImage' : 'zip';
    if (!new RegExp(`^[A-Za-z0-9_-][A-Za-z0-9._-]*\\.${suffix}$`).test(entry.asset ?? '')) {
      fail(`${platform}: bad asset name`);
    }
    if (!Number.isSafeInteger(entry.size) || entry.size <= 0) fail(`${platform}: bad size`);
    if (!/^[0-9a-f]{64}$/.test(entry.sha256 ?? '')) fail(`${platform}: bad sha256`);
  }
}

// Add or replace one platform's entry. Creates the manifest when it is missing.
function manifest(args) {
  const file = args.shift();
  const version = option(args, '--version');
  const platform = option(args, '--platform');
  const zip = option(args, '--zip');
  const assets = flag(args, '--assets');
  if (!file || !version || !platform || !zip) {
    fail('usage: manifest <manifest> --version V --platform P --zip Z [--assets]');
  }
  const extra = EXTRA_PLATFORMS.includes(platform);
  if (!PLATFORMS.includes(platform) && !extra) {
    fail(`platform must be one of ${[...PLATFORMS, ...EXTRA_PLATFORMS].join(', ')}`);
  }
  const current = fs.existsSync(file)
    ? JSON.parse(fs.readFileSync(file, 'utf8'))
    : { format: 1, product: 'sigil', version, platforms: {} };
  if (current.version !== version) fail(`${file} is for ${current.version}, not ${version}`);
  current.published = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');
  const section = extra ? (current.extra_platforms ??= {}) : current.platforms;
  section[platform] = {
    asset: path.basename(zip),
    size: fs.statSync(zip).size,
    sha256: sha256File(zip),
    ...(assets ? { assets: true } : {}),
  };
  // A Linux build made before the Windows one: the manifest is written, but it cannot be signed
  // until a Windows or macOS entry joins it (checkManifest in sign).
  if (Object.keys(current.platforms).length > 0) checkManifest(current);
  fs.writeFileSync(file, `${JSON.stringify(current, null, 2)}\n`);
  if (fs.existsSync(`${file}.sig`)) {
    fs.rmSync(`${file}.sig`);
    console.log(`removed the old ${file}.sig: sign again`);
  }
  console.log(`${file}: ${platform} -> ${section[platform].asset}`);
}

function sign(args) {
  const file = args.shift();
  const keyPath = option(args, '--key') ?? process.env.SIGIL_SIGNING_KEY ?? DEFAULT_KEY;
  const dir = option(args, '--dir');
  if (!file) fail('usage: sign <manifest> [--key path] [--dir folder-with-zips]');
  const bytes = fs.readFileSync(file);
  const parsed = JSON.parse(bytes.toString('utf8'));
  checkManifest(parsed);
  if (dir) {
    for (const [platform, entry] of [
      ...Object.entries(parsed.platforms),
      ...Object.entries(parsed.extra_platforms ?? {}),
    ]) {
      const zip = path.join(dir, entry.asset);
      if (!fs.existsSync(zip)) fail(`${platform}: ${zip} is missing`);
      if (fs.statSync(zip).size !== entry.size) fail(`${platform}: size does not match ${zip}`);
      if (sha256File(zip) !== entry.sha256) fail(`${platform}: sha256 does not match ${zip}`);
      console.log(`checked ${entry.asset}`);
    }
  }
  const key = loadPrivateKey(keyPath);
  const raw = rawPublicKey(crypto.createPublicKey(key));
  const message = Buffer.concat([Buffer.from(CONTEXT), bytes]);
  const signature = crypto.sign(null, message, key);
  if (!crypto.verify(null, message, crypto.createPublicKey(key), signature)) fail('self-check failed');
  const out = `${HEADER}\nkey ${keyId(raw)}\nsig ${signature.toString('base64')}\n`;
  fs.writeFileSync(`${file}.sig`, out);
  console.log(`signed ${file} for Sigil ${parsed.version} with key ${keyId(raw)}`);
  console.log(`wrote ${file}.sig`);
}

function verify(args) {
  const file = args.shift();
  const given = option(args, '--pubkey');
  if (!file) fail('usage: verify <manifest> [--pubkey base64]');
  let raw;
  if (given) {
    raw = Buffer.from(given, 'base64');
  } else {
    // The key compiled into Sigil.
    const source = fs.readFileSync(
      new URL('../rust/crates/desktop/src/updater.rs', import.meta.url),
      'utf8',
    );
    const match = source.match(/UPDATE_PUBLIC_KEY: &str = "([A-Za-z0-9+/=]+)"/);
    if (!match) fail('could not read UPDATE_PUBLIC_KEY from updater.rs');
    raw = Buffer.from(match[1], 'base64');
  }
  if (raw.length !== 32) fail('a public key is 32 bytes');
  const lines = fs.readFileSync(`${file}.sig`, 'utf8').trim().split('\n');
  if (lines[0] !== HEADER) fail('not a sigil signature file');
  const id = lines[1]?.replace(/^key /, '');
  if (id !== keyId(raw)) fail(`signed by key ${id}, not ${keyId(raw)}`);
  const signature = Buffer.from(lines[2]?.replace(/^sig /, '') ?? '', 'base64');
  const publicKey = crypto.createPublicKey({
    key: Buffer.concat([SPKI_PREFIX, raw]),
    format: 'der',
    type: 'spki',
  });
  const message = Buffer.concat([Buffer.from(CONTEXT), fs.readFileSync(file)]);
  if (!crypto.verify(null, message, publicKey, signature)) fail('signature does not verify');
  checkManifest(JSON.parse(fs.readFileSync(file, 'utf8')));
  console.log(`ok: ${file} is signed by ${id}`);
}

const [command, ...rest] = process.argv.slice(2);
const commands = { keygen, pubkey, manifest, sign, verify };
if (!commands[command]) {
  fail('usage: release-sign.mjs keygen|pubkey|manifest|sign|verify ... (see the header)');
}
commands[command](rest);
