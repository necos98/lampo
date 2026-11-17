//! The optional Defender exclusions: the MySQL data folder and the PHP/Apache server processes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lampo_domain::package::PackageKind;
use lampo_domain::paths::LampoPaths;
use lampo_domain::ports::ScanExclusionEditor;
use lampo_domain::scan_exclusion::{ScanExclusionChange, ScanExclusionOutcome, ScanExclusionTargets};
use lampo_domain::{PortError, PortResult};

use super::{cli, gui};
use crate::context::CallContext;
use crate::dto::*;
use crate::error::ErrorCode;
use crate::testing::MemInstalled;
use crate::use_cases::DefenderExclusion;

/// Records each request and answers with a fixed result.
struct FakeEditor {
    result: PortResult<ScanExclusionOutcome>,
    calls: Mutex<Vec<(ScanExclusionTargets, ScanExclusionChange)>>,
}

impl FakeEditor {
    fn calls(&self) -> Vec<(ScanExclusionTargets, ScanExclusionChange)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ScanExclusionEditor for FakeEditor {
    async fn change(&self, targets: &ScanExclusionTargets, change: ScanExclusionChange) -> PortResult<ScanExclusionOutcome> {
        self.calls.lock().unwrap().push((targets.clone(), change));
        self.result.clone()
    }
}

struct Setup {
    editor: Arc<FakeEditor>,
    installed: Arc<MemInstalled>,
    use_case: DefenderExclusion,
}

fn setup(result: PortResult<ScanExclusionOutcome>) -> Setup {
    let editor = Arc::new(FakeEditor { result, calls: Mutex::new(Vec::new()) });
    let installed = Arc::new(MemInstalled::default());
    let use_case = DefenderExclusion::new(editor.clone(), installed.clone(), LampoPaths::new(r"C:\Users\me\AppData\Roaming\Lampo", r"C:\Users\me\AppData\Local\Lampo"));
    Setup { editor, installed, use_case }
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
    DefenderExclusionRequestDto { change, php, mysql }
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
        DefenderExclusionResultDto { change: ScanExclusionChangeDto::Add, outcome: ScanExclusionOutcomeDto::Applied, folder: None, processes: SERVERS.map(String::from).to_vec() }
    );
    s.use_case.change(request(ScanExclusionChangeDto::Remove, true, false), gui()).await.unwrap();
    let php_only = ScanExclusionTargets { folder: None, processes: servers() };
    assert_eq!(s.editor.calls(), vec![(php_only.clone(), ScanExclusionChange::Add), (php_only, ScanExclusionChange::Remove)]);
}

#[tokio::test]
async fn php_and_mysql_together_are_one_request() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    let result = s.use_case.change(request(ScanExclusionChangeDto::Add, true, true), gui()).await.unwrap();
    assert_eq!(result.folder.as_deref(), Some(MYSQL_FOLDER));
    assert_eq!(s.editor.calls(), vec![(ScanExclusionTargets { folder: Some(PathBuf::from(MYSQL_FOLDER)), processes: servers() }, ScanExclusionChange::Add)]);
}

#[tokio::test]
async fn info_lists_what_each_part_covers() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    assert_eq!(s.use_case.info().unwrap(), DefenderExclusionInfoDto { folder: MYSQL_FOLDER.into(), processes: Vec::new() });
    install_servers(&s.installed);
    assert_eq!(s.use_case.info().unwrap().processes, SERVERS.map(String::from).to_vec());
}

#[tokio::test]
async fn the_mysql_form_names_only_the_data_folder() {
    let s = setup(Ok(ScanExclusionOutcome::Applied));
    install_servers(&s.installed);
    let added = s.use_case.change_mysql(ScanExclusionChangeDto::Add, gui()).await.unwrap();
    assert_eq!(added, ScanExclusionResultDto { change: ScanExclusionChangeDto::Add, outcome: ScanExclusionOutcomeDto::Applied, folder: MYSQL_FOLDER.into() });
    let removed = s.use_case.change(request(ScanExclusionChangeDto::Remove, false, true), CallContext::cli(true)).await.unwrap();
    assert_eq!(removed.processes, Vec::<String>::new());
    let folder_only = ScanExclusionTargets { folder: Some(PathBuf::from(MYSQL_FOLDER)), processes: Vec::new() };
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
    assert!(err.message.contains("--php") && err.message.contains("--mysql"), "{}", err.message);
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

#[test]
fn the_wire_format_is_kebab_case() {
    let result = ScanExclusionResultDto { change: ScanExclusionChangeDto::Remove, outcome: ScanExclusionOutcomeDto::DefenderUnavailable, folder: MYSQL_FOLDER.into() };
    assert_eq!(serde_json::to_value(&result).unwrap(), serde_json::json!({ "change": "remove", "outcome": "defender-unavailable", "folder": MYSQL_FOLDER }));
    let result = DefenderExclusionResultDto { change: ScanExclusionChangeDto::Add, outcome: ScanExclusionOutcomeDto::NotConfirmed, folder: None, processes: vec![SERVERS[2].into()] };
    assert_eq!(serde_json::to_value(&result).unwrap(), serde_json::json!({ "change": "add", "outcome": "not-confirmed", "folder": null, "processes": [SERVERS[2]] }));
    let request: DefenderExclusionRequestDto = serde_json::from_value(serde_json::json!({ "change": "remove", "php": true, "mysql": false })).unwrap();
    assert_eq!(request, DefenderExclusionRequestDto { change: ScanExclusionChangeDto::Remove, php: true, mysql: false });
}
