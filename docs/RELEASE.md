# Release su GitHub (repo + CI Windows + updater)

Questa guida copre, nell'ordine: (1) come predisporre il repository GitHub, (2) come si
pubblica una release, (3) cosa fa la CI, (4) come l'updater (firmato) consuma le release. Per il
dettaglio del meccanismo di aggiornamento vedi [UPDATER.md](UPDATER.md).

## Prerequisiti

- `git` e [GitHub CLI](https://cli.github.com/) installati, `gh auth login` fatto;
- in locale: Node 24 + pnpm 11 + Rust nella versione di `rust-toolchain.toml` (rustup la
  installa da solo al primo `cargo` nel repository; è la stessa della CI);
- la chiave di firma in `~/.tauri/lampo.key` (password in `~/.tauri/lampo.key.password`).

## 1. Repository, Environment e secret (una volta sola)

La chiave privata firma gli aggiornamenti che l'updater installa **da solo** su ogni macchina:
chi la ottiene può distribuire codice arbitrario a tutti gli utenti. Per questo vive solo in un
Environment protetto, letto dal solo job `release` (che non compila né installa dipendenze), e
solo un maintainer può creare i tag `v*` che lo avviano. I passi seguenti sono manuali: la CI
non ha i permessi per farli.

### 1.1 Repository

Dalla root del repository (PowerShell):

```powershell
# Crea la repo pubblica, collega il remote e pubblica il branch principale.
gh repo create necos98/lampo --public --source=. --remote=origin --push
```

L'endpoint dell'updater (`plugins.updater.endpoints` in `apps/desktop/src-tauri/tauri.conf.json`)
punta già a `necos98/lampo`: un fork deve cambiarlo, generare una propria coppia di chiavi e
aggiornare `pubkey` **prima** di compilare la prima release, perché entrambi sono compilati
dentro l'app.

### 1.2 Environment `release` con reviewer

Settings → Environments → **New environment** → nome `release` (deve coincidere con
`environment: release` di `build-windows.yml`), poi:

1. **Required reviewers**: aggiungere il maintainer (o i maintainer) che approvano ogni
   pubblicazione; con un solo maintainer lasciare disattivato *Prevent self-review*, altrimenti
   nessuno può approvare;
2. **Deployment branches and tags** → *Selected branches and tags* → **Add deployment branch or
   tag rule** → *Ref type: Tag*, pattern `v*`: l'Environment (e quindi la chiave) è utilizzabile
   solo da run su un tag `v*`, anche se qualcuno modificasse il workflow su un branch;
3. salvare, poi aggiungere i secret **nell'Environment** (non tra i secret del repository):

```powershell
Get-Content "$HOME\.tauri\lampo.key" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY --env release
Get-Content "$HOME\.tauri\lampo.key.password" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --env release
```

Se esistono ancora come secret di **repository** (configurazione precedente), cancellarli: li
potrebbe leggere qualunque workflow del repository.

```powershell
gh secret list                                   # secret di repository
gh secret delete TAURI_SIGNING_PRIVATE_KEY
gh secret delete TAURI_SIGNING_PRIVATE_KEY_PASSWORD
gh secret list --env release                     # devono comparire qui
```

### 1.3 Protezione dei tag `v*`

Settings → Rules → Rulesets → **New ruleset** → *New tag ruleset*:

1. nome `release-tags`, *Enforcement status: Active*;
2. *Bypass list*: solo il ruolo **Repository admin** (o i maintainer che pubblicano);
3. *Target tags* → **Add target** → *Include by pattern* `v*`;
4. regole: **Restrict creations**, **Restrict updates**, **Restrict deletions**,
   **Block force pushes**.

Così solo chi è nella bypass list può creare un tag `v*` (e far partire la firma) e un tag già
pubblicato non può essere spostato su un altro commit.

### 1.4 Permessi predefiniti di Actions

Settings → Actions → General → *Workflow permissions*: **Read repository contents and packages
permissions** (i workflow chiedono da sé `contents: write` solo dove serve) e disattivare *Allow
GitHub Actions to create and approve pull requests*.

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

Il push del tag fa partire `build-windows.yml`: il job `build-windows` testa e compila
l'installer (non firmato), poi il job `release` resta **in attesa di approvazione**. Dalla tab
Actions (o dal link nella mail di GitHub) → *Review deployments* → `release` → *Approve and
deploy*: il job firma l'installer e crea la Release con installer, `.sig`, `latest.json` e
checksum. Avanzamento con `gh run watch`. Il tag deve essere `v` + la versione nei file: la CI
fallisce se non coincidono.

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

Tutti i workflow hanno `permissions: contents: read`; le action sono fissate per SHA completo
(commento `# vX.Y.Z` accanto) e `.github/dependabot.yml` propone ogni settimana gli
aggiornamenti di action, crate e pacchetti npm. Rust è installato con `rustup toolchain install`
dalla versione di `rust-toolchain.toml`; ogni comando cargo usa `--locked` (fallisce se
`Cargo.lock` non è aggiornato invece di riscriverlo).

### `build-windows.yml` (tag `v*` o avvio manuale)

Job **`build-windows`** (runner `windows-latest`, sola lettura, nessun secret):

0. cache Rust (`Swatinem/rust-cache`, `shared-key: release`): le build su tag la ripristinano ma
   non la salvano (`save-if` solo su `main`). Le cache dei run su tag non sono visibili ad altri
   tag, quindi la cache viene scaldata da `cache-rust-main.yml` (push su `main` che tocca
   `Cargo.toml`/`Cargo.lock`/`rust-toolchain.toml`/workflow, o avvio manuale): fa solo
   `cargo build --release --workspace --locked` con un front-end segnaposto, senza Release né
   artifact;
1. su tag, controlla che il tag sia `v` + la versione di `apps/desktop/package.json`;
2. `pnpm install --frozen-lockfile`, `pnpm test` (test node in `scripts/`), `pnpm build`,
   `cargo test --workspace --locked` (i test che richiedono rete o binari reali sono `#[ignore]`);
3. `pnpm exec tauri build --ci --config src-tauri/tauri.bundle.conf.json` con un secondo
   `--config` che spegne `createUpdaterArtifacts` e `'--' --locked` per cargo (il `--` tra
   apici, altrimenti PowerShell lo toglie passando gli argomenti a pnpm): front-end, sidecar
   (`scripts/prepare-sidecars.ps1`, anch'esso `--locked`), backend in release e bundle NSIS,
   **senza firma**;
4. genera `<setup>.exe.sha256` e carica setup + `.sha256` come artifact CI `lampo-windows`.

Job **`release`** (`needs: build-windows`, Environment `release`, `contents: write`): parte
**solo se `github.ref` è un tag `refs/tags/v*`**, anche da avvio manuale (avviato da un branch
il workflow si ferma all'artifact non firmato). Non fa checkout:

1. scarica l'artifact e ricontrolla tag e versioni;
2. firma il setup con `tauri signer sign --app-version <versione>` (`@tauri-apps/cli` alla
   versione esatta del lockfile, senza script di installazione) usando i secret
   dell'Environment → `<setup>.exe.sig`;
3. genera `latest.json` (UTF-8 senza BOM; fallisce se manca la firma);
4. `gh release create --verify-tag --generate-notes` con setup, `.sig`, `.sha256` e
   `latest.json`.

| Asset | Note |
|---|---|
| `Lampo_<ver>_x64-setup.exe` | Installer NSIS (quello scaricato dall'updater) |
| `Lampo_<ver>_x64-setup.exe.sig` | Firma minisign dell'installer |
| `Lampo_<ver>_x64-setup.exe.sha256` | Checksum per download manuali (non usato dall'updater) |
| `latest.json` | Manifest letto da `tauri-plugin-updater` |

### `ci.yml` (push su `main` e pull request)

`cargo fmt --all --check` (stile in `rustfmt.toml`), `pnpm check` (svelte-check), `pnpm test`,
`pnpm build`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked`.

### Aggiornare toolchain e action

- **Rust**: cambiare `channel` in `rust-toolchain.toml` (mai sotto `rust-version` di
  `Cargo.toml`), poi `cargo clippy --workspace --all-targets --locked -- -D warnings` e
  `cargo test --workspace --locked` in locale: le nuove versioni portano nuovi lint.
- **Action**: le PR di Dependabot aggiornano SHA e commento insieme. A mano, risolvere lo SHA
  del tag con `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha` e scrivere
  `uses: <owner>/<repo>@<sha> # <tag>`; mai un tag o un branch.

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

- [ ] Environment `release` con reviewer e regola sui tag `v*` (§1.2), secret
      `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` nell'Environment e
      **non** tra i secret del repository
- [ ] ruleset `release-tags` attivo sui tag `v*` (§1.3)
- [ ] versione aggiornata con `node scripts/bump-version.mjs X.Y.Z` (il test «versioni
      sincronizzate» fallisce se `Cargo.toml`, `Cargo.lock` e `package.json` divergono)
- [ ] `pnpm test`, `pnpm check`, `cargo test --workspace --locked` verdi in locale
- [ ] tag `vX.Y.Z` creato e pushato, job `release` approvato, Release apparsa su GitHub
- [ ] asset presenti: setup + `.sig` + `.sha256` + `latest.json`
- [ ] `latest.json` raggiungibile da `releases/latest/download/latest.json`
- [ ] smoke test: da un'installazione precedente con Apache e MySQL avviati, «Controlla
      aggiornamenti» trova la nuova versione, «Aggiorna e riavvia» ferma i servizi, installa,
      riapre Lampo e gli shim (`php -v` da un terminale nuovo) funzionano
