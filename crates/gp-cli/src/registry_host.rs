//! `gp registry host`: a registry of your own on GitHub, whose themes a
//! page offers beside its own registry's (docs/host-your-own.md).
//!
//! It reads a registry's checkout and says where the road to hosting it
//! stands, step by step, as `next` does for a port: the repo, the registry
//! in it, the namespace, the theme, the check from outside, and a page of
//! your own (the project's public page plays only its own registry's themes). A page always installs games from its
//! own registry and takes only themes from yours, so yours holds no game.
//! It prints the repo's raw address, which your own page takes as its registry of themes
//! and a page of your own as its second registry of themes
//! (`GP_PAGE_THEMES_URL`). It writes nothing; it asks git what the remote
//! holds.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, anyhow};
use gp_package::registry::{Catalog, Folder, Target, grants};
use serde_json::{Value, json};

use crate::Outcome;

/// `OWNER/REPO` of a GitHub remote address: https, ssh or scp-like.
pub(crate) fn github_of(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    };
    (ok(owner) && ok(repo) && !repo.contains('/')).then(|| (owner.to_owned(), repo.to_owned()))
}

/// The raw address GitHub serves a repo's files at, for a branch.
pub(crate) fn raw_of(owner: &str, repo: &str, branch: &str) -> String {
    format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}")
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// What `host` is given.
pub struct HostArgs<'a> {
    /// The registry's checkout, if there is one yet.
    pub dir: Option<&'a Path>,
    /// OWNER/REPO, when the checkout's origin isn't a GitHub address.
    pub github: Option<&'a str>,
    /// The theme to publish, NAMESPACE/NAME.
    pub theme: Option<&'a str>,
    /// The theme's folder, for the publish command.
    pub theme_dir: Option<&'a Path>,
    /// The game the theme is for, NAMESPACE/NAME.
    pub game: Option<&'a str>,
}

/// What the checkout holds, read once.
#[derive(Default)]
struct Facts {
    dir: Option<PathBuf>,
    git: bool,
    origin: Option<String>,
    github: Option<(String, String)>,
    branch: Option<String>,
    head: Option<String>,
    remote_head: Option<String>,
    remote_asked: bool,
    registry: bool,
    name: Option<String>,
    granted: Vec<String>,
    catalog: Option<Catalog>,
}

fn gather(a: &HostArgs) -> Result<Facts> {
    let mut f = Facts::default();
    let Some(dir) = a.dir else { return Ok(f) };
    let dir = crate::registry::local_folder(&dir.to_string_lossy())
        .ok_or_else(|| anyhow!("the registry for host is its checkout's folder, not an address"))?;
    f.dir = Some(std::path::absolute(&dir)?);
    if !dir.is_dir() {
        return Ok(f);
    }
    f.git = git(&dir, &["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true");
    if f.git {
        f.origin = git(&dir, &["remote", "get-url", "origin"]);
        f.branch = git(&dir, &["symbolic-ref", "--short", "HEAD"]);
        f.head = git(&dir, &["rev-parse", "--verify", "-q", "HEAD"]);
        if let (Some(_), Some(b)) = (&f.origin, &f.branch) {
            f.remote_asked = true;
            f.remote_head = git(&dir, &["ls-remote", "origin", &format!("refs/heads/{b}")])
                .and_then(|l| l.split_whitespace().next().map(str::to_owned));
        }
    }
    f.github = match a.github {
        Some(g) => Some(
            g.split_once('/')
                .map(|(o, r)| (o.to_owned(), r.to_owned()))
                .filter(|(o, r)| github_of(&format!("https://github.com/{o}/{r}")).is_some())
                .ok_or_else(|| anyhow!("--github is OWNER/REPO, not {g}"))?,
        ),
        None => f.origin.as_deref().and_then(github_of),
    };
    let t = Folder(dir.clone());
    if let Some(b) = t.read("registry.toml").map_err(|e| anyhow!(e))? {
        f.registry = true;
        let v: toml::Table = String::from_utf8_lossy(&b).parse().unwrap_or_default();
        f.name = v.get("name").and_then(|n| n.as_str()).map(str::to_owned);
        f.granted = grants(&t)
            .map_err(|e| anyhow!(e))?
            .into_iter()
            .filter(|(_, g)| g.as_ref().is_ok_and(|g| g.status == "granted"))
            .map(|(n, _)| n)
            .collect();
    }
    if let Some(b) = t.read("catalog.toml").map_err(|e| anyhow!(e))? {
        f.catalog = toml::from_str(&String::from_utf8_lossy(&b)).ok();
    }
    Ok(f)
}

/// One step of the road.
struct Step {
    name: &'static str,
    who: &'static str,
    /// Done, not done, or not known from here (None).
    done: Option<bool>,
    says: String,
    cli: Vec<String>,
    asks_first: bool,
}

impl Step {
    fn json(&self, n: usize, now: usize) -> Value {
        json!({
            "n": n, "name": self.name, "who": self.who, "done": self.done,
            "now": n == now, "says": self.says, "cli": self.cli, "asks_first": self.asks_first,
        })
    }
}

pub fn host(a: &HostArgs) -> Result<Outcome> {
    let f = gather(a)?;
    let in_catalog = |name: &str| {
        f.catalog
            .as_ref()
            .and_then(|c| c.package.iter().find(|p| p.name == name))
    };
    let theme_entry = a.theme.and_then(in_catalog);
    let game = a
        .game
        .map(str::to_owned)
        .or_else(|| theme_entry.and_then(|t| t.for_games.first().cloned()));
    let game_s = game.clone().unwrap_or_else(|| "GAME".into());
    let theme_s = a.theme.unwrap_or("NAMESPACE/THEME");
    let ns = a
        .theme
        .and_then(|t| t.split_once('/'))
        .map(|(n, _)| n.to_owned());
    let ns_s = ns.clone().unwrap_or_else(|| "NAMESPACE".into());
    let dir_s = f
        .dir
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "DIR".into());
    let target = format!("git:{dir_s}");
    let (owner, repo) = f
        .github
        .clone()
        .unwrap_or_else(|| ("OWNER".into(), "REPO".into()));
    let branch = f.branch.clone().unwrap_or_else(|| "main".into());
    let raw = f.github.as_ref().map(|(o, r)| raw_of(o, r, &branch));
    let raw_s = raw
        .clone()
        .unwrap_or_else(|| raw_of(&owner, &repo, &branch));
    let pushed = f.head.is_some() && f.head == f.remote_head;
    let theme_dir = a
        .theme_dir
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "THEME_DIR".into());
    let theme_listed = theme_entry.is_some_and(|t| {
        t.kind == "content" && game.as_ref().is_none_or(|g| t.for_games.contains(g))
    });

    let steps = [
        Step {
            name: "Make a repo for your registry",
            who: "person",
            done: Some(f.git && f.github.is_some()),
            says: match (&f.dir, f.git, &f.origin) {
                (None, ..) => "Make an empty public repo on GitHub for your registry (acme/harbor-registry, say), clone it to a folder on this computer, and give that folder as the registry.".into(),
                (Some(d), false, _) => format!("{} isn't a git checkout yet. Clone your empty public GitHub repo there.", d.display()),
                (Some(d), true, None) => format!("{} has no origin. Add your GitHub repo as origin, or give --github OWNER/REPO.", d.display()),
                (Some(_), true, Some(o)) if f.github.is_none() => format!("The checkout's origin, {o}, isn't on GitHub. Give the GitHub repo it stands for with --github OWNER/REPO."),
                _ => format!("Your registry's repo is {owner}/{repo}, cloned at {dir_s}."),
            },
            cli: vec!["gh repo create OWNER/REPO --public --clone".into()],
            asks_first: true,
        },
        Step {
            name: "Make it a registry",
            who: "agent",
            done: Some(f.registry),
            says: if f.registry {
                format!("It is the registry {}.", f.name.as_deref().unwrap_or("(no name)"))
            } else {
                format!("Make it a registry, unsigned, with your namespace {ns_s} granted. This commits and pushes to your repo.")
            },
            cli: vec![format!("gp registry init {target} --name {repo} --namespace {ns_s}")],
            asks_first: true,
        },
        Step {
            name: "Grant your namespace",
            who: "agent",
            done: Some(match &ns {
                Some(n) => f.granted.contains(n),
                None => !f.granted.is_empty(),
            }),
            says: if f.granted.is_empty() {
                format!("Grant your namespace {ns_s}, so packages named {ns_s}/... install from it.")
            } else {
                format!("Granted: {}.", f.granted.join(", "))
            },
            cli: vec![format!("gp registry grant {target} {ns_s}")],
            asks_first: true,
        },
        Step {
            name: "Publish your theme",
            who: "agent",
            done: Some(theme_listed),
            says: if theme_listed {
                format!("{theme_s} is published and listed for {game_s}.")
            } else {
                format!("Publish your theme {theme_s} for {game_s}, then list it, so the page offers it.")
            },
            cli: vec![
                format!("gp registry publish {target} {theme_dir} --name {theme_s} --version VERSION --kind content --title TITLE --for {game_s}"),
                format!("gp registry reindex {target} --keep {theme_s}@VERSION"),
            ],
            asks_first: true,
        },
        Step {
            name: "Check it from outside",
            who: "agent",
            done: None,
            says: if f.head.is_some() && !pushed && f.remote_asked {
                format!("Your checkout has commits GitHub doesn't have yet: push them (git -C {dir_s} push origin {branch}), then check it as a player's browser reads it. GitHub's raw host can take five minutes to show a new file.")
            } else {
                "Check it as a player's browser reads it: every file by its hash, and the theme as the page would offer it. GitHub's raw host can take five minutes to show a new file.".into()
            },
            cli: vec![format!("gp registry check {raw_s} --for {game_s} --theme {theme_s}")],
            asks_first: false,
        },
        Step {
            name: "A page of your own",
            who: "person",
            done: None,
            says: format!("The project's public page plays only its own registry's themes, so yours go on a page of your own: build the page with your registry as its registry of themes ({game_s} still comes from the project's registry) and put it on your own free Cloudflare worker (it uploads, so it asks first). Then share your page's link. GitHub Pages works only for games with no module: it can't send the headers the page needs."),
            cli: vec![
                format!("GP_PAGE_THEMES_URL={raw_s} GP_PAGE_CONTENT={theme_s} scripts/web/release-check.sh --no-roms --game GAME_DIR"),
                "scripts/web/deploy.sh --no-roms --worker NAME --subdomain YOUR_SUBDOMAIN".into(),
                "scripts/web/pages.sh PAGE_DIR OUT (GitHub Pages)".into(),
            ],
            asks_first: true,
        },
    ];
    let now = steps
        .iter()
        .position(|s| s.done == Some(false))
        .unwrap_or(4)
        + 1;
    let next = &steps[now - 1];
    let stands = match now {
        1 => "You have no registry repo yet.".to_owned(),
        2 => format!("{owner}/{repo} is cloned at {dir_s} and isn't a registry yet."),
        3 => format!("{owner}/{repo} is a registry, and your namespace isn't granted in it."),
        4 => format!(
            "{owner}/{repo} is a registry without your theme {theme_s} listed for {game_s}."
        ),
        _ => format!("{owner}/{repo} holds your theme {theme_s}, listed for {game_s}."),
    };
    let mut text = vec![stands.clone(), String::new()];
    for (i, s) in steps.iter().enumerate() {
        let mark = match s.done {
            Some(true) => "done",
            Some(false) if i + 1 == now => "NOW ",
            Some(false) => "    ",
            None if i + 1 == now => "NOW ",
            None => " -  ",
        };
        text.push(format!("{mark} {}. {}: {}", i + 1, s.name, s.says));
        for c in &s.cli {
            text.push(format!("       {c}"));
        }
        if s.asks_first {
            text.push("       (writes to the outside world: ask the person first)".into());
        }
    }
    text.push(String::new());
    match &raw {
        Some(r) => {
            text.push(format!("Your registry's address: {r}"));
            text.push(format!(
                "  as your own page's registry of themes: GP_PAGE_THEMES_URL={r}"
            ));
        }
        None => {
            text.push("No address yet: it comes from the repo's GitHub name and branch.".into())
        }
    }
    let json = json!({
        "stage": {"n": now, "of": steps.len(), "name": next.name},
        "stands": stands,
        "next": next.json(now, now),
        "steps": steps.iter().enumerate().map(|(i, s)| s.json(i + 1, now)).collect::<Vec<_>>(),
        "raw": raw,
        "themes": raw.as_ref().map(|r| json!({"name": f.name.clone().unwrap_or_else(|| repo.clone()), "url": r})),
        "facts": {
            "dir": f.dir, "git": f.git, "origin": f.origin,
            "github": f.github.as_ref().map(|(o, r)| format!("{o}/{r}")),
            "branch": f.branch, "head": f.head, "remote_head": f.remote_head, "pushed": pushed,
            "registry": f.registry, "name": f.name, "granted": f.granted,
            "game": game, "theme": a.theme, "theme_listed": theme_listed,
        },
    });
    Ok(Outcome {
        json,
        text: text.join("\n"),
        exit: 0,
    })
}

#[cfg(test)]
mod tests;
