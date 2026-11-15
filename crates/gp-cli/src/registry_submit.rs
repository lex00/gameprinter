//! `gp registry submit`: a game or theme offered to a registry that lives
//! in a GitHub repo the person can't write to (lex00/gp-registry), as a
//! pull request its owner merges or not (issue #9).
//!
//! It runs the commands the owner would run, in a clone of the registry on
//! this machine, so every check `gp registry publish` runs is run here by
//! the same code: the folder is staged without its hidden files, the
//! namespace granted when the registry doesn't grant it yet (unsigned
//! registries only), the package published (`publish_cmd`, with its ROM
//! checks) and listed (`reindex_cmd --keep`), one commit each. Then the
//! branch goes to the person's fork (made with `gh repo fork` when they have
//! none) and `gh pr create` opens the pull request. Any check that fails
//! stops it before anything leaves this machine.
//!
//! | Where | What |
//! |---|---|
//! | `$GP_HOME/work/registry-submit/NS-NAME-VERSION/stage` | The package's files as submitted |
//! | `$GP_HOME/work/registry-submit/NS-NAME-VERSION/registry` | The clone, on branch `submit/NS-NAME-VERSION` |
//!
//! `gh` is the GitHub command line, logged in as the person; `$GP_GH` names
//! another program in its place (the tests' stand-in).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use gp_package::registry::{Folder, Reader, name_ok};
use serde_json::{Value, json};

use crate::Outcome;
use crate::registry::{Writing, grant_cmd, publish_cmd, reindex_cmd};

/// The registry the tests submit to (a stand-in served from a local bare
/// repo). gp registry submit has no default: --to names the registry.
#[cfg(test)]
const UPSTREAM: &str = "lex00/gp-registry";

/// What a submission is: the folder and how it is published.
pub struct Submission<'a> {
    pub dir: &'a Path,
    pub name: &'a str,
    pub version: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub for_games: &'a [String],
    /// The namespace's signing key, for a signed registry.
    pub key: Option<&'a Path>,
}

/// Where it goes: the registry's repo, the person's fork (None: theirs by
/// their GitHub login, made when missing), and whether to stop before
/// anything leaves this machine.
pub struct Route<'a> {
    pub upstream: &'a str,
    pub fork: Option<&'a str>,
    pub dry_run: bool,
    /// The program that answers for `gh`.
    pub gh: &'a str,
    /// The folder the stage and the clone go in.
    pub work: &'a Path,
}

/// The GitHub command line.
struct Gh<'a>(&'a str);

impl Gh<'_> {
    fn run(&self, args: &[&str]) -> Result<String> {
        let o = Command::new(self.0)
            .args(args)
            .output()
            .with_context(|| format!("running {} (the GitHub command line, gh)", self.0))?;
        if !o.status.success() {
            bail!(
                "gh {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
    }

    fn exists(&self, repo: &str) -> bool {
        self.run(&["repo", "view", repo, "--json", "name", "--jq", ".name"])
            .is_ok()
    }

    fn url(&self, repo: &str) -> Result<String> {
        self.run(&["repo", "view", repo, "--json", "url", "--jq", ".url"])
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("running git")?;
    if !o.status.success() {
        bail!(
            "git {} in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// A fresh folder: what an earlier submission left there is taken away.
fn fresh(dir: &Path) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir).with_context(|| format!("clearing {}", dir.display()))?;
    }
    fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))
}

/// Copies `from` into `to`, leaving out every file and folder whose name
/// starts with `.` (.git, .gitignore, .DS_Store): `gp registry publish`
/// takes every file it is given. Returns what was left out, relative.
fn stage(from: &Path, to: &Path) -> Result<Vec<String>> {
    let mut left = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let src = from.join(&rel);
        let mut entries: Vec<_> = fs::read_dir(&src)
            .with_context(|| format!("reading {}", src.display()))?
            .collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let r = rel.join(e.file_name());
            if e.file_name().to_string_lossy().starts_with('.') {
                left.push(r.to_string_lossy().replace('\\', "/"));
                continue;
            }
            if e.file_type()?.is_dir() {
                fs::create_dir_all(to.join(&r))?;
                stack.push(r);
            } else {
                fs::copy(e.path(), to.join(&r))?;
            }
        }
    }
    left.sort();
    Ok(left)
}

/// What a submission that stopped says: the check's words, then plainly
/// what happened and what to do.
fn stopped(step: &str, why: &str, s: &Submission, clone: &Path) -> Outcome {
    Outcome {
        json: json!({
            "submitted": false, "stopped_at": step, "why": why,
            "name": s.name, "version": s.version, "clone": clone,
        }),
        text: format!(
            "{why}\n\nThe submission of {}@{} stopped at {step}. Nothing was pushed to your fork and no pull request was opened; the registry and your fork are as they were. The same version can be submitted again, since it was never published. Fix what the check names in {} and run gp registry submit again. The clone it was checked in is {}, for a look.",
            s.name,
            s.version,
            s.dir.display(),
            clone.display()
        ),
        exit: 1,
    }
}

/// A command's words without its "Committed ... no origin" line: the
/// clone has no origin on purpose, and the push comes at the end.
fn words(o: &Outcome) -> String {
    o.text
        .lines()
        .filter(|l| !l.starts_with("Committed "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A command's outcome, or the words of its failure.
fn ran(r: Result<Outcome>) -> std::result::Result<Outcome, String> {
    match r {
        Ok(o) if o.exit == 0 => Ok(o),
        Ok(o) => Err(o.text),
        Err(e) => Err(format!("{e:#}")),
    }
}

pub fn run(s: &Submission, route: &Route) -> Result<Outcome> {
    if !name_ok(s.name) {
        bail!(
            "{:?} isn't a package name: NAMESPACE/NAME, lowercase letters, digits, '-' and '.'",
            s.name
        );
    }
    if !s.dir.is_dir() {
        bail!("{} isn't a folder", s.dir.display());
    }
    let (ns, short) = s.name.split_once('/').unwrap_or_default();
    let gh = Gh(route.gh);
    let slug = format!("{ns}-{short}-{}", s.version);
    let branch = format!("submit/{slug}");
    let base_dir = route.work.join(&slug);
    let (stage_dir, clone) = (base_dir.join("stage"), base_dir.join("registry"));
    let mut text = String::new();

    // The package, as it will arrive.
    fresh(&stage_dir)?;
    let left = stage(s.dir, &stage_dir)?;
    let _ = writeln!(
        text,
        "staged {} in {}{}",
        s.dir.display(),
        stage_dir.display(),
        if left.is_empty() {
            String::new()
        } else {
            format!(", leaving out {}", left.join(", "))
        }
    );

    // Who is submitting, and to which fork.
    let login = gh
        .run(&["api", "user", "--jq", ".login"])
        .context("gh isn't logged in to GitHub: run gh auth login first")?;
    let upstream_name = route.upstream.rsplit('/').next().unwrap_or(route.upstream);
    let fork = route
        .fork
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{login}/{upstream_name}"));
    let fork_owner = fork.split('/').next().unwrap_or(&login).to_owned();

    // The registry, cloned fresh, on a branch of the submission's own.
    if base_dir.join("registry").exists() {
        fs::remove_dir_all(&clone).with_context(|| clone.display().to_string())?;
    }
    let upstream_url = gh.url(route.upstream)?;
    let o = Command::new("git")
        .args(["clone", "-q", &upstream_url])
        .arg(&clone)
        .output()
        .context("running git clone")?;
    if !o.status.success() {
        bail!(
            "cloning {} ({upstream_url}): {}",
            route.upstream,
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    let base = git(&clone, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    // No `origin`: the registry commands then commit and never pull or
    // push; the branch goes to the fork at the end, by name.
    git(&clone, &["remote", "rename", "origin", "upstream"])?;
    git(&clone, &["checkout", "-q", "-b", &branch])?;
    if git(&clone, &["config", "user.email"]).is_err() {
        git(
            &clone,
            &[
                "config",
                "user.email",
                &format!("{login}@users.noreply.github.com"),
            ],
        )?;
        git(&clone, &["config", "user.name", &login])?;
    }
    let _ = writeln!(
        text,
        "cloned {} into {}, branch {branch} from {base}",
        route.upstream,
        clone.display()
    );
    let target = format!("git:{}", clone.display());
    let how = Writing {
        dry_run: false,
        no_push: false,
    };

    // The namespace: granted already, or granted in this pull request when
    // the registry is unsigned (the owner decides when they merge).
    let reg = Reader::unsigned(&Folder(clone.clone()))
        .map_err(|e| anyhow!("{} isn't a registry: {e}", route.upstream))?
        .file;
    let signed = !reg.signing.is_empty();
    let mut granted_here = false;
    if !reg.namespaces.iter().any(|n| n == ns) {
        if !reg.root.is_empty() {
            return Ok(stopped(
                "the namespace",
                &format!(
                    "{} doesn't grant the namespace {ns} yet, and it is signed, so only its owner can grant it (gp registry grant, with the root key). Ask the owner to grant {ns} with your public key",
                    route.upstream
                ),
                s,
                &clone,
            ));
        }
        match ran(grant_cmd(&target, ns, &[], None, how)) {
            Ok(o) => {
                granted_here = true;
                let _ = writeln!(text, "{}", words(&o));
            }
            Err(why) => return Ok(stopped("the namespace grant", &why, s, &clone)),
        }
    }

    // Every check publish runs, by publish itself.
    let published = match ran(publish_cmd(
        &target,
        &stage_dir,
        (s.name, s.version),
        (s.kind, s.title),
        s.for_games,
        s.key,
        how,
    )) {
        Ok(o) => o,
        Err(why) => return Ok(stopped("the publish checks", &why, s, &clone)),
    };
    let _ = writeln!(text, "{}", words(&published));

    // Its lines in the list files. A signed registry's lists are signed by
    // the owner's key, so the owner lists it after merging.
    let listed = if signed {
        let _ = writeln!(
            text,
            "{}'s list files are signed by its owner's key, so this pull request doesn't list {}@{}: the owner runs gp registry reindex --keep {}@{} after merging",
            route.upstream, s.name, s.version, s.name, s.version
        );
        false
    } else {
        let at = format!("{}@{}", s.name, s.version);
        match ran(reindex_cmd(
            &target,
            (std::slice::from_ref(&at), true, &[]),
            None,
            how,
        )) {
            Ok(o) => {
                let _ = writeln!(text, "{}", words(&o));
                true
            }
            Err(why) => return Ok(stopped("the listing (reindex)", &why, s, &clone)),
        }
    };

    let commits: Vec<String> = git(
        &clone,
        &["log", "--format=%h %s", &format!("upstream/{base}..HEAD")],
    )?
    .lines()
    .map(str::to_owned)
    .collect();
    let changed = git(
        &clone,
        &["diff", "--stat=100", &format!("upstream/{base}..HEAD")],
    )?;
    let title = format!("Submit {}@{}", s.name, s.version);
    let body = pr_body(s, &published.json, (granted_here, listed), &commits);
    let mut j = json!({
        "name": s.name, "version": s.version, "stage": stage_dir, "clone": clone,
        "branch": branch, "fork": fork, "upstream": route.upstream,
        "commits": commits, "granted": granted_here, "listed": listed,
        "id": published.json["id"], "title": title, "body": body,
        "left_out": left,
    });
    if route.dry_run {
        let _ = write!(
            text,
            "dry run: every check passed and the commits are made in {}, and nothing left this machine. A real run pushes branch {branch} to {fork}{} and opens the pull request \"{title}\" on {}.\n{changed}",
            clone.display(),
            if gh.exists(&fork) {
                ""
            } else {
                " (made first with gh repo fork, since you have none)"
            },
            route.upstream
        );
        j["submitted"] = json!(false);
        j["dry_run"] = json!(true);
        return Ok(Outcome::ok(j, text));
    }

    // The fork: the person's, made when they have none.
    if !gh.exists(&fork) {
        if route.fork.is_some() {
            bail!(
                "{fork} isn't a repo gh can see: make it with gh repo fork {} or leave --fork out",
                route.upstream
            );
        }
        gh.run(&[
            "repo",
            "fork",
            route.upstream,
            "--clone=false",
            "--default-branch-only",
        ])?;
        let _ = writeln!(text, "made your fork {fork}");
    }
    let fork_url = gh.url(&fork)?;
    git(&clone, &["remote", "add", "fork", &fork_url])?;
    // The branch is this submission's own, so a submission sent again
    // replaces it.
    git(&clone, &["push", "-q", "--force", "fork", &branch])?;
    let _ = writeln!(text, "pushed {branch} to {fork}");
    let open = gh
        .run(&[
            "pr",
            "list",
            "-R",
            route.upstream,
            "--head",
            &branch,
            "--state",
            "open",
            "--json",
            "url",
            "--jq",
            ".[0].url // empty",
        ])
        .unwrap_or_default();
    let url = if open.is_empty() {
        let f = base_dir.join("pull-request.md");
        fs::write(&f, &body)?;
        let head = format!("{fork_owner}:{branch}");
        gh.run(&[
            "pr",
            "create",
            "-R",
            route.upstream,
            "--base",
            &base,
            "--head",
            &head,
            "--title",
            &title,
            "--body-file",
            &f.to_string_lossy(),
        ])?
        .lines()
        .last()
        .unwrap_or_default()
        .to_owned()
    } else {
        open
    };
    let _ = write!(
        text,
        "pull request: {url}\nThe registry's owner reviews it and merges it or not; once it is merged the page offers {} for {}.",
        s.title,
        if s.for_games.is_empty() {
            "no game (it names none with --for)".to_owned()
        } else {
            s.for_games.join(", ")
        }
    );
    j["submitted"] = json!(true);
    j["pull_request"] = json!(url);
    Ok(Outcome::ok(j, text))
}

/// The pull request's words: what it holds, the checks it passed, and what
/// the owner decides.
fn pr_body(
    s: &Submission,
    published: &Value,
    (granted, listed): (bool, bool),
    commits: &[String],
) -> String {
    let mut b = format!(
        "Submitted with `gp registry submit`.\n\n| | |\n|---|---|\n| Package | `{}@{}` |\n| Kind | {} |\n| Title | {} |\n| For | {} |\n| Id | `{}` |\n\n",
        s.name,
        s.version,
        s.kind,
        s.title,
        if s.for_games.is_empty() {
            "no game".to_owned()
        } else {
            s.for_games
                .iter()
                .map(|g| format!("`{g}`"))
                .collect::<Vec<_>>()
                .join(", ")
        },
        published["id"].as_str().unwrap_or("")
    );
    b.push_str("It passed every check `gp registry publish` runs, on the submitter's machine: each file compared with each ROM the package names, a game with its own pictures compared with every ROM there, and the page's verdict for each game.\n\n");
    if let Some(page) = published["page"].as_array() {
        for p in page.iter().filter_map(Value::as_str) {
            let _ = writeln!(b, "- {p}");
        }
        if !page.is_empty() {
            b.push('\n');
        }
    }
    if granted {
        let ns = s.name.split('/').next().unwrap_or("");
        let _ = writeln!(
            b,
            "The registry didn't grant the namespace `{ns}`, so this pull request grants it (`gp registry grant`): merging it lets packages named `{ns}/...` install from the registry.\n"
        );
    }
    if !listed {
        let _ = writeln!(
            b,
            "The list files aren't changed here: the owner lists it after merging with `gp registry reindex TARGET --keep {}@{} --key SIGNING_KEY`.\n",
            s.name, s.version
        );
    }
    b.push_str("Commits:\n\n");
    for c in commits {
        let _ = writeln!(b, "- {c}");
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stage_leaves_out_hidden_files() {
        let d = tempfile::tempdir().unwrap();
        let (from, to) = (d.path().join("from"), d.path().join("to"));
        fs::create_dir_all(from.join("theme/.cache")).unwrap();
        fs::create_dir_all(from.join(".git")).unwrap();
        fs::write(from.join("theme/look.txt"), "made up\n").unwrap();
        fs::write(from.join("theme/.cache/x"), "x").unwrap();
        fs::write(from.join(".gitignore"), "x").unwrap();
        fs::write(from.join(".git/HEAD"), "x").unwrap();
        fs::create_dir_all(&to).unwrap();
        let left = stage(&from, &to).unwrap();
        assert_eq!(left, [".git", ".gitignore", "theme/.cache"]);
        assert_eq!(
            fs::read_to_string(to.join("theme/look.txt")).unwrap(),
            "made up\n"
        );
        assert!(!to.join(".git").exists() && !to.join("theme/.cache").exists());
    }

    fn sh(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{args:?}: {o:?}");
    }

    /// A made-up registry in a bare repo (the upstream), a stand-in for gh
    /// that answers for it and for a fork it makes as a second bare repo,
    /// and logs each call.
    struct World {
        d: tempfile::TempDir,
        gh: PathBuf,
    }

    impl World {
        fn new() -> Self {
            let d = tempfile::tempdir().unwrap();
            let up = d.path().join("upstream.git");
            sh(
                d.path(),
                &["init", "-q", "--bare", "-b", "main", "upstream.git"],
            );
            let w = d.path().join("seed");
            sh(d.path(), &["clone", "-q", up.to_str().unwrap(), "seed"]);
            let t = Folder(w.clone());
            let none = gp_package::registry::Signer::none();
            gp_package::registry::init(
                &t,
                "made-up",
                &none,
                &[],
                &[("lex00".into(), vec![])],
                "2026-11-14",
            )
            .unwrap();
            gp_package::registry::start_yanks(
                &t,
                &none,
                "2026-11-14T00:00:00Z",
                "2099-01-01T00:00:00Z",
            )
            .unwrap();
            fs::write(w.join("README.md"), "a made-up registry\n").unwrap();
            sh(&w, &["add", "-A"]);
            sh(
                &w,
                &[
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "user.name=t",
                    "commit",
                    "-q",
                    "-m",
                    "Make it",
                ],
            );
            sh(&w, &["push", "-q", "origin", "HEAD:main"]);
            let gh = d.path().join("gh");
            let root = d.path().display();
            fs::write(
                &gh,
                format!(
                    r#"#!/bin/sh
echo "$*" >> {root}/gh.log
repo_path() {{
  case "$1" in
    lex00/gp-registry) echo {root}/upstream.git ;;
    friend/gp-registry) echo {root}/fork.git ;;
    *) echo {root}/none.git ;;
  esac
}}
case "$1 $2" in
  "api user") echo friend ;;
  "repo view")
    p=$(repo_path "$3")
    [ -d "$p" ] || {{ echo "no such repo" >&2; exit 1; }}
    case "$5" in url) echo "$p" ;; *) basename "$3" ;; esac ;;
  "repo fork") git clone -q --bare {root}/upstream.git {root}/fork.git ;;
  "pr list") echo "" ;;
  "pr create") echo "opening"; echo "https://github.invalid/lex00/gp-registry/pull/1" ;;
  *) echo "the stand-in doesn't know: $*" >&2; exit 1 ;;
esac
"#
                ),
            )
            .unwrap();
            gp_mcp::proc::make_executable(&gh).unwrap();
            World { d, gh }
        }

        fn theme(&self, name: &str) -> PathBuf {
            let t = self.d.path().join(name);
            fs::create_dir_all(t.join("theme")).unwrap();
            fs::create_dir_all(t.join(".git")).unwrap();
            fs::write(t.join("theme/look.txt"), "made up\n").unwrap();
            fs::write(t.join(".git/HEAD"), "x").unwrap();
            t
        }

        fn submit(&self, dir: &Path, dry_run: bool) -> Outcome {
            let games = ["lex00/donkey-kong".to_owned()];
            let work = self.d.path().join("work");
            run(
                &Submission {
                    dir,
                    name: "friend/night",
                    version: "0.9.0",
                    kind: "content",
                    title: "Night",
                    for_games: &games,
                    key: None,
                },
                &Route {
                    upstream: UPSTREAM,
                    fork: None,
                    dry_run,
                    gh: self.gh.to_str().unwrap(),
                    work: &work,
                },
            )
            .unwrap()
        }

        fn log(&self) -> String {
            fs::read_to_string(self.d.path().join("gh.log")).unwrap_or_default()
        }

        fn fork(&self) -> PathBuf {
            self.d.path().join("fork.git")
        }
    }

    fn out(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_owned()
    }

    /// A theme goes to a fork made for it, as one branch with the grant,
    /// the publish and the listing, and a pull request is opened for it;
    /// a dry run before it makes no fork and opens nothing.
    #[test]
    fn a_submission_forks_pushes_and_opens_a_pull_request() {
        let w = World::new();
        let dir = w.theme("night");
        let o = w.submit(&dir, true);
        assert_eq!(o.exit, 0, "{}", o.text);
        assert!(
            o.text.contains("dry run") && o.text.contains("made first with gh repo fork"),
            "{}",
            o.text
        );
        assert!(!w.fork().exists());
        assert!(!w.log().contains("pr create"));

        let o = w.submit(&dir, false);
        assert_eq!(o.exit, 0, "{}", o.text);
        assert_eq!(
            o.json["pull_request"],
            "https://github.invalid/lex00/gp-registry/pull/1"
        );
        assert_eq!(o.json["left_out"], json!([".git"]));
        let branch = "submit/friend-night-0.9.0";
        let log = out(&w.fork(), &["log", "--format=%s", branch]);
        assert_eq!(
            log.lines().take(3).collect::<Vec<_>>(),
            [
                "Rewrite the list files",
                "Publish friend/night@0.9.0",
                "Grant the friend namespace"
            ]
        );
        let by_game = out(
            &w.fork(),
            &["show", &format!("{branch}:by-game/lex00/donkey-kong.toml")],
        );
        assert!(by_game.contains("package = \"friend/night\""), "{by_game}");
        assert!(
            out(
                &w.fork(),
                &["show", &format!("{branch}:namespaces/friend.toml")]
            )
            .contains("granted")
        );
        let log = w.log();
        let create = log.lines().find(|l| l.starts_with("pr create")).unwrap();
        assert!(create.contains("-R lex00/gp-registry --base main --head friend:submit/friend-night-0.9.0 --title Submit friend/night@0.9.0"), "{create}");
        let body =
            fs::read_to_string(w.d.path().join("work/friend-night-0.9.0/pull-request.md")).unwrap();
        assert!(
            body.contains("grants it (`gp registry grant`)")
                && body.contains("Publish friend/night@0.9.0"),
            "{body}"
        );
        // The upstream is as it was: only the owner merges.
        assert_eq!(
            out(
                &w.d.path().join("upstream.git"),
                &["log", "--format=%s", "main"]
            ),
            "Make it"
        );
    }

    /// A check publish runs refuses the package: the submission stops
    /// there, says so plainly, and nothing is forked, pushed or opened.
    #[test]
    fn a_failed_check_stops_before_anything_leaves() {
        let w = World::new();
        let dir = w.theme("bad");
        // A package that says it plays from its own pictures and names the
        // ROMs it is lifted from: publish refuses it.
        fs::create_dir_all(dir.join("play")).unwrap();
        fs::write(
            dir.join("play/page.json"),
            r#"{"title": "Bad", "rom": {"kind": "none"}}"#,
        )
        .unwrap();
        fs::write(dir.join("roms.toml"), "made up\n").unwrap();
        let o = w.submit(&dir, false);
        assert_eq!(o.exit, 1, "{}", o.text);
        assert_eq!(o.json["stopped_at"], "the publish checks");
        assert!(o.text.contains("roms.toml"), "{}", o.text);
        assert!(
            o.text
                .contains("Nothing was pushed to your fork and no pull request was opened"),
            "{}",
            o.text
        );
        assert!(!w.fork().exists());
        let log = w.log();
        assert!(
            !log.contains("repo fork") && !log.contains("pr create"),
            "{log}"
        );
    }
}
