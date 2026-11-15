// Build and validate a self-contained SDK source distribution without publishing it.
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';

const repo = resolve(import.meta.dirname, '..');
const destination = join(repo, 'target', 'agent-sdk', `build-${Date.now()}`);
const source = join(destination, 'daily-agent-sdk');
mkdirSync(join(source, 'crates'), { recursive: true });
for (const name of ['daily-agent-core', 'daily-agent-providers']) {
  cpSync(join(repo, 'crates', name), join(source, 'crates', name), { recursive: true });
}
const manifest = readFileSync(join(repo, 'Cargo.toml'), 'utf8').replace(/^members = .*$/m, 'members = ["crates/daily-agent-core", "crates/daily-agent-providers"]');
writeFileSync(join(source, 'Cargo.toml'), manifest);
cpSync(join(repo, 'LICENSE'), join(source, 'LICENSE'));
mkdirSync(join(source, 'docs'));
cpSync(join(repo, 'docs/subscription-adapters.md'), join(source, 'docs/subscription-adapters.md'));
cpSync(join(repo, 'postman/simulations/task-runtime'), join(source, 'postman/simulations/task-runtime'), { recursive: true });
cpSync(join(repo, 'postman/mocks/task-model'), join(source, 'postman/mocks/task-model'), { recursive: true });
mkdirSync(join(source, 'scripts'));
cpSync(join(repo, 'scripts/check-agent-sdk.mjs'), join(source, 'scripts/check-agent-sdk.mjs'));
writeFileSync(join(source, 'README.md'), `# Daily Agent SDK\n\nStandalone Rust workspace containing daily-agent-core and daily-agent-providers.\n\n- cargo test --workspace\n- cargo run -p daily-agent-core --example framework\n- node scripts/check-agent-sdk.mjs\n\nSee each crate's README for embedding, limits, persistence and login features.\n`);
const env = { ...process.env, CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR ?? join(repo, 'target') };
execFileSync('cargo', ['check', '--manifest-path', join(source, 'Cargo.toml'), '--workspace', '--all-features', '--all-targets', '--offline'], { cwd: repo, env, stdio: 'inherit' });
execFileSync(process.execPath, [join(repo, 'scripts/check-agent-sdk.mjs'), '--source', source], { cwd: repo, env, stdio: 'inherit' });
execFileSync('cargo', ['run', '--manifest-path', join(source, 'Cargo.toml'), '-p', 'daily-agent-core', '--example', 'framework', '--offline', '--quiet'], { cwd: repo, env, stdio: 'inherit' });
const archive = join(destination, 'daily-agent-sdk.tar.gz');
execFileSync('tar', ['-czf', archive, '-C', dirname(source), basename(source)], { stdio: 'inherit' });
console.log(JSON.stringify({ source, archive, verified: true }));
