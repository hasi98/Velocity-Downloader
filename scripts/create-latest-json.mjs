import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, resolve } from 'node:path';

const args = new Map(
  process.argv
    .slice(2)
    .filter(arg => arg.startsWith('--') && arg.includes('='))
    .map(arg => {
      const [key, ...value] = arg.slice(2).split('=');
      return [key, value.join('=')];
    })
);

const repo = args.get('repo') ?? 'hasi98/Velocity-Downloader';
const configPath = resolve('src-tauri/tauri.conf.json');
const tauriConfig = JSON.parse(readFileSync(configPath, 'utf8'));
const version = args.get('version') ?? tauriConfig.version;
const tag = args.get('tag') ?? `v${version}`;
const output = resolve(args.get('out') ?? 'src-tauri/target/release/bundle/latest.json');
const installerPath = resolve(
  args.get('installer') ??
    `src-tauri/target/release/bundle/nsis/Velocity Download Manager_${version}_x64-setup.exe`
);
const assetName = args.get('asset') ?? basename(installerPath);

if (!existsSync(installerPath)) {
  throw new Error(`Installer not found: ${installerPath}`);
}

const installer = readFileSync(installerPath);
const sha256 = createHash('sha256').update(installer).digest('hex');
const assetUrl = `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(assetName)}`;

const manifest = {
  version,
  notes: args.get('notes') ?? `Velocity Download Manager ${version}`,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      url: assetUrl,
      sha256,
    },
  },
};

mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Wrote ${output}`);
console.log(`sha256: ${sha256}`);
