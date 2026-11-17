<p align="center">
  <img src="apps/desktop/src-tauri/icons/128x128.png" alt="Lampo" width="96" height="96">
</p>

<h1 align="center">Lampo</h1>

Lampo è un ambiente di sviluppo PHP locale per Windows: un'app desktop che scarica e gestisce
Apache, PHP, MySQL, Redis e Mailpit e serve ogni progetto al suo indirizzo, senza Docker e senza servizi di
Windows. Include la CLI `lampo`, pensata anche per i coding agent.

## Funzionalità

- **Un solo Apache** con un virtual host per progetto.
- **Qualsiasi versione di PHP dalla 5.6 in su**, scelta per progetto e scaricata da windows.php.net.
- **MySQL condiviso** con un'istanza per versione e **Redis** facoltativo.
- **Mailpit** per la posta: la posta inviata dai progetti (anche con `mail()` di PHP) resta sul
  computer e si legge nell'interfaccia web (`http://127.0.0.1:8025`, SMTP su `127.0.0.1:1025`);
  attivo di serie nei nuovi progetti Laravel, Symfony, WordPress e PHP generico.
- **Riconoscimento del progetto** (Laravel, WordPress, Symfony, Composer; da Docker Compose e
  Laravel Sail anche PHP e servizi) con preset di configurazione.
- **Indirizzi senza permessi di amministratore**: `<progetto>.localhost` e una porta dedicata su
  `127.0.0.1`. In alternativa, un nome nel file hosts (con conferma UAC).
- **Database creato in automatico** quando aggiungi un progetto che usa MySQL.
- **Un utente MySQL per progetto**: ogni nuovo progetto riceve un utente `lampo_<id>` che vede solo
  il suo database e quelli dei suoi worktree; root riceve una password casuale, cifrata con DPAPI
  per il tuo utente Windows. Dettagli e passaggio dei progetti esistenti più sotto.
- **Nuovi progetti**: clone da un repository Git o nuova applicazione Laravel con il Laravel
  Installer gestito da Lampo (usa il PHP e il Composer di Lampo, non tocca quelli globali).
- **Un ambiente per ogni git worktree**, per far lavorare più coding agent in parallelo: indirizzo
  proprio e database condiviso, nuovo o clonato.
- **CLI `lampo` per gli agent**: `lampo agent context` descrive in una sola chiamata il progetto
  della cartella corrente, output `--json` ed exit code stabili, nessun prompt interattivo.
- **Integrazione con Claude Code**, facoltativa: a ogni sessione dentro un progetto Lampo Claude
  riceve URL, PHP, database e servizi del progetto, più le skill `lampo:…` per usare la CLI.
- **Shim `.exe`** per `php`, `composer`, `mysql`, `mysqldump` e `redis-cli`, che scelgono da soli
  la versione giusta per la cartella corrente, installati nel PATH insieme alla CLI `lampo`;
  `mysql` e `mysqldump` usano già istanza e database del progetto (`mysql -e "SHOW TABLES"`).
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
già nel PATH, e l'app la aggiorna a ogni avvio dopo un aggiornamento. `lampo shims uninstall`
toglie la cartella dal PATH e cancella gli shim e la copia della CLI (**Rimuovi dal PATH** in
Impostazioni › Strumenti da riga di comando); la disinstallazione di Lampo lo fa da sé. I comandi
che cambiano stato passano dall'app, che viene avviata in background se non è già aperta.

```powershell
lampo status                          # servizi e progetti
lampo agent context --json            # tutto sul progetto della cartella corrente (per gli agent)
lampo project detect --json          # cosa rileva Lampo nella cartella, senza registrarla
lampo project create --activate       # registra la cartella corrente come progetto
lampo project new --git https://github.com/acme/shop.git --in D:\sites
lampo project new --laravel shop --in D:\sites --php 8.4
lampo project set --ext intl --addon xdebug --ini memory_limit=512M   # modifica l'ambiente
lampo project set --address api.shop.localhost --no-address 127.0.0.1:8101   # indirizzi
lampo project rename "Shop Legacy"    # nome visualizzato (l'id resta)
lampo project set --mysql-credentials project   # utente MySQL dedicato (poi aggiorna il .env)
lampo project deactivate --all        # disattiva tutti i progetti (o activate --all)
lampo project remove shop --yes       # la toglie da Lampo: cartella e database restano
lampo project import shop.lampo.toml  # importa un progetto esportato (senza direttive Apache né chiavi php.ini rischiose)
lampo package extensions              # estensioni incluse nel PHP del progetto (per --ext)
lampo package uninstall php 8.2 --yes # rimuove un pacchetto non più usato
lampo settings                        # impostazioni globali: porte, PHP predefinito
lampo settings set --apache-port 8080 --mysql-port 8.4=3310   # cambia le porte (riavvia i servizi)
lampo settings set --project-credentials off   # i nuovi progetti usano root invece di un utente proprio
lampo worktree up --db clone:shop     # ambiente per il git worktree corrente
lampo exec -- composer install        # comando con il PHP e il Composer del progetto
lampo which php                       # quale PHP (o composer, mysql...) userebbe lo shim qui, e perché
lampo open --code                     # apre il sito in VS Code (o --explorer, --terminal, --browser)
lampo logs error -f                   # log degli errori del sito, in tempo reale
lampo logs php --level errors         # solo gli errori (o --level warnings: avvisi ed errori)
lampo logs php --open-folder          # apre la cartella del log (o --open: il file)
lampo doctor                          # diagnostica (exit code 14 se trova problemi)
lampo completions powershell | Out-String | Invoke-Expression   # completamento dei comandi
```

Ogni comando accetta `--json` (tranne `exec` e `completions`) e restituisce exit code stabili,
elencati in `lampo --help` e in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#lampo-exit-codes-stable).
`lampo` non chiede mai conferme e non apre il prompt UAC senza `--allow-elevation`.
Per avere il completamento in ogni sessione aggiungi la riga di `lampo completions powershell` al
tuo `$PROFILE`; `lampo completions --help` mostra come farlo anche in bash, zsh e fish.

## Credenziali MySQL

Ogni progetto creato con Lampo riceve un utente MySQL proprio, `lampo_<id>` (per esempio
`lampo_shop`), con tutti i privilegi sul suo database e su quelli dei suoi worktree e nessun
privilegio altrove: un altro progetto, o una dipendenza Composer/npm compromessa, non legge gli
altri database. Utente e password sono nei dettagli di connessione del progetto (sezione
«Database, cache e posta» con **Copia blocco .env**, `lampo project info`, `lampo agent context`);
Lampo non scrive mai il `.env`. Gli shim `mysql` e `mysqldump` si collegano già con quell'utente.

Root riceve una password casuale, salvata cifrata con DPAPI (la decifra solo il tuo utente
Windows, su questo computer), che Lampo usa per i propri client: non va mai copiata altrove.

**Progetti esistenti.** I progetti creati prima di questa funzione restano su root senza password,
come indica il loro `.env`, e continuano a funzionare. Root riceve la password solo quando nessun
progetto di quella versione di MySQL usa più root. Per passare un progetto all'utente dedicato:

1. attiva **Utente MySQL dedicato** nella sezione «Database, cache e posta» del progetto (oppure
   `lampo project set shop --mysql-credentials project`) e salva;
2. copia nel `.env` del progetto il nuovo `DB_USERNAME` e `DB_PASSWORD`.

Quando l'ultimo progetto di un'istanza è passato all'utente dedicato, root riceve la sua password
all'avvio successivo dell'istanza (o subito, se è già avviata). L'impostazione **Utente MySQL
dedicato per i nuovi progetti** (`lampo settings set --project-credentials off`) fa usare root ai
progetti creati in seguito; un progetto lasciato su root dopo che root ha ricevuto la password usa
quella password, mostrata nei suoi dettagli di connessione.

**PHP 7.3 o precedente con MySQL 8.4.** Il PHP fino alla 7.3 non conosce `caching_sha2_password`,
l'autenticazione predefinita di MySQL 8, e fallisce con l'errore 2054 («The server requested
authentication method unknown to the client»). Per questi progetti Lampo passa l'utente con cui si
collega l'applicazione (l'utente dedicato, o root per un progetto ancora su root) a
`mysql_native_password`, mantenendo la password, e il `my.ini` generato per MySQL 8.4 attiva quel
plugin (`mysql_native_password=ON`; l'istanza ascolta solo su 127.0.0.1). Succede alla creazione, al
cambio di PHP e al cambio di servizio; tornando a PHP 7.4 o successivo l'utente dedicato torna a
`caching_sha2_password` (root resta su `mysql_native_password`). La risposta di `lampo project
create`, `lampo project set-php` e `lampo project set` mostra un `warning:` e `lampo doctor` un
avviso (`mysql-auth`); se la connessione non riesce comunque, sposta il progetto su MySQL 5.7:
`lampo project set shop --service mysql@5.7`.

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

## Esclusioni da Microsoft Defender (facoltative)

Due esclusioni facoltative, da aggiungere solo se le vuoi:

- **PHP e Apache** (`--php`): se la prima richiesta a ogni sito impiega diversi secondi (le
  successive no), la causa è di solito la scansione in tempo reale di Defender sui file PHP letti
  per la prima volta; la sua cache si azzera a ogni aggiornamento delle definizioni, a ogni
  riavvio e quando i file cambiano, quindi il ritardo si ripresenta di continuo. Lampo aggiunge
  un'**esclusione per processo** per il `php-cgi.exe` di ogni PHP installato e per `httpd.exe` di
  Apache (`%LOCALAPPDATA%\Lampo\packages\...`): Defender non analizza i file aperti da quei
  processi, per tutti i progetti ovunque si trovino, senza escludere alcuna cartella di sorgenti.
  Vale per le versioni installate al momento: dopo aver installato un'altra versione di PHP, ripeti
  `add --php`.
- **Dati MySQL** (`--mysql`): le suite di test ricreano il database a ogni esecuzione e Defender
  analizza ogni scrittura dei file di MySQL (`.ibd`, redo log). Lampo esclude la sola cartella dei
  dati MySQL, `%LOCALAPPDATA%\Lampo\mysql` (i dati di tutte le istanze).

Come:

- dalla GUI: Impostazioni › CLI e integrazioni › Microsoft Defender;
- dalla CLI: `lampo defender exclusion add --php --allow-elevation`, `--mysql`, o entrambe
  (`--php --mysql`); `remove` le toglie. `lampo mysql defender-exclusion add|remove` resta valido e
  equivale a `--mysql`.

Ogni richiesta chiede una sola conferma di amministratore (UAC), qualunque cosa includa; Lampo non
aggiunge mai un'esclusione da solo, né all'installazione né all'avvio. Viene escluso solo questo:
mai un altro programma (`php.exe`, `mysqld.exe`), un carattere jolly, il resto dei pacchetti o dei
dati, una cartella di progetto o un'estensione. Il compromesso: un file dannoso letto da PHP o
Apache, o salvato nella cartella dei dati MySQL, non verrebbe analizzato in quel momento (lo è
comunque quando viene scritto da un altro programma, copiato o eseguito).

Se Defender non è l'antivirus attivo (ne usi un altro, o è disattivato), o se le esclusioni sono
gestite da Tamper Protection o dai criteri di chi gestisce il PC (Criteri di gruppo, Intune), Lampo
lo dice (la CLI esce con codice 12; 8 se annulli il prompt UAC; 10 se con `--php` non c'è nessun
PHP o Apache installato). Senza diritti di amministratore le esclusioni di Defender non si possono
leggere, quindi Lampo mostra solo l'esito dell'ultima richiesta. Per controllarle, da un PowerShell
amministratore: `(Get-MpPreference).ExclusionProcess` e `(Get-MpPreference).ExclusionPath`. La
funzione è disponibile solo con la cartella dati standard (non con `LAMPO_HOME`).

Il disinstallatore non rimuove le esclusioni (dati e pacchetti restano su disco): toglile prima con
`lampo defender exclusion remove --php --mysql --allow-elevation` (o dalla GUI), oppure dopo, da un
PowerShell amministratore aperto con il tuo utente:
`Remove-MpPreference -ExclusionPath "$env:LOCALAPPDATA\Lampo\mysql"` e, per ogni eseguibile
elencato da `(Get-MpPreference).ExclusionProcess` sotto `Lampo\packages`,
`Remove-MpPreference -ExclusionProcess "<percorso>"`.

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
