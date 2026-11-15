# Plugin Claude Code: skill, icona, test ed eval

Guida per chi mantiene il plugin `lampo` di Claude Code. Il meccanismo di generazione e
registrazione è descritto in [ARCHITECTURE.md](ARCHITECTURE.md#claude-code-integration); qui
si parla dei contenuti e di come verificarli.

## Struttura nel repository

```text
claude-plugin/
  .claude-plugin/plugin.json   manifest di sviluppo, solo per `claude plugin eval` (non distribuito)
  icon.png                     icona del plugin (512 px, l'icona dell'app)
  skills/<id>/SKILL.md         skill, più eventuali file di riferimento accanto
  evals/<caso>/                casi di eval (prompt.md + graders/*.md), solo nel repository
  evals/results/               risultati delle esecuzioni (ignorati da git)
```

- `crates/lampo-application/build.rs` incorpora **tutti** i file sotto `claude-plugin/skills/`;
  `claude-plugin/icon.png` è incorporata con `include_bytes!` (`embedded_icon`). L'app li
  scrive nel plugin generato in `%LOCALAPPDATA%\Lampo\agent-plugins\claude-code\plugins\lampo\` (l'icona in
  `.claude-plugin/icon.png`, dove la cerca `claude plugin validate`).
- Gli eval e il manifest di sviluppo **non** finiscono nel plugin generato: non cambiano
  l'hash di versione e non arrivano agli utenti.
- Ogni modifica a skill o icona cambia la versione `<app>-h<hash>`, quindi all'avvio successivo
  l'app aggiorna il plugin installato.

## Le skill

Tutte in inglese (descrizioni comprese), invocate come `lampo:<id>`:

| Skill | Contenuto |
|-------|-----------|
| `lampo` | Orientamento (`lampo agent context --json`), risoluzione del sito, `--json`, exit code e cosa fare per ciascuno, regole di autonomia, indice delle altre skill |
| `php` | Shim, `lampo exec`/`lampo env`, versioni di PHP, estensioni e componenti aggiuntivi (Xdebug, php-redis), valori php.ini, cambio versione |
| `database` | Credenziali, `lampo db list/create/import/export/drop`, `.env`, database per i test, modalità dei worktree, regole di sicurezza |
| `worktrees` | `lampo worktree up/down/list/prune`, modalità shared/fresh/clone, flusso consigliato con più agent |
| `services-logs` | Servizi, log, `lampo doctor`, playbook (500, 404/rewrite, porte occupate, MySQL, PHP sbagliato, estensioni) |
| `new-project` | `project create`, `project new --git/--laravel`, preset, indirizzi, attivazione |
| `laravel` | `.env` per MySQL/Redis di Lampo, migrazioni, queue e scheduler da terminale, test su database dedicato, Vite, Xdebug, errori comuni |
| `migrate` | Procedura di migrazione da Docker/Sail, Laragon, XAMPP/WAMP, Herd (file di riferimento `docker.md`, `laragon.md`, `xampp-wamp.md`, `herd.md`) |

Regole per chi scrive le skill:

- Integrazione "fantasma": le skill non devono mai far creare `AGENTS.md`, `CLAUDE.md`,
  `.claude/settings.json` o altri file di Lampo nei progetti.
- Le regole di autonomia (cosa l'agent fa da solo, cosa chiede prima) stanno nella skill
  `lampo` e si ripetono dove servono (database, worktree, php, migrate).
- Solo comandi e flag reali: il test sotto lo verifica.
- `SKILL.md` sotto le 250 righe, `description` su una riga (100–1024 caratteri) che dica cosa fa
  e quando usarla, con frasi tipiche dell'utente; il materiale lungo va in file accanto,
  collegati con link relativi.

## Test deterministici

```powershell
cargo test -p lampo-cli --test skills_cli
cargo test -p lampo-application claude_code
cargo test -p lampo-infra-services --test claude_validate -- --ignored --nocapture
```

- `skills_cli` estrae ogni invocazione `lampo …` dai blocchi di codice e dagli span inline di
  tutti i `.md` delle skill e la fa analizzare alla definizione clap reale
  (`lampo_cli::cli::build_command`): sottocomandi, flag e valori enumerati devono esistere.
  I segnaposto `<nome>` diventano un valore fittizio. In uno span inline si può citare un
  comando senza i suoi argomenti obbligatori (`lampo db import` in una frase); in un blocco di
  codice la riga deve essere completa. Controlla anche frontmatter, lunghezza e link.
- `claude_validate` (ignorato di default, serve `claude` installato) genera il plugin con le
  skill e l'icona vere ed esegue `claude plugin validate` su plugin e marketplace; fallisce anche
  se ricompare l'avviso `ICON_MISSING`. Non installa nulla.

## Eval di attivazione

Gli eval in `claude-plugin/evals/` misurano solo se la skill giusta si **attiva**: ogni caso ha
un `prompt.md` (frase dell'utente in italiano o in inglese, strumenti in sola lettura
`[Read, Glob, Grep, Skill]`) e un grader `tool_used` sullo strumento `Skill` con `input_match`
sul nome della skill (`lampo:migrate`...). Due casi per skill: `<skill>-it` e `<skill>-en`.

Costano: ogni esecuzione è una sessione Claude vera, con le proprie credenziali (3 run per caso
di default, più il braccio di confronto senza plugin). Non girano in CI. Per eseguirli, dalla
radice del repository:

```powershell
claude plugin eval claude-plugin --no-publish --max-cost-usd 3 --runs 1
claude plugin eval claude-plugin --no-publish --max-cost-usd 1 --case "migrate-*"
```

- Il target `claude-plugin` carica il manifest di sviluppo (plugin `lampo`), le skill da
  `claude-plugin/skills` e i casi da `claude-plugin/evals`; i risultati vanno in
  `claude-plugin/evals/results/` (ignorata da git). Il primo avvio chiede di confermare la
  fiducia nella cartella (`--trust-plugin` per saltare la domanda).
- `--max-cost-usd` è il tetto di spesa: la corsa si ferma (exit 2) quando lo raggiunge.
  `--runs 1` riduce il costo; `--case <glob>` o `--json <file>` per filtrare e salvare.
- Si può valutare anche il plugin generato e già installato (`lampo@lampo`), ma i casi devono
  stare sotto la cartella del plugin: per questo si usa il manifest di sviluppo.
- L'hook `SessionStart` non è nel manifest di sviluppo: gli eval misurano le skill da sole,
  senza il contesto di progetto che l'hook aggiunge nelle sessioni reali.

## Rilevamento dei progetti migrati

`lampo project create` e la schermata "Aggiungi progetto" usano anche il rilevatore Docker
(`crates/lampo-infra-store/src/detectors/docker.rs`): legge `compose.yaml`,
`docker-compose.yml`, i `Dockerfile` (anche in `docker/` e `.docker/`) e i runtime di Laravel
Sail. È un indizio "di ambiente" (`ENVIRONMENT_ONLY`): non sceglie il tipo né il preset, ma si
unisce a quello vincente (`combine_hints`) con il PHP dell'immagine, i servizi (`mysql`,
`redis`) e gli avvisi (PostgreSQL non supportato, MariaDB, Mailpit, motori di ricerca). La
creazione aggiunge i servizi rilevati non versionati che il preset non ha (Redis). Laragon,
XAMPP e Herd non lasciano marcatori affidabili nella cartella del progetto: li copre la skill
`lampo:migrate`.
