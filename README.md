<p align="center">
  <img src="apps/desktop/src-tauri/icons/128x128.png" alt="Lampo" width="96" height="96">
</p>

<h1 align="center">Lampo</h1>

Lampo è un ambiente di sviluppo PHP locale per Windows: un'app desktop che scarica e gestisce
Apache, PHP, MySQL e Redis e serve ogni progetto al suo indirizzo, senza Docker e senza servizi di
Windows. Include la CLI `lampo`, pensata anche per i coding agent.

## Funzionalità

- **Un solo Apache** con un virtual host per progetto.
- **Qualsiasi versione di PHP dalla 5.6 in su**, scelta per progetto e scaricata da windows.php.net.
- **MySQL condiviso** con un'istanza per versione e **Redis** facoltativo.
- **Riconoscimento del progetto** (Laravel, WordPress, Symfony, Composer; da Docker Compose e
  Laravel Sail anche PHP e servizi) con preset di configurazione.
- **Indirizzi senza permessi di amministratore**: `<progetto>.localhost` e una porta dedicata su
  `127.0.0.1`. In alternativa, un nome nel file hosts (con conferma UAC).
- **Database creato in automatico** quando aggiungi un progetto che usa MySQL.
- **Nuovi progetti**: clone da un repository Git o nuova applicazione Laravel con il Laravel
  Installer gestito da Lampo (usa il PHP e il Composer di Lampo, non tocca quelli globali).
- **Un ambiente per ogni git worktree**, per far lavorare più coding agent in parallelo: indirizzo
  proprio e database condiviso, nuovo o clonato.
- **CLI `lampo` per gli agent**: `lampo agent context` descrive in una sola chiamata il progetto
  della cartella corrente, output `--json` ed exit code stabili, nessun prompt interattivo.
- **Integrazione con Claude Code**, facoltativa: a ogni sessione dentro un progetto Lampo Claude
  riceve URL, PHP, database e servizi del progetto, più le skill `lampo:…` per usare la CLI.
- **Shim `.exe`** per `php`, `composer`, `mysql`, `mysqldump` e `redis-cli`, che scelgono da soli
  la versione giusta per la cartella corrente, installati nel PATH insieme alla CLI `lampo`.
- **Apri il progetto** in Esplora risorse, in VS Code o in un terminale con il PHP del progetto nel
  PATH.
- **Log** di servizi e siti nell'app e **diagnostica** (`doctor`: runtime Visual C++, porte, PATH,
  file hosts, pacchetti).
- **Icona nella tray** con lo stato dei servizi, che restano attivi anche a finestra chiusa.
- **Aggiornamenti dall'app**, firmati.

## Requisiti

- Windows 10 o 11 a 64 bit.
- Git nel PATH, per clonare i progetti da un repository.
- Visual Studio Code, facoltativo, per aprire i progetti dall'app.

## Installazione

Scarica `Lampo_<versione>_x64-setup.exe` dall'[ultima release](https://github.com/necos98/lampo/releases/latest)
e avvialo. Gli aggiornamenti successivi arrivano dall'app (Impostazioni › Aggiornamenti).

L'installer non chiede permessi di amministratore e mette l'app in
`%LOCALAPPDATA%\Programs\Lampo`; gli aggiornamenti restano nella stessa cartella. I dati stanno
altrove e sopravvivono ad aggiornamenti e disinstallazione: configurazione e progetti in
`%APPDATA%\Lampo`, pacchetti, database MySQL, log e shim in `%LOCALAPPDATA%\Lampo`.

## Primi passi

1. Al primo avvio Lampo scarica Apache, mod_fcgid, PHP, MySQL e Composer, una volta sola, nella
   tua cartella utente. Se mancano dei runtime Visual C++ te lo segnala con il link per scaricarli.
2. Con **Aggiungi progetto** scegli una cartella esistente, oppure cloni un repository o crei un
   nuovo progetto Laravel. Lampo riconosce il tipo di progetto e propone PHP, servizi e indirizzo.
3. Attiva il progetto e aprilo nel browser all'indirizzo mostrato (per esempio
   `http://shop.localhost` o `http://127.0.0.1:8100`).
4. Facoltativo: in Impostazioni › Strumenti da riga di comando, **Installa nel PATH** aggiunge gli
   shim `php`, `composer`, `mysql`, `mysqldump`, `redis-cli` e la CLI `lampo`.

## CLI

`lampo.exe` si trova nella cartella di installazione, accanto all'app. **Installa nel PATH** (o
`lampo shims install`) ne mette una copia nella cartella degli shim (`%LOCALAPPDATA%\Lampo\bin`),
già nel PATH, e l'app la aggiorna a ogni avvio dopo un aggiornamento. I comandi che cambiano
stato passano dall'app, che viene avviata in background se non è già aperta.

```powershell
lampo status                          # servizi e progetti
lampo agent context --json            # tutto sul progetto della cartella corrente (per gli agent)
lampo project detect --json          # cosa rileva Lampo nella cartella, senza registrarla
lampo project create --activate       # registra la cartella corrente come progetto
lampo project new --git https://github.com/acme/shop.git --in D:\sites
lampo project new --laravel shop --in D:\sites --php 8.4
lampo project set --ext intl --addon xdebug --ini memory_limit=512M   # modifica l'ambiente
lampo package extensions              # estensioni incluse nel PHP del progetto (per --ext)
lampo package uninstall php 8.2 --yes # rimuove un pacchetto non più usato
lampo worktree up --db clone:shop     # ambiente per il git worktree corrente
lampo exec -- composer install        # comando con il PHP e il Composer del progetto
lampo logs error -f                   # log degli errori del sito, in tempo reale
lampo doctor                          # diagnostica (exit code 14 se trova problemi)
```

Ogni comando accetta `--json` (tranne `exec`) e restituisce exit code stabili, elencati in
`lampo --help` e in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#lampo-exit-codes-stable).
`lampo` non chiede mai conferme e non apre il prompt UAC senza `--allow-elevation`.

## Integrazione Claude Code

In Impostazioni › Integrazioni › Claude Code, **Installa** registra in Claude Code (solo per il
tuo utente, con `claude plugin …`) un plugin generato da Lampo in
`%LOCALAPPDATA%\Lampo\agent-plugins\claude-code`. Nei progetti non viene scritto nulla.

- All'avvio di ogni sessione un hook esegue `lampo agent context --hook`: dentro un progetto o
  un worktree di Lampo aggiunge al contesto progetto, URL, PHP, document root, database e
  servizi; altrove non aggiunge niente.
- Le skill `lampo:…` spiegano a Claude come usare la CLI `lampo`.
- Serve la CLI nel PATH (Impostazioni › Strumenti da riga di comando).
- Dopo un aggiornamento di Lampo il plugin, se l'avevi installato, si aggiorna da solo all'avvio;
  **Rimuovi** lo toglie da Claude Code. Lampo non tocca Claude Code se non lo chiedi tu, e lo
  disinstallatore toglie il plugin.

## Documentazione

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): architettura, gestione dei processi, CLI e exit
  code, risoluzione del sito, build e test end-to-end.
- [docs/RELEASE.md](docs/RELEASE.md): come si pubblica una release.
- [docs/UPDATER.md](docs/UPDATER.md): come funziona l'aggiornamento firmato.
- [docs/CLAUDE-PLUGIN.md](docs/CLAUDE-PLUGIN.md): skill del plugin Claude Code, test ed eval.

## Sviluppo

Prerequisiti: Rust >= 1.85 (toolchain MSVC), Node 24, pnpm 11.

```powershell
cd apps/desktop; pnpm install; pnpm build; cd ../..   # build/ serve al crate Tauri
cargo check --workspace
cargo test --workspace
cargo fmt --all        # stile in rustfmt.toml; la CI esegue `cargo fmt --all --check`
cd apps/desktop; pnpm check; pnpm tauri dev
```

Altri comandi (da `apps/desktop`):

```powershell
pnpm dev:mock         # solo GUI, con un backend simulato nel browser
pnpm test             # test degli script (bump di versione, configurazione del bundle)
pnpm bundle           # installer NSIS con CLI e helper inclusi
pnpm bump 0.3.0       # allinea la versione in Cargo.toml, Cargo.lock e package.json
```

Il test end-to-end (`scripts\e2e-cli.ps1`) usa l'app, la CLI e i download reali in un
`LAMPO_HOME` isolato: vedi [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#end-to-end-check).

## Licenza

Distribuito con licenza [MIT](LICENSE).
