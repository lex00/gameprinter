//! The public page's response headers (scripts/web/public/_headers), read
//! once for every local way of serving a game, so a game played on this
//! machine runs under the same Content-Security-Policy and the same
//! isolation headers as the public page (docs/web.md, "Serving a game on
//! this machine"): `gp registry serve` and `gp theme look play` (the page
//! with a registry, or a game's own files), the mechanic page and gp serve
//! (the game's frame: the rebuild's preview and a new game built to play),
//! and `gp host serve`'s playable links, which `build_publish` names.
//!
//! The file is in Cloudflare Pages' form: a path pattern on a line of its
//! own (`*` matches any run of characters), then its headers indented as
//! `Name: value`. Rules that match one path add their headers together.

/// The public page's `_headers`, as this gp was built with it.
pub const PUBLIC: &str = include_str!("../../../scripts/web/public/_headers");

/// One rule: its path pattern and its headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub pattern: String,
    pub headers: Vec<(String, String)>,
}

/// The rules of a `_headers` text, comments and blank lines left out.
pub fn parse(text: &str) -> Vec<Rule> {
    let mut rules: Vec<Rule> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            if let (Some(r), Some((k, v))) = (rules.last_mut(), line.trim().split_once(':')) {
                r.headers.push((k.trim().to_owned(), v.trim().to_owned()));
            }
        } else {
            rules.push(Rule {
                pattern: line.trim().to_owned(),
                headers: Vec::new(),
            });
        }
    }
    rules
}

/// Whether `pattern` (with `*` for any run of characters) matches `path`.
fn matches(pattern: &str, path: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == path;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !path.starts_with(first) || path.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &path[first.len()..];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// The headers `text`'s rules give `path` (the request's path, no query),
/// each name once: two rules that set one name join their values with
/// ", ", as Cloudflare Pages does.
pub fn for_path(text: &str, path: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for r in parse(text).iter().filter(|r| matches(&r.pattern, path)) {
        for (k, v) in &r.headers {
            match out.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(k)) {
                Some((_, have)) => {
                    have.push_str(", ");
                    have.push_str(v);
                }
                None => out.push((k.clone(), v.clone())),
            }
        }
    }
    out
}

/// The public page's headers for `path`.
pub fn public_for(path: &str) -> Vec<(String, String)> {
    for_path(PUBLIC, path)
}

/// Header lines (`Name: value`) for an HTTP answer.
pub fn lines(headers: &[(String, String)]) -> Vec<String> {
    headers.iter().map(|(k, v)| format!("{k}: {v}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_page_sets_its_policy_and_isolation_on_every_path() {
        for path in [
            "/",
            "/index.html",
            "/game/preview.js",
            "/play/abc/index.html",
        ] {
            let h = public_for(path);
            let get = |n: &str| h.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
            let csp = get("Content-Security-Policy").unwrap();
            assert!(
                csp.contains("default-src 'none'")
                    && csp.contains("script-src 'self' 'wasm-unsafe-eval'"),
                "{csp}"
            );
            assert!(
                !csp.contains("unsafe-inline") && !csp.contains("'unsafe-eval'"),
                "{csp}"
            );
            assert_eq!(get("Cross-Origin-Opener-Policy"), Some("same-origin"));
            assert_eq!(get("Cross-Origin-Embedder-Policy"), Some("require-corp"));
            assert_eq!(get("X-Content-Type-Options"), Some("nosniff"));
        }
    }

    #[test]
    fn rules_match_by_pattern_and_add_together() {
        let t = "# a comment\n/*\n  A: 1\n/game/*\n  B: 2\n  A: 3\n/exact\n  C: 4\n";
        assert_eq!(for_path(t, "/x"), vec![("A".into(), "1".into())]);
        assert_eq!(
            for_path(t, "/game/p.js"),
            vec![("A".into(), "1, 3".into()), ("B".into(), "2".into())]
        );
        assert_eq!(for_path(t, "/exact").len(), 2);
        assert!(matches("/a*b*c", "/a-x-b-y-c") && !matches("/a*b", "/a-c"));
    }
}
