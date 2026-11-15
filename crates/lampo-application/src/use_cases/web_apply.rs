//! Applying the Apache configuration so that one broken site never takes the others down: the
//! vhosts whose custom directives the policy refuses are left out up front, and when Apache
//! rejects the rest the broken vhosts are isolated by bisection (a few `httpd -t` runs for one
//! broken site among many, instead of one run per site).

use std::ops::Range;

use lampo_domain::PortError;
use lampo_domain::ports::WebServerConfigurator;
use lampo_domain::web::WebServerConfig;

use super::web_state::WebOutcome;
use crate::error::{AppError, ErrorCode};

/// A site left out of the configuration, and why (`httpd -t` output or the policy's reason).
type Exclusion = (String, String);

/// Applies `desired`, leaving out what is broken; never touches a running Apache.
pub(crate) async fn apply_web_config(configurator: &dyn WebServerConfigurator, desired: &WebServerConfig) -> WebOutcome {
    let (allowed, mut excluded) = leave_out_refused_directives(desired);
    let output = match configurator.apply(&allowed).await {
        Ok(()) => return WebOutcome::Applied { exclusion: exclusion_error(&excluded) },
        Err(PortError::ConfigRejected { output }) => output,
        Err(e) => return WebOutcome::Failed(e.into()),
    };
    let rejected = || WebOutcome::Rejected(PortError::ConfigRejected { output: output.clone() }.into());
    let mut base = allowed.clone();
    base.vhosts.clear();
    match configurator.apply(&base).await {
        Ok(()) => {}
        // Rejected without any vhost: the problem is global, no site is to blame.
        Err(PortError::ConfigRejected { .. }) => return rejected(),
        Err(e) => return WebOutcome::Failed(e.into()),
    }
    let isolation = match isolate_rejected(configurator, &allowed, output.clone()).await {
        Ok(isolation) => isolation,
        Err(e) => return WebOutcome::Failed(e.into()),
    };
    if isolation.rejected.is_empty() {
        // Every part is fine on its own: the problem is a combination of sites.
        return rejected();
    }
    excluded.extend(isolation.rejected.into_iter().map(|(i, output)| (allowed.vhosts[i].site.to_string(), output)));
    let mut good = base;
    good.vhosts = isolation.accepted.into_iter().map(|i| allowed.vhosts[i].clone()).collect();
    match configurator.apply(&good).await {
        Ok(()) => WebOutcome::Applied { exclusion: exclusion_error(&excluded) },
        Err(e) => WebOutcome::from_port_error(e),
    }
}

/// The vhosts (indices into `config.vhosts`, ascending) Apache accepts, and the ones it rejects
/// on their own with the rejection output.
#[derive(Default)]
struct Isolation {
    accepted: Vec<usize>,
    rejected: Vec<(usize, String)>,
}

/// Bisects `config.vhosts`, which Apache rejected together with `output`: each rejected group is
/// split in two halves validated on their own, down to the single broken vhosts. A group whose
/// halves are both accepted is accepted (the final apply reports the combination).
async fn isolate_rejected(configurator: &dyn WebServerConfigurator, config: &WebServerConfig, output: String) -> Result<Isolation, PortError> {
    let mut isolation = Isolation::default();
    let mut rejected_groups: Vec<(Range<usize>, String)> = vec![(0..config.vhosts.len(), output)];
    while let Some((group, output)) = rejected_groups.pop() {
        if group.len() == 1 {
            isolation.rejected.push((group.start, output));
            continue;
        }
        let middle = group.start + group.len() / 2;
        for half in [group.start..middle, middle..group.end] {
            let mut probe = config.clone();
            probe.vhosts = config.vhosts[half.clone()].to_vec();
            match configurator.apply(&probe).await {
                Ok(()) => isolation.accepted.extend(half),
                Err(PortError::ConfigRejected { output }) => rejected_groups.push((half, output)),
                Err(e) => return Err(e),
            }
        }
    }
    isolation.accepted.sort_unstable();
    isolation.rejected.sort_unstable_by_key(|(i, _)| *i);
    Ok(isolation)
}

/// Splits `desired` into the config to apply and the `(site, reason)` of the vhosts whose custom
/// directives the policy refuses (`ApacheDirectives::validate`).
fn leave_out_refused_directives(desired: &WebServerConfig) -> (WebServerConfig, Vec<Exclusion>) {
    let mut allowed = desired.clone();
    let mut refused = Vec::new();
    allowed.vhosts.retain(|vhost| match vhost.custom_directives.validate() {
        Ok(()) => true,
        Err(e) => {
            refused.push((vhost.site.to_string(), e.to_string()));
            false
        }
    });
    (allowed, refused)
}

/// The `ConfigRejected` error naming the left-out sites, `None` when every site is served.
fn exclusion_error(excluded: &[Exclusion]) -> Option<AppError> {
    let (_, first) = excluded.first()?;
    let sites: Vec<&str> = excluded.iter().map(|(s, _)| s.as_str()).collect();
    let message = format!("the configuration of {} was rejected and is not served; the other sites are still served", sites.join(", "));
    Some(AppError::new(ErrorCode::ConfigRejected, message).with_details(serde_json::json!({
        "output": first,
        "excludedSites": sites,
        "rejections": excluded.iter().map(|(s, o)| serde_json::json!({ "site": s, "output": o })).collect::<Vec<_>>(),
    })))
}
