// Compile and run a real consumer outside the desktop workspace, with default SDK features.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const sourceIndex = process.argv.indexOf('--source');
const repo = sourceIndex >= 0 ? resolve(process.argv[sourceIndex + 1]) : resolve(import.meta.dirname, '..');
const consumer = mkdtempSync(join(tmpdir(), 'daily-agent-sdk-consumer-'));
const manifest = join(consumer, 'Cargo.toml');
const env = { ...process.env, CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR ?? join(repo, 'target') };
const cargo = (args, options = {}) => execFileSync('cargo', args, { cwd: repo, env, ...options });
let passed = false;
try {
  mkdirSync(join(consumer, 'src'));
  writeFileSync(manifest, `[package]
name = "daily-agent-sdk-consumer"
version = "0.0.0"
edition = "2021"

[dependencies]
daily-agent-core = { path = ${JSON.stringify(join(repo, 'crates/daily-agent-core'))} }
daily-agent-providers = { path = ${JSON.stringify(join(repo, 'crates/daily-agent-providers'))} }
async-trait = "0.1"
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
`);
  writeFileSync(join(consumer, 'src/main.rs'), readFileSync(join(repo, 'crates/daily-agent-core/examples/multi_agent.rs'), 'utf8') + `
// Type-check provider binding from a separate application without making an HTTP call.
const _: fn() -> std::sync::Arc<dyn daily_agent_core::model::Model> = || {
    use daily_agent_providers::{ModelConfig, ModelProviderKind, ProviderModel};
    std::sync::Arc::new(ProviderModel::with_adapter(
        Default::default(),
        ModelConfig::for_provider(ModelProviderKind::OpenAiCompatible, "http://localhost/v1", "fixture", "dummy"),
        std::sync::Arc::from(daily_agent_providers::adapter_for(ModelProviderKind::OpenAiCompatible)),
    ))
};
`);
  const metadata = JSON.parse(cargo(['metadata', '--manifest-path', manifest, '--format-version', '1', '--offline'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }));
  for (const name of ['daily-core', 'daily-desktop-runtime', 'daily-app-server', 'rusqlite']) {
    assert(!metadata.packages.some(pkg => pkg.name === name), `Standalone SDK pulled desktop dependency ${name}`);
  }
  const provider = metadata.packages.find(pkg => pkg.name === 'daily-agent-providers');
  assert(!metadata.resolve.nodes.find(node => node.id === provider.id).features.includes('login'));
  const core = metadata.packages.find(pkg => pkg.name === 'daily-agent-core');
  assert(!core.dependencies.some(dep => ['reqwest', 'daily-agent-providers', 'ts-rs'].includes(dep.name) && !dep.optional));
  const output = cargo(['run', '--manifest-path', manifest, '--offline', '--quiet'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
  assert(output.includes('Review complete. Reviewed:'));
  console.log('Standalone SDK consumer passed: delegation ran, providers compiled, desktop dependencies and login features absent.');
  passed = true;
} finally {
  if (passed) rmSync(consumer, { recursive: true, force: true });
  else console.error(`Failed SDK consumer retained at ${consumer}`);
}
