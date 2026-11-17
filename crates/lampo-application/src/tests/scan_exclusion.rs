//! The optional Defender exclusions: the MySQL data folder, the PHP/Apache server processes and a
//! projects folder.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lampo_domain::address::SiteAddress;
use lampo_domain::package::PackageKind;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::{ProcessExclusionRecordWriter, ProjectRepository, ScanExclusionEditor};
use lampo_domain::project::{DisplayName, Project, ProjectEnvironment, ProjectId, ProjectRoot};
use lampo_domain::scan_exclusion::{ProcessExclusionRecord, ProcessExclusionRequest, ScanExclusionChange, ScanExclusionOutcome, ScanExclusionTargets};
use lampo_domain::hostname::Tld;
use lampo_domain::{PortError, PortResult};

use super::{cli, gui};
use crate::context::CallContext;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::{MemExclusionRecord, MemInstalled, MemProjects, php};
use crate::use_cases::DefenderExclusion;

/// Folders the fake accepts as a projects folder: two levels below `C:\Users\me` or deeper
/// (`Desktop\Progetti`, not `Desktop`: the drive and the root are components too), unless refused.
const ACCEPTED_DEPTH: usize = 6;

/// Records each request and answers with a fixed result.
struct FakeEditor {
    result: PortResult<ScanExclusionOutcome>,
    calls: Mutex<Vec<(ScanExclusionTargets, ScanExclusionChange)>>,
    checks: Mutex<Vec<(ScanExclusionTargets, ScanExclusionChange)>>,
    refused: Mutex<Vec<PathBuf>>,
}

impl FakeEditor {
    fn calls(&self) -> Vec<(ScanExclusionTargets, ScanExclusionChange)> {
        self.calls.lock().unwrap().clone()
    }

    fn refuse(&self, folder: &str) {
        self.refused.lock().unwrap().push(PathBuf::from(folder));
    }
}

#[async_trait]
impl ScanExclusionEditor for FakeEditor {
    async fn change(&self, targets: &ScanExclusionTargets, change: ScanExclusionChange) -> PortResult<ScanExclusionOutcome> {
        self.calls.lock().unwrap().push((targets.clone(), change));
        self.result.clone()
    }

    fn accepted_projects_folder(&self, folder: &Path) -> PortResult<PathBuf> {
        let accepted = folder.starts_with(r"C:\Users\me") && folder.components().count() >= ACCEPTED_DEPTH && !self.refused.lock().unwrap().iter().any(|r| r == folder);
        accepted.then(|| folder.to_path_buf()).ok_or_else(|| PortError::invalid_data(folder.display().to_string(), "refused"))
    }

    /// The targets with the adapter's spelling (lower case, to tell it from the requested one),
    /// or the fixed error; the checks are recorded, never as requests.
    fn accepted_targets(&self, targets: &ScanExclusionTargets, change: ScanExclusionChange) -> PortResult<ScanExclusionTargets> {
        self.checks.lock().unwrap().push((targets.clone(), change));
        self.result.clone()?;
        let spelled = |path: &PathBuf| PathBuf::from(path.display().to_string().to_lowercase());
        Ok(ScanExclusionTargets {
            folder: targets.folder.as_ref().map(spelled),
            processes: targets.processes.iter().map(spelled).collect(),
            projects: targets.projects.as_ref().map(spelled),
        })
    }
}

struct Setup {
    editor: Arc<FakeEditor>,
    installed: Arc<MemInstalled>,
    projects: Arc<MemProjects>,
    record: Arc<MemExclusionRecord>,
    use_case: DefenderExclusion,
}

fn setup(result: PortResult<ScanExclusionOutcome>) -> Setup {
    let editor = Arc::new(FakeEditor { result, calls: Mutex::new(Vec::new()), checks: Mutex::new(Vec::new()), refused: Mutex::new(Vec::new()) });
    let installed = Arc::new(MemInstalled::default());
    let projects = Arc::new(MemProjects::default());
    let paths = LampoPaths::new(r"C:\Users\me\AppData\Roaming\Lampo", r"C:\Users\me\AppData\Local\Lampo");
    let record = Arc::new(MemExclusionRecord::default());
    let use_case = DefenderExclusion::new(editor.clone(), installed.clone(), projects.clone(), paths, record.clone(), record.clone());
    Setup { editor, installed, projects, record, use_case }
}

/// PHP 8.3 and 7.4, Apache, and packages that run no server Lampo excludes.
fn install_servers(installed: &MemInstalled) {
    installed.add(PackageKind::PHP, "8.3.14", None);
    installed.add(PackageKind::PHP, "7.4.33", None);
    installed.add(PackageKind::APACHE, "2.4.62", None);
    installed.add(PackageKind::MYSQL, "8.4.3", None);
    installed.add(PackageKind::MOD_FCGID, "2.3.10", None);
    installed.add(PackageKind::XDEBUG, "3.5.0", Some("8.3"));
}

const MYSQL_FOLDER: &str = r"C:\Users\me\AppData\Local\Lampo\mysql";
const SERVERS: [&str; 3] = [r"C:\lampo\packages\apache-2.4\bin\httpd.exe", r"C:\lampo\packages\php-7.4\php-cgi.exe", r"C:\lampo\packages\php-8.3\php-cgi.exe"];

fn request(change: ScanExclusionChangeDto, php: bool, mysql: bool) -> DefenderExclusionRequestDto {
    DefenderExclusionRequestDto::new(change, php, mysql)
}

fn servers() -> Vec<PathBuf> {
    SERVERS.map(PathBuf::from).to_vec()
}

#[tokio::test]
async fn php_names_every_installed_php_cgi_and_httpd_in_one_request() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    let added = s.use_case.change(request(ScanExclusionChangeDto::Add, true, false), CallContext::cli(true)).await.unwrap();
    assert_eq!(
        added,
        DefenderExclusionResultDto {
            change: ScanExclusionChangeDto::Add,
            outcome: ScanExclusionOutcomeDto::Applied,
            folder: None,
            processes: SERVERS.map(String::from).to_vec(),
            projects: None
        }
    );
    s.use_case.change(request(ScanExclusionChangeDto::Remove, true, false), gui()).await.unwrap();
    let php_only = ScanExclusionTargets { processes: servers(), ..Default::default() };
    assert_eq!(s.editor.calls(), vec![(php_only.clone(), ScanExclusionChange::Add), (php_only, ScanExclusionChange::Remove)]);
}

#[tokio::test]
async fn php_and_mysql_together_are_one_request() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    let result = s.use_case.change(request(ScanExclusionChangeDto::Add, true, true), gui()).await.unwrap();
    assert_eq!(result.folder.as_deref(), Some(MYSQL_FOLDER));
    assert_eq!(s.editor.calls(), vec![(ScanExclusionTargets { folder: Some(PathBuf::from(MYSQL_FOLDER)), processes: servers(), projects: None }, ScanExclusionChange::Add)]);
}

#[tokio::test]
async fn info_lists_what_each_part_covers() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    assert_eq!(s.use_case.info().unwrap(), DefenderExclusionInfoDto { folder: MYSQL_FOLDER.into(), processes: Vec::new(), projects_folder: None, uninstalled_processes: Vec::new() });
    install_servers(&s.installed);
    assert_eq!(s.use_case.info().unwrap().processes, SERVERS.map(String::from).to_vec());
}

/// A PHP Lampo excluded and then uninstalled.
const UNINSTALLED_PHP: &str = r"C:\lampo\packages\php-8.2\php-cgi.exe";

/// The record of an applied exclusion of every installed server and of [`UNINSTALLED_PHP`].
fn excluded_with_an_uninstalled_php(record: &MemExclusionRecord) {
    let mut excluded = servers();
    excluded.push(PathBuf::from(UNINSTALLED_PHP));
    record.write(&ProcessExclusionRecord { excluded, ..Default::default() }).unwrap();
}

#[tokio::test]
async fn info_lists_the_excluded_executables_no_longer_installed() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    excluded_with_an_uninstalled_php(&s.record);
    let info = s.use_case.info().unwrap();
    assert_eq!(info.uninstalled_processes, vec![UNINSTALLED_PHP.to_string()]);
    assert_eq!(info.processes, SERVERS.map(String::from).to_vec(), "the installed ones stay in `php`");
}

#[tokio::test]
async fn info_lists_nothing_uninstalled_while_every_excluded_executable_is_installed() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    s.record.write(&ProcessExclusionRecord { excluded: servers(), ..Default::default() }).unwrap();
    assert!(s.use_case.info().unwrap().uninstalled_processes.is_empty());
}

#[tokio::test]
async fn info_without_a_readable_record_lists_nothing_uninstalled() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    excluded_with_an_uninstalled_php(&s.record);
    s.record.corrupt();
    let info = s.use_case.info().unwrap();
    assert!(info.uninstalled_processes.is_empty());
    assert_eq!(info.processes, SERVERS.map(String::from).to_vec(), "the rest is still listed");
}

#[tokio::test]
async fn removing_the_uninstalled_exclusions_empties_the_list() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    excluded_with_an_uninstalled_php(&s.record);
    s.use_case.change(DefenderExclusionRequestDto::remove_uninstalled(), gui()).await.unwrap();
    assert_eq!(s.editor.calls(), vec![(ScanExclusionTargets { processes: vec![PathBuf::from(UNINSTALLED_PHP)], ..Default::default() }, ScanExclusionChange::Remove)]);
    assert!(s.use_case.info().unwrap().uninstalled_processes.is_empty());
}

#[tokio::test]
async fn the_mysql_form_names_only_the_data_folder() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    let added = s.use_case.change_mysql(ScanExclusionChangeDto::Add, gui()).await.unwrap();
    assert_eq!(added, ScanExclusionResultDto { change: ScanExclusionChangeDto::Add, outcome: ScanExclusionOutcomeDto::Applied, folder: MYSQL_FOLDER.into() });
    let removed = s.use_case.change(request(ScanExclusionChangeDto::Remove, false, true), CallContext::cli(true)).await.unwrap();
    assert_eq!(removed.processes, Vec::<String>::new());
    let folder_only = ScanExclusionTargets { folder: Some(PathBuf::from(MYSQL_FOLDER)), ..Default::default() };
    assert_eq!(s.editor.calls(), vec![(folder_only.clone(), ScanExclusionChange::Add), (folder_only, ScanExclusionChange::Remove)]);
}

#[tokio::test]
async fn the_cli_never_prompts_without_allow_elevation() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    for change in [ScanExclusionChangeDto::Add, ScanExclusionChangeDto::Remove] {
        for (php, mysql) in [(true, false), (false, true), (true, true)] {
            let err = s.use_case.change(request(change, php, mysql), cli()).await.unwrap_err();
            assert_eq!(err.code, ErrorCode::ElevationRequired);
            assert!(err.message.contains("--allow-elevation"), "{}", err.message);
        }
        assert_eq!(s.use_case.change_mysql(change, cli()).await.unwrap_err().code, ErrorCode::ElevationRequired);
    }
    assert!(s.editor.calls().is_empty());
}

#[tokio::test]
async fn a_request_naming_nothing_is_refused() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    let err = s.use_case.change(request(ScanExclusionChangeDto::Add, false, false), gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("--php") && err.message.contains("--mysql") && err.message.contains("--projects"), "{}", err.message);
    assert!(s.editor.calls().is_empty());
}

#[tokio::test]
async fn php_without_any_server_installed_never_prompts() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    s.installed.add(PackageKind::MYSQL, "8.4.3", None);
    for (change, mysql) in [(ScanExclusionChangeDto::Add, false), (ScanExclusionChangeDto::Remove, true)] {
        assert_eq!(s.use_case.change(request(change, true, mysql), gui()).await.unwrap_err().code, ErrorCode::PackageMissing);
    }
    assert!(s.editor.calls().is_empty());
}

#[tokio::test]
async fn apache_alone_is_enough() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    s.installed.add(PackageKind::APACHE, "2.4.62", None);
    let result = s.use_case.change(request(ScanExclusionChangeDto::Add, true, false), gui()).await.unwrap();
    assert_eq!(result.processes, vec![SERVERS[0].to_string()]);
}

#[tokio::test]
async fn outcomes_the_user_can_act_on_are_results() {
    for (outcome, dto) in [
        (ScanExclusionOutcome::Cancelled, ScanExclusionOutcomeDto::Cancelled),
        (ScanExclusionOutcome::DefenderUnavailable, ScanExclusionOutcomeDto::DefenderUnavailable),
        (ScanExclusionOutcome::Refused, ScanExclusionOutcomeDto::Refused),
        (ScanExclusionOutcome::NotConfirmed, ScanExclusionOutcomeDto::NotConfirmed),
    ] {
        let s = setup(Ok(outcome));
        install_servers(&s.installed);
        assert_eq!(s.use_case.change(request(ScanExclusionChangeDto::Add, true, true), gui()).await.unwrap().outcome, dto, "{outcome:?}");
        assert_eq!(s.use_case.change_mysql(ScanExclusionChangeDto::Add, gui()).await.unwrap().outcome, dto, "{outcome:?}");
    }
}

#[tokio::test]
async fn adapter_failures_are_errors() {
    let s = setup(Err(PortError::invalid_data(MYSQL_FOLDER, "only the MySQL data folder of a Windows user profile can be excluded")));
    assert_eq!(s.use_case.change_mysql(ScanExclusionChangeDto::Add, gui()).await.unwrap_err().code, ErrorCode::Validation);
    let s = setup(Err(PortError::not_found("elevated helper", "lampo-hosts-helper.exe")));
    assert_eq!(s.use_case.change(request(ScanExclusionChangeDto::Remove, false, true), gui()).await.unwrap_err().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn every_php_request_is_remembered_with_its_outcome_and_executables() {
    for outcome in [ScanExclusionOutcome::Applied, ScanExclusionOutcome::Cancelled, ScanExclusionOutcome::Refused, ScanExclusionOutcome::NotConfirmed] {
        let s = setup(Ok(outcome));
        install_servers(&s.installed);
        s.use_case.change(request(ScanExclusionChangeDto::Add, true, false), gui()).await.unwrap();
        let record = s.record.get();
        assert_eq!(record.last_request, Some(ProcessExclusionRequest { change: ScanExclusionChange::Add, outcome, processes: servers() }), "{outcome:?}");
        assert_eq!(record.excluded, if outcome == ScanExclusionOutcome::Applied { servers() } else { Vec::new() }, "{outcome:?}");
    }
}

#[tokio::test]
async fn an_applied_removal_is_remembered() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    s.use_case.change(request(ScanExclusionChangeDto::Add, true, false), gui()).await.unwrap();
    s.use_case.change(request(ScanExclusionChangeDto::Remove, true, true), gui()).await.unwrap();
    let record = s.record.get();
    assert!(record.excluded.is_empty());
    assert_eq!(record.last_request.map(|r| r.change), Some(ScanExclusionChange::Remove));
}

#[tokio::test]
async fn requests_without_php_and_failed_requests_are_not_remembered() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    s.use_case.change_mysql(ScanExclusionChangeDto::Add, gui()).await.unwrap();
    assert_eq!(s.record.get(), ProcessExclusionRecord::default(), "mysql alone");
    let s = setup(Err(PortError::not_found("elevated helper", "lampo-hosts-helper.exe")));
    install_servers(&s.installed);
    s.use_case.change(request(ScanExclusionChangeDto::Add, true, false), gui()).await.unwrap_err();
    assert_eq!(s.record.get(), ProcessExclusionRecord::default(), "no outcome to remember");
}

#[tokio::test]
async fn a_record_that_does_not_load_starts_over() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    s.record.corrupt();
    let result = s.use_case.change(request(ScanExclusionChangeDto::Add, true, false), gui()).await.unwrap();
    assert_eq!(result.outcome, ScanExclusionOutcomeDto::Applied, "the request itself succeeded");
    assert_eq!(s.record.get().excluded, servers());
}

#[test]
fn the_wire_format_is_kebab_case() {
    let result = ScanExclusionResultDto { change: ScanExclusionChangeDto::Remove, outcome: ScanExclusionOutcomeDto::DefenderUnavailable, folder: MYSQL_FOLDER.into() };
    assert_eq!(serde_json::to_value(&result).unwrap(), serde_json::json!({ "change": "remove", "outcome": "defender-unavailable", "folder": MYSQL_FOLDER }));
    let result = DefenderExclusionResultDto {
        change: ScanExclusionChangeDto::Add,
        outcome: ScanExclusionOutcomeDto::NotConfirmed,
        folder: None,
        processes: vec![SERVERS[2].into()],
        projects: Some(PROJECTS.into()),
    };
    assert_eq!(serde_json::to_value(&result).unwrap(), serde_json::json!({ "change": "add", "outcome": "not-confirmed", "folder": null, "processes": [SERVERS[2]], "projects": PROJECTS }));
    let request: DefenderExclusionRequestDto = serde_json::from_value(serde_json::json!({ "change": "remove", "php": true, "mysql": false })).unwrap();
    assert_eq!(request, DefenderExclusionRequestDto::new(ScanExclusionChangeDto::Remove, true, false), "the projects fields are optional");
    let request: DefenderExclusionRequestDto =
        serde_json::from_value(serde_json::json!({ "change": "add", "php": false, "mysql": false, "projects": true, "projectsFolder": PROJECTS })).unwrap();
    assert_eq!(request, projects_request(ScanExclusionChangeDto::Add, Some(PROJECTS)));
}

const PROJECTS: &str = r"C:\Users\me\Desktop\Progetti";

fn projects_request(change: ScanExclusionChangeDto, folder: Option<&str>) -> DefenderExclusionRequestDto {
    DefenderExclusionRequestDto { projects: true, projects_folder: folder.map(String::from), ..DefenderExclusionRequestDto::new(change, false, false) }
}

/// A project saved at each of `roots`.
fn register_projects(projects: &MemProjects, roots: &[&str]) {
    for (i, root) in roots.iter().enumerate() {
        let id = format!("site{i}");
        let addresses = SiteAddress::defaults_for(&id, &Tld::default()).unwrap();
        let project =
            Project::new(ProjectId::new(&id).unwrap(), DisplayName::new(&id).unwrap(), ProjectRoot::new(root).unwrap(), addresses, ProjectEnvironment::minimal(php("8.3"))).unwrap();
        projects.save(&project).unwrap();
    }
}

#[tokio::test]
async fn the_suggested_projects_folder_is_the_deepest_one_the_adapter_accepts() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    assert_eq!(s.use_case.info().unwrap().projects_folder, None, "no project");
    register_projects(&s.projects, &[r"C:\Users\me\Desktop\Progetti\shop", r"C:\Users\me\Desktop\Progetti\clienti\blog"]);
    assert_eq!(s.use_case.info().unwrap().projects_folder.as_deref(), Some(PROJECTS));
    // The adapter refuses `Desktop` and the profile: nothing above the common folder is suggested.
    register_projects(&s.projects, &[r"C:\Users\me\Documents\other"]);
    assert_eq!(s.use_case.info().unwrap().projects_folder, None);
    assert!(s.editor.calls().is_empty(), "suggesting never prompts");
}

#[tokio::test]
async fn a_suggestion_walks_up_to_an_accepted_folder() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    // `Progetti\clienti` is refused by the fake (as a link would be): its parent is suggested.
    register_projects(&s.projects, &[r"C:\Users\me\Desktop\Progetti\clienti\shop"]);
    s.editor.refuse(r"C:\Users\me\Desktop\Progetti\clienti");
    assert_eq!(s.use_case.info().unwrap().projects_folder.as_deref(), Some(PROJECTS));
}

#[tokio::test]
async fn projects_names_the_suggested_or_the_given_folder() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    register_projects(&s.projects, &[r"C:\Users\me\Desktop\Progetti\shop"]);
    let added = s.use_case.change(projects_request(ScanExclusionChangeDto::Add, None), cli()).await.unwrap_err();
    assert_eq!(added.code, ErrorCode::ElevationRequired, "the CLI needs --allow-elevation");
    let added = s.use_case.change(projects_request(ScanExclusionChangeDto::Add, None), gui()).await.unwrap();
    assert_eq!(added.projects.as_deref(), Some(PROJECTS));
    assert_eq!((added.folder, added.processes), (None, Vec::new()));
    // A given folder is normalised, never checked against the projects (the adapter checks it).
    let removed = s.use_case.change(projects_request(ScanExclusionChangeDto::Remove, Some("c:/Users/me/Code/")), CallContext::cli(true)).await.unwrap();
    assert_eq!(removed.projects.as_deref(), Some(r"C:\Users\me\Code"));
    let only = |folder: &str| ScanExclusionTargets { projects: Some(PathBuf::from(folder)), ..Default::default() };
    assert_eq!(s.editor.calls(), vec![(only(PROJECTS), ScanExclusionChange::Add), (only(r"C:\Users\me\Code"), ScanExclusionChange::Remove)]);
}

#[tokio::test]
async fn projects_with_php_and_mysql_is_one_request() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    let request = DefenderExclusionRequestDto { php: true, mysql: true, ..projects_request(ScanExclusionChangeDto::Add, Some(PROJECTS)) };
    s.use_case.change(request, gui()).await.unwrap();
    let every = ScanExclusionTargets { folder: Some(PathBuf::from(MYSQL_FOLDER)), processes: servers(), projects: Some(PathBuf::from(PROJECTS)) };
    assert_eq!(s.editor.calls(), vec![(every, ScanExclusionChange::Add)]);
}

#[tokio::test]
async fn projects_without_a_folder_to_suggest_never_prompts() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    register_projects(&s.projects, &[r"C:\Users\me\Desktop\shop"]);
    let err = s.use_case.change(projects_request(ScanExclusionChangeDto::Add, None), gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.message.contains("--projects <folder>"), "{}", err.message);
    for bad in ["Progetti", r"\\?\", ""] {
        let err = s.use_case.change(projects_request(ScanExclusionChangeDto::Add, Some(bad)), gui()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Validation, "{bad:?}");
    }
    let stray = DefenderExclusionRequestDto { projects_folder: Some(PROJECTS.into()), ..DefenderExclusionRequestDto::new(ScanExclusionChangeDto::Add, false, true) };
    assert_eq!(s.use_case.change(stray, gui()).await.unwrap_err().code, ErrorCode::Validation, "a folder without `projects`");
    assert!(s.editor.calls().is_empty());
}

fn commands_request(php: bool, mysql: bool, projects: bool) -> DefenderExclusionRequestDto {
    DefenderExclusionRequestDto { projects, ..request(ScanExclusionChangeDto::Add, php, mysql) }
}

#[tokio::test]
async fn commands_name_what_adding_would_send_without_a_prompt() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    register_projects(&s.projects, &[r"C:\Users\me\Desktop\Progetti\shop"]);
    let commands = s.use_case.commands(commands_request(true, true, true)).unwrap();
    assert!(s.editor.calls().is_empty(), "nothing is changed, no prompt");

    s.use_case.change(commands_request(true, true, true), gui()).await.unwrap();
    let checks = s.editor.checks.lock().unwrap().clone();
    assert_eq!(checks, s.editor.calls(), "the targets adding sends, checked by the adapter the same way");
    let path = |p: &str| p.to_lowercase();
    let mut expected = vec![
        (DefenderExclusionKindDto::Path, path(MYSQL_FOLDER), format!("Add-MpPreference -ExclusionPath '{}'", path(MYSQL_FOLDER))),
        (DefenderExclusionKindDto::Path, path(PROJECTS), format!("Add-MpPreference -ExclusionPath '{}'", path(PROJECTS))),
    ];
    expected.extend(SERVERS.map(|exe| (DefenderExclusionKindDto::Process, path(exe), format!("Add-MpPreference -ExclusionProcess '{}'", path(exe)))));
    let commands: Vec<_> = commands.into_iter().map(|c| (c.kind, c.path, c.command)).collect();
    assert_eq!(commands, expected, "the adapter's spelling, one command per target");
}

#[tokio::test]
async fn commands_quote_the_given_projects_folder() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    let request = DefenderExclusionRequestDto { projects_folder: Some(r"c:/users/me/d'angelo progetti/".into()), ..commands_request(false, false, true) };
    let commands = s.use_case.commands(request).unwrap();
    assert_eq!(commands.iter().map(|c| c.command.as_str()).collect::<Vec<_>>(), [r"Add-MpPreference -ExclusionPath 'c:\users\me\d''angelo progetti'"]);
    assert_eq!(commands[0].path, r"c:\users\me\d'angelo progetti");
}

#[tokio::test]
async fn commands_refuse_what_adding_refuses() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    register_projects(&s.projects, &[r"C:\Users\me\Desktop\shop"]);
    assert_eq!(s.use_case.commands(commands_request(false, false, false)).unwrap_err().code, ErrorCode::Validation, "nothing named");
    assert_eq!(s.use_case.commands(commands_request(true, false, false)).unwrap_err().code, ErrorCode::PackageMissing, "no PHP or Apache");
    assert_eq!(s.use_case.commands(commands_request(false, false, true)).unwrap_err().code, ErrorCode::Validation, "no folder to suggest");
    let stray = DefenderExclusionRequestDto { projects_folder: Some(PROJECTS.into()), ..commands_request(false, true, false) };
    assert_eq!(s.use_case.commands(stray).unwrap_err().code, ErrorCode::Validation, "a folder without `projects`");

    let s = setup(Err(PortError::invalid_data(MYSQL_FOLDER, "a junction is in the way")));
    let err = s.use_case.commands(commands_request(false, true, false)).unwrap_err();
    assert_eq!((err.code, err.message.contains("junction")), (ErrorCode::Validation, true), "{}", err.message);
    assert!(s.editor.calls().is_empty());
}

#[test]
fn a_command_on_the_wire() {
    let command = DefenderExclusionCommandDto { kind: DefenderExclusionKindDto::Process, path: SERVERS[0].into(), command: format!("Add-MpPreference -ExclusionProcess '{}'", SERVERS[0]) };
    assert_eq!(serde_json::to_value(&command).unwrap(), serde_json::json!({ "kind": "process", "path": SERVERS[0], "command": command.command }));
}
