//! The tool `host_your_own`: the road to a registry and a page of the
//! person's own (docs/host-your-own.md), step by step, from `gp --json
//! registry host` (crates/gp-cli/src/registry_host.rs): a repo for the
//! registry, the registry in it, their namespace, their theme, the check
//! from outside, the public page with `r=` (which adds their themes to its
//! own registry's games), and optionally a page of their own. Like `next` it reads and starts
//! nothing: every step's commands run through `gp_run`, where each one that
//! writes to the outside world (init, grant, publish and reindex on a git:
//! target, which commit and push) needs the person's yes.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::{Config, Reply, run_gp};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostArgs {
    /// The registry's checkout on this computer: a clone of the person's
    /// public GitHub repo for it. Left out before there is one: the answer
    /// is then the first step, making that repo.
    #[serde(default)]
    pub registry: Option<String>,
    /// The person's theme as a package name, NAMESPACE/NAME
    /// (acme/harbor-lights); its namespace is the one to grant.
    #[serde(default)]
    pub theme: Option<String>,
    /// The theme's folder, for the publish command in the steps.
    #[serde(default)]
    pub theme_dir: Option<String>,
    /// The game the theme is for, NAMESPACE/NAME (acme/rooftop).
    #[serde(default)]
    pub game: Option<String>,
    /// The GitHub repo, OWNER/REPO, when the checkout's origin isn't a
    /// GitHub address.
    #[serde(default)]
    pub github: Option<String>,
}

/// A package name or OWNER/REPO: two plain parts.
fn two_parts(s: &str) -> bool {
    let ok = |p: &str| {
        !p.is_empty()
            && p.len() <= 100
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
            && !p.starts_with('.')
    };
    s.split_once('/').is_some_and(|(a, b)| ok(a) && ok(b))
}

/// `host_your_own`.
pub fn host(config: &Config, a: HostArgs) -> Result<Reply, String> {
    let mut args: Vec<String> = vec!["registry".into(), "host".into()];
    if let Some(r) = &a.registry {
        if r.starts_with('-') || r.contains("://") {
            return Err(format!(
                "registry is the checkout's folder on this computer, not {r:?}"
            ));
        }
        args.push(r.strip_prefix("git:").unwrap_or(r).to_owned());
    }
    for (flag, v) in [
        ("--theme", &a.theme),
        ("--game", &a.game),
        ("--github", &a.github),
    ] {
        if let Some(v) = v {
            if !two_parts(v) {
                return Err(format!(
                    "{} is two parts, like acme/harbor-lights (got {v:?})",
                    &flag[2..]
                ));
            }
            args.push(flag.into());
            args.push(v.clone());
        }
    }
    if let Some(d) = &a.theme_dir {
        if d.starts_with('-') {
            return Err(format!("theme_dir is a folder, not {d:?}"));
        }
        args.push("--theme-dir".into());
        args.push(d.clone());
    }
    let os: Vec<&std::ffi::OsStr> = args.iter().map(|s| s.as_ref()).collect();
    let (code, mut v) = run_gp(config, &os)?;
    if code != 0 {
        return Err(v["error"]
            .as_str()
            .map_or_else(|| v.to_string(), str::to_owned));
    }
    v["how"] = json!(
        "Run each step's cli through gp_run, one at a time, and call host_your_own again after each to see the next. A step with asks_first writes to the outside world (a public repo, a push, an upload): tell the person exactly what it does and where, and ask first; gp_run asks for their yes too. Making the GitHub repo and deploying a page are done by the person, or by you with their yes."
    );
    Ok(Reply::ok(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn config(dir: &Path, body: &str) -> Config {
        let gp = dir.join("gp");
        std::fs::write(&gp, format!("#!/bin/sh\n{body}\n")).unwrap();
        crate::proc::make_executable(&gp).unwrap();
        Config {
            gp,
            repo: dir.to_owned(),
            work: dir.join("work"),
            home: dir.join("home"),
            rom: None,
            assets: None,
            games: None,
        }
    }

    #[test]
    fn it_asks_gp_registry_host_and_passes_the_steps_on() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("args");
        let c = config(
            dir.path(),
            &format!(
                "echo \"$@\" > {}\necho '{{\"stage\": {{\"n\": 4}}, \"steps\": []}}'",
                log.display()
            ),
        );
        let r = host(
            &c,
            HostArgs {
                registry: Some("git:/tmp/acme-registry".into()),
                theme: Some("acme/harbor-lights".into()),
                theme_dir: None,
                game: Some("acme/rooftop".into()),
                github: None,
            },
        )
        .unwrap();
        assert!(!r.is_error);
        assert_eq!(r.json["stage"]["n"], 4);
        assert!(r.json["how"].as_str().unwrap().contains("ask first"));
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().trim(),
            "--json registry host /tmp/acme-registry --theme acme/harbor-lights --game acme/rooftop"
        );
    }

    #[test]
    fn names_and_folders_are_held_to_their_shape() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path(), "echo '{}'");
        let bad = |a: HostArgs| host(&c, a).unwrap_err();
        let none = || HostArgs {
            registry: None,
            theme: None,
            theme_dir: None,
            game: None,
            github: None,
        };
        assert!(
            bad(HostArgs {
                theme: Some("harbor".into()),
                ..none()
            })
            .contains("two parts")
        );
        assert!(
            bad(HostArgs {
                game: Some("a/b/c".into()),
                ..none()
            })
            .contains("two parts")
        );
        assert!(
            bad(HostArgs {
                registry: Some("--exec".into()),
                ..none()
            })
            .contains("folder")
        );
        assert!(
            bad(HostArgs {
                registry: Some("https://raw.githubusercontent.com/a/b/main".into()),
                ..none()
            })
            .contains("folder")
        );
    }
}
