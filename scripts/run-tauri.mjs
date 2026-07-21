import { spawn } from 'node:child_process';
import path from 'node:path';
import process from 'node:process';

const cargoBin = path.join(process.env.HOME || '', '.cargo', 'bin');
const env = {
  ...process.env,
  PATH: [cargoBin, process.env.PATH || ''].filter(Boolean).join(path.delimiter),
};

const isWindows = process.platform === 'win32';
const npmCmd = isWindows ? 'npm.cmd' : 'npm';

const args = process.argv.slice(2);
const child = spawn(npmCmd, ['exec', '--', 'tauri', ...args], {
  stdio: 'inherit',
  env,
  shell: isWindows, // Windows necesita shell para .cmd
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});
