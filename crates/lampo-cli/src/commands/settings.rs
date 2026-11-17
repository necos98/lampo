//! `settings ...`.

use std::collections::BTreeMap;

use lampo_application::AppResult;
use lampo_application::control::ControlRequest;
use lampo_application::dto::*;

use super::{Cmd, addressing, validation};
use crate::cli::{SettingsCommand, SettingsSetArgs};
use crate::render;

pub async fn run(c: &Cmd<'_>, command: Option<SettingsCommand>) -> AppResult<i32> {
    match command.unwrap_or(SettingsCommand::Get) {
        SettingsCommand::Get => {
            let settings = c.ctx.settings.get()?;
            c.io.emit(&settings, render::settings);
        }
        SettingsCommand::Set(args) => {
            let result: SettingsPatchResultDto = c.app(ControlRequest::SettingsPatch(patch_request(args)?)).await?;
            c.io.emit(&result, render::settings_patch);
        }
    }
    Ok(0)
}

/// `settings set` flags → a [`SettingsPatchRequest`]. Only the command-line syntax
/// (`SERIES=PORT`, `START-END`) is parsed here; the app validates the values.
fn patch_request(args: SettingsSetArgs) -> AppResult<SettingsPatchRequest> {
    let mut service_ports = BTreeMap::new();
    let mut put = |service: String, port: u16| match service_ports.insert(service.clone(), port) {
        Some(_) => Err(validation(format!("the port of {service} is given twice"))),
        None => Ok(()),
    };
    for pair in &args.mysql_port {
        let (series, port) = assignment("--mysql-port", "SERIES=PORT", pair)?;
        let series = series.strip_prefix("mysql-").unwrap_or(series);
        put(format!("mysql-{series}"), port)?;
    }
    if let Some(port) = args.redis_port {
        put("redis".into(), port)?;
    }
    for pair in &args.port {
        let (service, port) = assignment("--port", "SERVICE=PORT", pair)?;
        put(service.to_ascii_lowercase(), port)?;
    }
    let mut service_ui_ports = BTreeMap::new();
    for pair in &args.ui_port {
        let (service, port) = assignment("--ui-port", "SERVICE=PORT", pair)?;
        let service = service.to_ascii_lowercase();
        if service_ui_ports.insert(service.clone(), port).is_some() {
            return Err(validation(format!("the web UI port of {service} is given twice")));
        }
    }
    let loopback_port_range = args
        .loopback_range
        .as_deref()
        .map(|range| {
            let parsed = range.split_once('-').and_then(|(start, end)| Some(PortRangeDto { start: start.trim().parse().ok()?, end: end.trim().parse().ok()? }));
            parsed.ok_or_else(|| validation(format!("--loopback-range expects START-END (`8100-8199`), got `{range}`")))
        })
        .transpose()?;
    Ok(SettingsPatchRequest {
        apache_port: args.apache_port,
        service_ports,
        service_ui_ports,
        default_php_version: args.default_php,
        loopback_port_range,
        default_addressing: args.default_addressing.map(addressing::to_dto),
        auto_restart_services: args.auto_restart,
        mysql_project_credentials: args.project_credentials,
        php_idle_timeout_secs: args.php_idle_timeout,
    })
}

/// `NAME=PORT` with a port in 1-65535.
fn assignment<'a>(flag: &str, syntax: &str, pair: &'a str) -> AppResult<(&'a str, u16)> {
    let bad = || validation(format!("{flag} expects {syntax} with a port in 1-65535, got `{pair}`"));
    let (name, port) = pair.split_once('=').ok_or_else(bad)?;
    let port: u16 = port.trim().parse().map_err(|_| bad())?;
    match (name.trim(), port) {
        ("", _) | (_, 0) => Err(bad()),
        (name, port) => Ok((name, port)),
    }
}
