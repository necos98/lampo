# Updater (tauri-plugin-updater)

Lampo si aggiorna con il plugin ufficiale `tauri-plugin-updater` (Tauri v2): aggiornamenti
**firmati** (minisign), installer NSIS in modalità `passive` e riavvio automatico. Prima di
installare, Lampo ferma Apache, MySQL e gli altri servizi in modo pulito.

## Come funziona

1. All'apertura della finestra (e col pulsante «Controlla aggiornamenti» in Impostazioni →
   Aggiornamenti, o dalla voce omonima del menu nell'area di notifica) la GUI chiama `check()`,
   che scarica `https://github.com/necos98/lampo/releases/latest/download/latest.json` e
   confronta la versione con quella installata. Il controllo all'avvio è silenzioso: senza rete
   o senza release non mostra nulla.
2. Se c'è una versione più nuova compare il pallino su «Impostazioni» e la sezione
   Aggiornamenti mostra versione, data e note brevi.
3. «Aggiorna e riavvia», nell'ordine:
   1. `update.download()` scarica il setup NSIS (barra di progresso) e ne verifica la **firma**
      con la chiave pubblica in `tauri.conf.json` (firma errata = aggiornamento rifiutato);
   2. il comando `prepare_update` esegue lo stesso arresto pulito di «Esci» (`Shutdown`):
      Apache per primo, poi MySQL e gli altri servizi configurati. Da quel momento la pipe di
      controllo rifiuta le richieste della CLI che cambiano stato (`app-unavailable`), così
      nessun `lampo` riavvia un servizio nel frattempo. L'app non esce e non riavvia nulla da
      sola. Se il comando fallisce l'aggiornamento prosegue comunque (il Job Object chiude i
      processi rimasti quando l'app esce) e l'errore finisce nella console;
   3. `update.install()` avvia l'installer in modalità `passive` (solo barra di avanzamento,
      nessuna domanda). Su Windows il plugin chiama subito `std::process::exit(0)`: per questo
      il passo 2 è necessario, perché l'uscita salta il `Shutdown` di «Esci»;
   4. `relaunch()` è solo il fallback se il processo è ancora vivo. Se l'installer non parte,
      `cancel_update` riapre la pipe di controllo e riporta i servizi allo stato configurato.
4. L'installer chiude `lampo-desktop.exe`; l'hook `NSIS_HOOK_PREINSTALL`
   (`apps/desktop/src-tauri/windows/installer-hooks.nsh`) termina anche `lampo.exe` e
   `lampo-hosts-helper.exe` eventualmente in esecuzione, che altrimenti bloccherebbero i file.
   L'installer riusa la cartella dell'installazione precedente (letta dal registro,
   `HKCU\Software\lampo\Lampo`): l'app resta in `%LOCALAPPDATA%\Programs\Lampo`, i dati in
   `%LOCALAPPDATA%\Lampo` non vengono toccati. A fine installazione Lampo si riapre da sola.
5. Al riavvio `Startup` registra la cartella dell'app in `%LOCALAPPDATA%\Lampo\app-location.txt`
   e aggiorna gli shim (`php.exe`, `composer.exe`, … e la copia di `lampo.exe` nella cartella
   shim): sono copie dei vecchi `lampo-shim.exe` e `lampo.exe`, quindi vengono sostituite quelle
   diverse (confronto byte per byte) e aggiunte quelle mancanti. Se gli shim non erano
   installati non fa nulla e
   non tocca il PATH. Uno shim in esecuzione viene spostato da parte e sostituito.

## File coinvolti

| File | Ruolo |
|---|---|
| `apps/desktop/src-tauri/tauri.conf.json` | `bundle.targets=["nsis"]`, `plugins.updater` (`pubkey`, `endpoints`, `windows.installMode=passive`); niente `version` (la eredita dal crate) |
| `apps/desktop/src-tauri/tauri.bundle.conf.json` | Config di release (`pnpm bundle`): sidecar, `createUpdaterArtifacts`, hook NSIS. Le build di sviluppo non richiedono la chiave |
| `apps/desktop/src-tauri/windows/installer-hooks.nsh` | Cartella di installazione `%LOCALAPPDATA%\Programs\Lampo` (mai la cartella dati); termina `lampo.exe` e `lampo-hosts-helper.exe` prima di copiare i file |
| `apps/desktop/src-tauri/capabilities/default.json` | Permessi `updater:default` e `process:allow-restart` |
| `apps/desktop/src-tauri/src/lib.rs` | Registra i plugin `updater` e `process` (dopo `single-instance`) |
| `apps/desktop/src-tauri/src/commands/app.rs` | Comandi `prepare_update` / `cancel_update` |
| `crates/lampo-application/src/use_cases/lifecycle.rs` | `Shutdown` + `ShutdownGate` (pipe chiusa alle mutazioni), `Startup` (posizione dell'app, refresh shim) |
| `crates/lampo-infra-store/src/shim_installer.rs` | `ShimInstaller::refresh` (shim e copia della CLI) |
| `crates/lampo-infra-store/src/app_location.rs` | `app-location.txt`: dove la copia della CLI trova l'app |
| `apps/desktop/src/lib/backend/ports.ts` | Porta `Updater` del `Backend` (reale in `tauri.ts`, simulata in `mock/`) |
| `apps/desktop/src/lib/stores/updater.svelte.ts` | Stato e sequenza di installazione |
| `apps/desktop/src/lib/components/settings/UpdatesPanel.svelte` | Sezione «Aggiornamenti» delle Impostazioni |
| `.github/workflows/build-windows.yml` | Build firmata + `latest.json` + Release |
| `scripts/bump-version.test.mjs` | Test: versioni sincronizzate, config updater, hook NSIS |

Con `pnpm dev:mock` l'updater è simulato: trova sempre la 0.2.0, mostra il progresso e al
«riavvio» ricarica la pagina.

## Chiave di firma

Gli aggiornamenti sono firmati con una chiave minisign generata con
`pnpm tauri signer generate -w "$HOME/.tauri/lampo.key" -p <password> --ci` (da `apps/desktop`).

- **Chiave pubblica**: in `plugins.updater.pubkey` di `tauri.conf.json` (committata, è pubblica;
  copia anche in `~/.tauri/lampo.key.pub`).
- **Chiave privata**: in locale `~/.tauri/lampo.key` (password in `~/.tauri/lampo.key.password`).
  **Non va mai committata** né copiata nel repository o nei log.
- **Secret GitHub** (Settings → Secrets and variables → Actions):
  - `TAURI_SIGNING_PRIVATE_KEY`: contenuto del file `lampo.key`;
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: contenuto di `lampo.key.password`.
- **Se la chiave privata va persa** (o la password) le installazioni esistenti non potranno più
  verificare nuovi aggiornamenti: servirebbe una nuova coppia di chiavi e un'installazione manuale
  una tantum su ogni macchina. Tenere un backup sicuro fuori dal repository.

## latest.json

Generato dalla CI (formato statico del plugin) e pubblicato come asset della Release;
l'endpoint `releases/latest/download/latest.json` punta sempre all'ultima release.

```json
{
  "version": "0.2.0",
  "notes": "Lampo v0.2.0",
  "pub_date": "2026-01-01T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contenuto di Lampo_0.2.0_x64-setup.exe.sig>",
      "url": "https://github.com/necos98/lampo/releases/download/v0.2.0/Lampo_0.2.0_x64-setup.exe"
    },
    "windows-x86_64-nsis": { "...": "stesso contenuto" }
  }
}
```

- `version` viene da `apps/desktop/package.json` (allineata a `Cargo.toml`, senza `v`); il tag
  della Release deve essere `v<version>` e la CI fallisce se non coincide.
- Il file è UTF-8 **senza BOM**; se il `.sig` manca la CI fallisce.
- Asset di ogni Release: `*-setup.exe`, `*-setup.exe.sig`, `*-setup.exe.sha256`, `latest.json`.
  Niente exe portable: `lampo-desktop.exe` da solo non ha i sidecar.

## Versione

La versione vive in `Cargo.toml` (`[workspace.package] version`, ereditata da tutti i crate) e in
`apps/desktop/package.json`; `tauri.conf.json` non ha `version` (Tauri usa quella del crate
`lampo-desktop`). Si cambia solo con `node scripts/bump-version.mjs X.Y.Z` (o `pnpm bump X.Y.Z`
da `apps/desktop`), che aggiorna anche le voci `lampo-*` di `Cargo.lock`; il test «versioni
sincronizzate» (`pnpm test`) fallisce se divergono.

## Versioni dei plugin

`tauri build` rifiuta pacchetti npm e crate Rust dei plugin con minor diverse. Il workspace ha
`rust-version = "1.85"`, quindi Cargo sceglie le versioni dei crate compatibili con Rust 1.85
(`tauri-plugin-updater` 2.12, `tauri-plugin-process` 2.3); i pacchetti npm sono fissati con `~`
alle stesse minor. Alzando `rust-version` si possono aggiornare insieme crate e pacchetti.
