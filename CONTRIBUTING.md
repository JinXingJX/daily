# Contributing to Daily

Thanks for your interest! Bug reports, docs fixes, and pull requests are welcome. For larger changes, please open an issue first so we can agree on the approach.

## Setup

Follow the [quick start](README.md#quick-start), then make sure everything passes:

```bash
npm test                                              # Rust tests, protocol check, renderer + Electron tests, build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

`AGENTS.md` describes the code layout and conventions in more detail (it is also what coding agents read).

## Making changes

- **Keep Rust the source of truth for wire types.** Add serialized types with the `cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))` derive and run `npm run gen:protocol`. Never edit `src/generated/`, `electron/shared/commands.generated.ts`, or `packages/plugin-sdk/src/methods.generated.ts` by hand.
- **New desktop command:** declare it in `desktop_commands!` (`crates/daily-desktop-runtime/src/runtime/mod.rs`) and keep the handler thin; put desktop logic in `daily-desktop-runtime` and reusable agent execution in `daily-agent-core`.
- **New plugin RPC method:** see [docs/plugin-development.md](docs/plugin-development.md#adding-a-host-api-method).
- **UI text:** write English in `t("…")` and add the Chinese translation to `src/i18n/zh-CN.ts`. Rust messages are English; runtime text shown in the UI needs a template in `packages/plugin-sdk/src/hostText.ts`.
- **Security-sensitive code** (paths, network, IPC, plugin frames, settings scopes) needs a regression test at the lowest layer that enforces the rule. See [SECURITY.md](SECURITY.md).
- **Tests:** Rust unit tests live next to the code; renderer and Electron tests use Vitest (`*.test.ts(x)`). Add a focused test for every bug fix.

## Style

- TypeScript: strict mode, two-space indentation, React function components, `PascalCase` components and types, `useSomething` hooks, `@/` imports in the renderer, `cn()` for class names, primitives from `src/components/ui/`.
- Rust: `rustfmt` defaults and warning-free Clippy.
- Comments explain why, not what.

## Commits and pull requests

- Use short imperative commit subjects, optionally prefixed: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`.
- A pull request should describe the behavior change, list the commands you ran, link related issues, and include screenshots or a recording for UI changes.
- CI must pass before review.

By contributing you agree that your contributions are licensed under the [MIT License](LICENSE) and that you follow the [Code of Conduct](CODE_OF_CONDUCT.md).
