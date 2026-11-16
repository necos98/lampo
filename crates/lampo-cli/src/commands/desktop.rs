//! `open`: the resolved site in one of the user's desktop apps, started from this process.

use lampo_application::AppResult;
use lampo_application::dto::{ExternalApp, OpenedInDto};

use super::Cmd;
use crate::cli::OpenArgs;
use crate::render;

pub fn open(c: &Cmd<'_>, args: OpenArgs) -> AppResult<i32> {
    let site = c.site_id()?;
    let opened = c.ctx.opener.open(&site, args.app())?;
    if let Some(warning) = unserved_warning(&opened) {
        c.io.err_line(&warning);
    }
    c.io.emit(&opened, render::opened);
    Ok(0)
}

/// The browser shows an error page for a site Apache does not serve: say how to serve it.
fn unserved_warning(opened: &OpenedInDto) -> Option<String> {
    (opened.app == ExternalApp::Browser && !opened.served)
        .then(|| format!("lampo: warning: `{site}` is not active, so the page will not load: run `lampo project activate {site}`", site = opened.site))
}
