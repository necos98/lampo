# Release su GitHub (repo + CI Windows + updater)

Questa guida copre, nell'ordine: (1) come predisporre il repository GitHub, (2) come si
pubblica una release, (3) cosa fa la CI, (4) come l'updater (firmato) consuma le release. Per il
dettaglio del meccanismo di aggiornamento vedi [UPDATER.md](UPDATER.md).

## Prerequisiti

- `git` e [GitHub CLI](https://cli.github.com/) installati, `gh auth login` fatto;
- in locale: Node 24 + pnpm 11 + Rust stable (gli stessi della CI);
- la chiave di firma in `~/.tauri/lampo.key` (password in `~/.tauri/lampo.key.password`).

## 1. Repository e secret (una volta sola)

Dalla root del repository (PowerShell):

```powershell
# Crea la repo pubblica, collega il remote e pubblica il branch principale.
gh repo create necos98/lampo --public --source=. --remote=origin --push

# Secret di firma usati da build-windows.yml (mai committare chiave e password).
Get-Content "$HOME\.tauri\lampo.key" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY
Get-Content "$HOME\.tauri\lampo.key.password" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

L'endpoint dell'updater (`plugins.updater.endpoints` in `apps/desktop/src-tauri/tauri.conf.json`)
punta già a `necos98/lampo`: un fork deve cambiarlo, generare una propria coppia di chiavi e
aggiornare `pubkey` **prima** di compilare la prima release, perché entrambi sono compilati
dentro l'app.

## 2. Pubblicare una release

```powershell
# allinea la versione (Cargo.toml, Cargo.lock, apps/desktop/package.json) e tagga:
node scripts/bump-version.mjs X.Y.Z
cd apps/desktop; pnpm test; cd ../..
git add -A
git commit -m "vX.Y.Z"
git tag vX.Y.Z
git push
git push origin vX.Y.Z
```

Il push del tag fa partire `build-windows.yml`, che testa, compila, firma e crea la Release con
installer firmato, `latest.json` e checksum. Avanzamento con `gh run watch` o nella tab Actions.
Il tag deve essere `v` + la versione nei file: la CI fallisce se non coincidono.

Build locale equivalente (serve la chiave):

```powershell
cd apps/desktop
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content "$HOME\.tauri\lampo.key" -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Get-Content "$HOME\.tauri\lampo.key.password" -Raw
pnpm bundle   # -> target\release\bundle\nsis\Lampo_X.Y.Z_x64-setup.exe (+ .sig)
```

Senza la chiave `pnpm bundle` fallisce alla firma (`createUpdaterArtifacts`); `pnpm tauri build`
senza `--config src-tauri/tauri.bundle.conf.json` e `pnpm tauri dev` non la richiedono.

## 3. Cosa fa la CI

### `build-windows.yml` (tag `v*` o avvio manuale)

Runner `windows-latest`:

0. cache Rust (`Swatinem/rust-cache`, `shared-key: release`): le build su tag la ripristinano ma
   non la salvano (`save-if` solo su `main`). Le cache dei run su tag non sono visibili ad altri
   tag, quindi la cache viene scaldata da `cache-rust-main.yml` (push su `main` che tocca
   `Cargo.toml`/`Cargo.lock`/workflow, o avvio manuale): fa solo `cargo build --release
   --workspace` con un front-end segnaposto, senza Release né artifact;
1. su tag, controlla che il tag sia `v` + la versione di `apps/desktop/package.json`;
2. `pnpm install --frozen-lockfile`, `pnpm test` (test node in `scripts/`), `pnpm build`,
   `cargo test --workspace` (i test che richiedono rete o binari reali sono `#[ignore]`);
3. `pnpm tauri build --ci --config src-tauri/tauri.bundle.conf.json` con i secret di firma:
   front-end, sidecar (`scripts/prepare-sidecars.ps1`), backend in release, bundle NSIS e
   `<setup>.exe.sig`;
4. genera `latest.json` (UTF-8 senza BOM; fallisce se manca la firma) e `<setup>.exe.sha256`;
5. carica setup + `.sig` + `.sha256` + `latest.json` come artifact CI;
6. **solo su tag `v*`**: `gh release create` con gli stessi file e `--generate-notes`.

| Asset | Note |
|---|---|
| `Lampo_<ver>_x64-setup.exe` | Installer NSIS (quello scaricato dall'updater) |
| `Lampo_<ver>_x64-setup.exe.sig` | Firma minisign dell'installer |
| `Lampo_<ver>_x64-setup.exe.sha256` | Checksum per download manuali (non usato dall'updater) |
| `latest.json` | Manifest letto da `tauri-plugin-updater` |

### `ci.yml` (push su `main` e pull request)

`cargo fmt --all --check` (stile in `rustfmt.toml`), `pnpm check` (svelte-check), `pnpm test`,
`pnpm build`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

## 4. Come l'updater consuma le release

1. `check()` scarica `https://github.com/necos98/lampo/releases/latest/download/latest.json` e
   confronta `version` con quella installata;
2. se è più nuova, `download()` scarica il setup NSIS e ne verifica la firma con la `pubkey`;
3. `prepare_update` ferma i servizi, `install()` avvia l'installer in modalità `passive`; l'app
   si chiude e si riapre da sola e al riavvio aggiorna gli shim.

## 5. Note di rilascio

La CI crea la Release con `--generate-notes` (elenco dei commit/PR); le note a mano si
aggiungono dopo con `gh release edit vX.Y.Z --notes-file <file>`.

## 6. Checklist release

- [ ] secret `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` presenti su GitHub
- [ ] versione aggiornata con `node scripts/bump-version.mjs X.Y.Z` (il test «versioni
      sincronizzate» fallisce se `Cargo.toml`, `Cargo.lock` e `package.json` divergono)
- [ ] `pnpm test`, `pnpm check`, `cargo test --workspace` verdi in locale
- [ ] tag `vX.Y.Z` creato e pushato, Release apparsa su GitHub
- [ ] asset presenti: setup + `.sig` + `.sha256` + `latest.json`
- [ ] `latest.json` raggiungibile da `releases/latest/download/latest.json`
- [ ] smoke test: da un'installazione precedente con Apache e MySQL avviati, «Controlla
      aggiornamenti» trova la nuova versione, «Aggiorna e riavvia» ferma i servizi, installa,
      riapre Lampo e gli shim (`php -v` da un terminale nuovo) funzionano
