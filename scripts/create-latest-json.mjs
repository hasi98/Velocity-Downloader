import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

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
const assetName = args.get('asset') ?? `Velocity.Downloader_${version}_x64-setup.exe`;
const signaturePath = resolve(
  args.get('signature') ?? `src-tauri/target/release/bundle/nsis/Velocity Downloader_${version}_x64-setup.exe.sig`
);

if (!existsSync(signaturePath)) {
  throw new Error(`Signature file not found: ${signaturePath}`);
}

const assetUrl = `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(assetName)}`;
const signature = readFileSync(signaturePath, 'utf8').trim();

const manifest = {
  version,
  notes: args.get('notes') ?? `Velocity Downloader ${version}`,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      signature,
      url: assetUrl,
    },
  },
};

mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Wrote ${output}`);
