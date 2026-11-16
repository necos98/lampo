# Lampo desktop app

SvelteKit (static SPA, SSR off) + Svelte 5 + TypeScript + Tailwind 4 + shadcn-svelte, hosted by
Tauri 2 (`src-tauri/`, the desktop composition root).

```powershell
pnpm install
pnpm check        # svelte-check
pnpm test         # node --test: scripts/*.test.mjs and src/**/*.test.ts
pnpm build        # static build into build/ (needed before `cargo check` of src-tauri)
pnpm tauri dev    # run the app (Tauri CLI is a dev dependency)
```

* `src/lib/api/` — typed wrappers of every Tauri command and event. Never call `invoke` directly.
  The DTO types are generated from Rust into `bindings.ts` (never edit it): after changing a DTO,
  from the repository root run `cargo test -p lampo-application --lib ts_bindings -- --ignored`
  (`cargo test` fails while it is out of date). The generator uses `ts-rs` (derives on the DTOs
  only) rather than `tauri-specta`, which would rewrite the command signatures and registration.
* `src/lib/messages/` — all Italian UI strings.
* `src/lib/components/ui/` — shadcn-svelte components (`pnpm dlx shadcn-svelte@latest add <name> --yes`).

SvelteKit 3 note: config lives in `vite.config.ts` (no `svelte.config.js`), and the lib alias is
`#lib` (package.json `imports`), with explicit `.js` extensions in imports.

Unit tests (`*.test.ts`) run on Node's native TypeScript support, so the modules they cover must be
plain TypeScript without runes or runtime imports, and the tests import them with a `.ts`
extension (e.g. `src/lib/logs/`).
