// Starts hoard for the browser tests: a fresh data directory and a storage
// folder of media generated with ffmpeg, so every run starts from the same
// library. Playwright runs this as its webServer and stops it afterwards.
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const bin = process.env.HOARD_BIN ?? path.resolve(here, '../../target/debug/hoard');
const port = process.env.E2E_PORT ?? '8151';

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'hoard-web-e2e-'));
const lib = path.join(dir, 'library');
fs.mkdirSync(path.join(lib, 'clips'), { recursive: true });

const ffmpeg = (...args) => execFileSync('ffmpeg', ['-nostdin', '-loglevel', 'error', '-y', ...args]);
ffmpeg(
  '-f', 'lavfi', '-i', 'testsrc=size=320x180:rate=10:duration=3',
  '-c:v', 'libx264', '-pix_fmt', 'yuv420p', path.join(lib, 'clips', 'red.mp4'),
);
ffmpeg(
  '-f', 'lavfi', '-i', 'color=c=blue:size=160x90:rate=10:duration=1',
  '-c:v', 'libx264', '-pix_fmt', 'yuv420p', path.join(lib, 'blue.mp4'),
);
ffmpeg('-f', 'lavfi', '-i', 'testsrc=size=200x100', '-frames:v', '1', path.join(lib, 'photo.png'));

const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('HOARD_')));
const child = spawn(bin, [], {
  stdio: 'inherit',
  env: {
    ...env,
    HOARD_DATA_DIR: path.join(dir, 'data'),
    HOARD_LISTEN: `127.0.0.1:${port}`,
    HOARD_ROOTS: lib,
    HOARD_YTDLP_AUTO_UPDATE: 'false',
  },
});

const stop = () => {
  child.kill();
  fs.rmSync(dir, { recursive: true, force: true });
  process.exit(0);
};
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
child.on('exit', (code) => {
  fs.rmSync(dir, { recursive: true, force: true });
  process.exit(code ?? 1);
});
