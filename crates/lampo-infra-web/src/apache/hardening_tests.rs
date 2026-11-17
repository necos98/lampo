//! The local web perimeter of the generated configuration: what Apache must refuse to serve
//! from a document root (VCS metadata, secrets, logs, dumps, directory listings) and which
//! `Host` headers each address answers (DNS-rebinding protection on the dedicated ports).
//! The regular expressions themselves are exercised against real Apache by `tests/smoke_apache.rs`.

use lampo_domain::project::ApacheDirectives;

use super::render::ApacheConfigRenderer;
use super::tests_support::{sample_config, sample_options, vhost};

fn render_main() -> String {
    ApacheConfigRenderer::new(sample_options()).render_main(&sample_config()).unwrap()
}

/// `(ServerName, ServerAlias…)` of the `<VirtualHost {address}>` blocks of `text`, in order.
fn names_of_blocks(text: &str, address: &str) -> Vec<Vec<String>> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in text.lines().map(str::trim) {
        if let Some(addresses) = line.strip_prefix("<VirtualHost ").and_then(|l| l.strip_suffix('>')) {
            current = addresses.split(' ').any(|a| a == address).then(Vec::new);
        } else if line == "</VirtualHost>" {
            blocks.extend(current.take());
        } else if let Some(names) = current.as_mut()
            && let Some(name) = line.strip_prefix("ServerName ").or_else(|| line.strip_prefix("ServerAlias "))
        {
            names.push(name.to_string());
        }
    }
    blocks
}

#[test]
fn version_control_directories_are_denied() {
    let main = render_main();
    assert!(main.contains("<DirectoryMatch \"(?i)/\\.(git|svn|hg)(/|$)\">\n    Require all denied\n</DirectoryMatch>"), "{main}");
}

#[test]
fn secrets_logs_and_dumps_are_denied() {
    let main = render_main();
    assert!(main.contains("<FilesMatch \"(?i)^\\.(env|git)|\\.(log|sql|bak)$\">\n    Require all denied\n</FilesMatch>"), "{main}");
    assert!(main.contains("<Files \".ht*\">\n    Require all denied\n</Files>"));
}

/// The denial every site repeats as its last section (see vhost.conf.j2).
const BARRIER: &str = "    <Location \"/\">
        <If \"%{REQUEST_FILENAME} =~ m#(/\\.(ht|env|git)[^/]*|/\\.(git|svn|hg)(/.*)?|\\.(log|sql|bak))$#i\">
            AuthMerging Off
            Satisfy All
            Require all denied
        </If>
    </Location>
</VirtualHost>";

#[test]
fn every_site_ends_with_the_internal_files_barrier() {
    // Apache merges a site's last top-level <If> after its custom <Location>/<If> sections, so
    // the barrier must close every block of every site, after the custom directives.
    for (name, text) in ApacheConfigRenderer::new(sample_options()).render(&sample_config()).unwrap() {
        if !name.starts_with("vhosts/") {
            continue;
        }
        let blocks = text.matches("<VirtualHost ").count();
        assert!(blocks > 0, "{name}");
        assert_eq!(text.matches(BARRIER).count(), blocks, "{name}:\n{text}");
        if let Some(custom) = text.find("# --- End of custom directives ---") {
            assert!(custom < text.find(BARRIER).unwrap(), "{name}: the barrier must follow the custom directives");
        }
    }
}

#[test]
fn the_last_custom_line_is_continued_only_where_it_was_validated() {
    // A `\` followed by a blank does not continue the line for Apache nor for the validation:
    // trimming the blank would join the line with the end-of-directives comment.
    let render = |text: &str| {
        let mut config = sample_config();
        config.vhosts[1].custom_directives = ApacheDirectives::new(text).unwrap();
        ApacheConfigRenderer::new(sample_options()).render_vhost(&config, &config.vhosts[1]).unwrap()
    };
    let shop = render("Header set X-A \\ \n\n");
    assert!(shop.contains("\nHeader set X-A \\ \n    # --- End of custom directives ---\n"), "{shop}");
    let blank = render(" \n\t\n");
    assert!(!blank.contains("# --- End of custom directives ---"), "blank directives render no section: {blank}");
}

#[test]
fn directory_listing_is_off_by_default_but_htaccess_still_works() {
    for (name, text) in ApacheConfigRenderer::new(sample_options()).render(&sample_config()).unwrap() {
        assert!(!text.contains("Indexes"), "{name} enables directory listing");
        if name.starts_with("vhosts/") {
            assert!(text.contains("Options FollowSymLinks ExecCGI\n        AllowOverride All"), "{name}");
        }
    }
}

#[test]
fn every_listened_address_starts_with_the_catch_all() {
    let main = render_main();
    let catch_all = "<VirtualHost 127.0.0.1:80 [::1]:80 127.0.0.1:8080 127.0.0.1:8101>";
    assert!(main.contains(catch_all), "{main}");
    // The catch-all is defined before the sites are included, so it is the default vhost
    // (first one) of every address: a Host header no site claims gets its 404.
    assert!(main.find(catch_all).unwrap() < main.find("IncludeOptional").unwrap());
    let names = names_of_blocks(&main, "127.0.0.1:8101");
    assert_eq!(names.len(), 1);
    assert!(!names[0].iter().any(|n| n == "localhost" || n == "127.0.0.1"), "the catch-all must not claim the loopback names: {names:?}");
}

#[test]
fn dedicated_port_answers_to_loopback_and_its_own_names_only() {
    let files = ApacheConfigRenderer::new(sample_options()).render(&sample_config()).unwrap();
    let worktree = &files[3].1;
    assert_eq!(names_of_blocks(worktree, "127.0.0.1:8101"), [["feature-x.shop.lampo.invalid", "localhost", "127.0.0.1"]]);
    let legacy = &files[1].1;
    assert_eq!(names_of_blocks(legacy, "127.0.0.1:8080"), [["legacy.test", "localhost", "127.0.0.1"]]);
}

#[test]
fn dedicated_port_keeps_every_configured_name() {
    let mut config = sample_config();
    config.vhosts = vec![vhost("shop", &["shop.localhost", "shop.test", "api.shop.localhost"], Some(8200), r"C:\shop")];
    let text = &ApacheConfigRenderer::new(sample_options()).render(&config).unwrap()[1].1;
    assert_eq!(names_of_blocks(text, "127.0.0.1:8200"), [["shop.localhost", "shop.test", "api.shop.localhost", "localhost", "127.0.0.1"]]);
    // The shared port stays name-based: no loopback aliases there.
    assert_eq!(names_of_blocks(text, "127.0.0.1:80"), [["shop.localhost", "shop.test", "api.shop.localhost"]]);
}
