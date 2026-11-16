//! Addresses of a new project: `--addressing`, the `--hosts-name` alias and the
//! `default-addressing` setting used when neither is given.

use lampo_application::dto::AddressingDto;
use lampo_application::{AppError, AppResult, ErrorCode};

use super::Cmd;
use crate::cli::AddressingArg;

pub(super) fn to_dto(arg: AddressingArg) -> AddressingDto {
    match arg {
        AddressingArg::HostsOnly => AddressingDto::HostsOnly,
        AddressingArg::Standard => AddressingDto::Standard,
        AddressingArg::NoElevation => AddressingDto::NoElevation,
    }
}

/// `--addressing` (or `--hosts-name`, an alias of `standard`), else the `default-addressing`
/// setting, read locally.
pub(super) fn for_new_project(c: &Cmd<'_>, chosen: Option<AddressingArg>, hosts_name: bool) -> AppResult<AddressingDto> {
    match (chosen, hosts_name) {
        (Some(arg), _) => Ok(to_dto(arg)),
        (None, true) => Ok(AddressingDto::Standard),
        (None, false) => Ok(c.ctx.settings.get()?.default_addressing),
    }
}

/// `project new` refuses a hosts-file addressing without `--allow-elevation` before setting the
/// folder up, so the registration cannot fail after the clone or the installer ran.
pub(super) fn require_elevation_up_front(c: &Cmd<'_>, addressing: AddressingDto) -> AppResult<()> {
    if addressing == AddressingDto::NoElevation || c.g.allow_elevation {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::ElevationRequired,
        "the new project's addresses include a hosts-file name, which needs administrator permission: re-run with --allow-elevation or pass --addressing no-elevation (nothing was set up)",
    ))
}

#[cfg(test)]
mod tests {
    use lampo_application::context::ElevationPolicy;
    use lampo_application::control::ControlRequest;
    use lampo_application::dto::*;
    use serde_json::json;

    use crate::testing::*;

    fn sent_create(fake: &Fake) -> (CreateProjectRequest, ElevationPolicy) {
        match fake.last_request() {
            (ControlRequest::ProjectCreate(r), ctx) => (r, ctx.elevation),
            (other, _) => panic!("wrong request: {other:?}"),
        }
    }

    fn with_default(fake: &Fake, addressing: &str) {
        fake.settings.lock().unwrap()["defaultAddressing"] = json!(addressing);
    }

    #[tokio::test]
    async fn create_takes_each_addressing_mode() {
        for (mode, expected) in [("hosts-only", AddressingDto::HostsOnly), ("standard", AddressingDto::Standard), ("no-elevation", AddressingDto::NoElevation)] {
            let fake = Fake::new();
            fake.reply(Ok(project_json()));
            let (code, _, err) = run_cli(&fake, &["project", "create", "--addressing", mode, "--allow-elevation"]).await;
            assert_eq!(code, 0, "{mode}: {err}");
            let (request, elevation) = sent_create(&fake);
            assert_eq!(request.addressing, expected, "{mode}");
            assert_eq!(elevation, ElevationPolicy::Allow);
        }
    }

    #[tokio::test]
    async fn create_without_a_choice_uses_the_default_addressing_setting() {
        let fake = Fake::new();
        with_default(&fake, "hosts-only");
        fake.reply(Ok(project_json()));
        let (code, _, _) = run_cli(&fake, &["project", "create"]).await;
        assert_eq!(code, 0);
        assert_eq!(sent_create(&fake).0.addressing, AddressingDto::HostsOnly);

        fake.reply(Ok(project_json()));
        run_cli(&fake, &["project", "create", "--addressing", "no-elevation"]).await;
        assert_eq!(sent_create(&fake).0.addressing, AddressingDto::NoElevation, "the flag wins over the setting");

        fake.reply(Ok(project_json()));
        run_cli(&fake, &["project", "create", "--hosts-name"]).await;
        assert_eq!(sent_create(&fake).0.addressing, AddressingDto::Standard, "--hosts-name is an alias of standard");
    }

    #[test]
    fn hosts_name_and_addressing_cannot_be_combined_and_modes_are_checked() {
        let parse = |args: &[&str]| crate::cli::build_command().try_get_matches_from(args);
        assert!(parse(&["lampo", "project", "create", "--hosts-name", "--addressing", "hosts-only"]).is_err());
        assert!(parse(&["lampo", "project", "create", "--addressing", "everywhere"]).is_err());
        assert!(parse(&["lampo", "project", "new", "--git", "https://x/y.git", "--no-register", "--addressing", "standard"]).is_err());
    }

    #[tokio::test]
    async fn project_new_needs_elevation_for_a_hosts_file_default_before_scaffolding() {
        let fake = Fake::new();
        with_default(&fake, "hosts-only");
        let (code, out, _) = run_cli(&fake, &["project", "new", "--git", "https://example.com/shop.git", "--json"]).await;
        assert_eq!(code, 7, "{out}");
        assert!(out.contains("--allow-elevation") && out.contains("nothing was set up"), "{out}");
        assert!(fake.sent.lock().unwrap().is_empty(), "nothing was sent to the app");
    }

    #[tokio::test]
    async fn project_new_registers_with_the_chosen_addressing() {
        let fake = Fake::new();
        fake.reply(Ok(json!({ "folder": r"C:\work\shop", "phpVersion": null })));
        fake.reply(Ok(project_json()));
        let (code, _, err) = run_cli(&fake, &["project", "new", "--git", "https://example.com/shop.git", "--addressing", "hosts-only", "--allow-elevation"]).await;
        assert_eq!(code, 0, "{err}");
        assert_eq!(sent_create(&fake).0.addressing, AddressingDto::HostsOnly);
    }

    #[tokio::test]
    async fn settings_set_sends_the_default_addressing() {
        let fake = Fake::new();
        fake.reply(Ok(json!({
            "settings": settings_json(),
            "changes": [{"setting": "default-addressing", "from": "no-elevation", "to": "hosts-only"}],
            "sync": {"started": [], "stopped": [], "restarted": [], "hostsFileUpdated": false, "warnings": []},
            "notes": []
        })));
        let (code, out, _) = run_cli(&fake, &["settings", "set", "--default-addressing", "hosts-only"]).await;
        assert_eq!(code, 0);
        assert_eq!(out.trim(), "default-addressing: no-elevation -> hosts-only");
        let ControlRequest::SettingsPatch(r) = fake.last_request().0 else { panic!("wrong request") };
        assert_eq!(r, SettingsPatchRequest { default_addressing: Some(AddressingDto::HostsOnly), ..Default::default() });
    }

    #[tokio::test]
    async fn settings_get_shows_the_default_addressing() {
        let fake = Fake::new();
        with_default(&fake, "standard");
        let (code, out, _) = run_cli(&fake, &["settings"]).await;
        assert_eq!(code, 0);
        assert!(out.lines().any(|l| l.starts_with("default addressing") && l.ends_with("standard")), "{out}");
    }
}
