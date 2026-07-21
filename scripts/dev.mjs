import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const cargoBin = path.join(process.env.HOME || '', '.cargo', 'bin');
const env = {
  ...process.env,
  PATH: [cargoBin, process.env.PATH || ''].filter(Boolean).join(path.delimiter),
};

const server = spawn(process.execPath, [path.join(rootDir, 'scripts', 'dev-server.mjs')], {
  stdio: 'inherit',
  env,
});

const tauri = spawn('npm', ['exec', '--', 'tauri', 'dev'], {
  stdio: 'inherit',
  env,
});

const stop = signal => {
  if (!server.killed) server.kill(signal);
  if (!tauri.killed) tauri.kill(signal);
};

process.on('SIGINT', () => stop('SIGINT'));
process.on('SIGTERM', () => stop('SIGTERM'));

tauri.on('exit', (code, signal) => {
  stop(signal || 'SIGTERM');
  process.exit(code ?? 1);
});

server.on('exit', code => {
  if (code && code !== 0) {
    stop('SIGTERM');
    process.exit(code);
  }
});
