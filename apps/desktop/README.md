# Lampo desktop app

SvelteKit (static SPA, SSR off) + Svelte 5 + TypeScript + Tailwind 4 + shadcn-svelte, hosted by
Tauri 2 (`src-tauri/`, the desktop composition root).

```powershell
pnpm install
pnpm check        # svelte-check
pnpm build        # static build into build/ (needed before `cargo check` of src-tauri)
pnpm tauri dev    # run the app (Tauri CLI is a dev dependency)
```

* `src/lib/api/` — typed mirror of every Tauri command, DTO and event. Never call `invoke` directly.
* `src/lib/messages/` — all Italian UI strings.
* `src/lib/components/ui/` — shadcn-svelte components (`pnpm dlx shadcn-svelte@latest add <name> --yes`).

SvelteKit 3 note: config lives in `vite.config.ts` (no `svelte.config.js`), and the lib alias is
`#lib` (package.json `imports`), with explicit `.js` extensions in imports.
