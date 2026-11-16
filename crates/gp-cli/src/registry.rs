//! `gp registry`: make a registry in a folder or an S3 bucket, re-sign its
//! list files, and test one from outside, as a browser would read it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use clap::Subcommand;
use gp_package::registry::{
    Folder, Listed, Publish, Reader, Signer, Target, attest, grant, grants, init, listed, publish,
    reindex, start_yanks, unyank, would_list, yank,
};
use gp_package::sig::{NAMESPACE, fingerprint};
use serde_json::{Value, json};

use crate::Outcome;

#[derive(Subcommand)]
pub enum RegistryCmd {
    /// Serve a folder registry and a built page on this machine only.
    ///
    /// Answers on 127.0.0.1 until stopped with ctrl-C, the page's files
    /// first and then the registry's, with the page's _headers. Point the
    /// page's page.json source at "." to play from this registry.
    ///
    /// Example: gp registry serve $HOME/gp-registry --page target/page-live
    /// --port 8377
    Serve {
        /// The registry folder to serve. Example: $HOME/gp-registry
        dir: std::path::PathBuf,
        /// The built page, served beside the registry: a folder holding
        /// index.html. Example: --page target/page-live
        #[arg(long)]
        page: std::path::PathBuf,
        /// The port on 127.0.0.1. Example: --port 8377
        #[arg(long, default_value_t = 8377)]
        port: u16,
    },
    /// Make a registry in a folder or s3://BUCKET.
    ///
    /// Writes registry.toml and a file for each namespace under namespaces/.
    /// Both are written once, so init can't be run again on a registry that
    /// exists: use gp registry grant to add a namespace later. With no keys
    /// the registry is unsigned: readers check hashes, not signatures.
    ///
    /// Example: gp registry init $HOME/gp-registry --name gp-registry
    /// --namespace lex00
    Init {
        /// Where to make it: a folder, git:PATH (commits, then pushes to
        /// origin) or s3://BUCKET. Example: $HOME/gp-registry
        target: String,
        /// The registry's name, written in registry.toml and in each list
        /// file. Example: --name gp-registry
        #[arg(long)]
        name: String,
        /// The root private key file (ssh), for a signed registry; its .pub
        /// beside it is the key readers pin. Goes with --signing-key.
        /// Example: --root-key ~/.ssh/gp-root
        #[arg(long)]
        root_key: Option<PathBuf>,
        /// The private key file that signs the list files, for a signed
        /// registry; its .pub is read. Goes with --root-key. Example:
        /// --signing-key ~/.ssh/gp-signing
        #[arg(long)]
        signing_key: Option<PathBuf>,
        /// A namespace to grant from the start; repeat for more. NS alone
        /// for an unsigned registry, NS=KEY_FILE for a signed one (the
        /// publisher's key file; its .pub is read). Example: --namespace
        /// lex00
        #[arg(long)]
        namespace: Vec<String>,
        /// On a git:PATH target: commit, and don't push. Example: --no-push
        #[arg(long)]
        no_push: bool,
    },
    /// Publish a folder of files as one exact version.
    ///
    /// Every file under DIR goes in, hidden ones too (.git, .gitignore), so
    /// stage a copy without them first. The package id is the hash of the
    /// files, and a version is published once. A package that names ROMs
    /// is compared with each before it goes into a git: target. The page
    /// doesn't offer a new version until gp registry reindex lists it, and
    /// doesn't install one whose namespace isn't granted (gp registry
    /// grant). On a git: target the checkout must be clean; gp pulls
    /// origin first and commits "Publish NAME@VERSION".
    ///
    /// Example: gp registry publish git:$HOME/gp-registry ./stage --name
    /// friend/night --version 0.9.0 --title "Night" --for acme/rooftop
    /// --dry-run
    Publish {
        /// The registry: a folder, git:PATH (commits, then pushes to origin)
        /// or s3://BUCKET. Example: git:$HOME/gp-registry
        target: String,
        /// The folder to publish, as it should arrive. Example: ./stage
        dir: PathBuf,
        /// The package's name, NAMESPACE/NAME, in lowercase letters,
        /// digits, '-' and '.'. Example: --name friend/night
        #[arg(long)]
        name: String,
        /// This version, never published before under the name. Versions
        /// compare by their dotted numbers (1.0.10 after 1.0.9). Example:
        /// --version 0.9.0
        #[arg(long)]
        version: String,
        /// content (a theme or other content for a game) or game. Example:
        /// --kind content
        #[arg(long, default_value = "content")]
        kind: String,
        /// The name players see, in the page's theme setting. Example:
        /// --title "Night"
        #[arg(long)]
        title: String,
        /// A game this content is for, as NAMESPACE/NAME; repeat for more.
        /// The page offers it for that game after a reindex. Example: --for
        /// acme/rooftop
        #[arg(long = "for")]
        for_games: Vec<String>,
        /// The namespace's signing private key file, for a signed registry;
        /// its .pub beside it is read. Example: --key ~/.ssh/gp-friend
        #[arg(long)]
        key: Option<PathBuf>,
        #[command(flatten)]
        how: Writing,
    },
    /// Rewrite the list files from these name@version entries, signed if
    /// --key is given.
    ///
    /// The list files are catalog.toml (each package's latest), by-game/
    /// (what the page offers for each game) and yanks.toml (kept as it is).
    /// Given versions, they are rebuilt from exactly those, so a listed
    /// version left off would be dropped: that is refused unless it is
    /// named with --drop. With --keep, or with no versions, they start from
    /// what they list now and the versions given are added. On a git:
    /// target the commit is "Rewrite the list files".
    ///
    /// Example: gp registry reindex git:$HOME/gp-registry --keep
    /// friend/night@0.9.2
    Reindex {
        /// The registry: a folder, git:PATH (commits, then pushes to origin)
        /// or s3://BUCKET. Example: git:$HOME/gp-registry
        target: String,
        /// Published versions to list, as NAME@VERSION. Without --keep,
        /// every version that should stay listed. Example:
        /// acme/rooftop@1.0.9 friend/night@0.9.2
        versions: Vec<String>,
        /// Start from every version the list files name now and add the
        /// versions given, so adding one can't drop the others. Given no
        /// versions, reindex does this by itself. Example: --keep
        #[arg(long)]
        keep: bool,
        /// A version the lists name now that may be dropped, as
        /// NAME@VERSION; repeat for more. Without it a reindex that would
        /// drop a listed version is refused and names it. Example: --drop
        /// friend/night@0.9.0
        #[arg(long = "drop", value_name = "NAME@VERSION")]
        drops: Vec<String>,
        /// The signing private key file, for a registry whose list files
        /// are signed; they then expire in 7 days. Example: --key
        /// ~/.ssh/gp-signing
        #[arg(long)]
        key: Option<PathBuf>,
        #[command(flatten)]
        how: Writing,
    },
    /// Grant a namespace in a registry that exists, so packages named
    /// NAMESPACE/... can be installed from it.
    ///
    /// Writes namespaces/NAMESPACE.toml (status "granted") and adds the name
    /// to registry.toml's namespaces list, the same two files `gp registry
    /// init` writes. A namespace already granted is refused and nothing
    /// changes. Granting gives nobody write access: whoever can write to the
    /// target still decides what is published. Without the grant the page
    /// lists a theme in the namespace and then fails to install it.
    ///
    /// Example: gp registry grant git:$HOME/gp-registry friend
    Grant {
        /// The registry: a folder, git:PATH (commits, then pushes to origin)
        /// or s3://BUCKET. Example: git:$HOME/gp-registry
        target: String,
        /// The namespace to grant: lowercase letters, digits, '-' and '.'.
        /// Packages named NAMESPACE/NAME then install. Example: friend
        namespace: String,
        /// The publisher's public key file, for a signed registry: it signs
        /// the namespace's packages. Repeat for more than one. An unsigned
        /// registry needs none. Example: --key friend.pub
        #[arg(long)]
        key: Vec<PathBuf>,
        /// The registry's root private key file, for a signed registry: it
        /// signs the namespace file and registry.toml. Example: --root-key
        /// ~/.ssh/gp-root
        #[arg(long)]
        root_key: Option<PathBuf>,
        #[command(flatten)]
        how: Writing,
    },
    /// List the namespaces a registry has granted, with each one's status
    /// and number of keys.
    ///
    /// Example: gp registry grants $HOME/gp-registry
    Grants {
        /// The registry: a folder, git:PATH, s3://BUCKET or an https address.
        /// Example: https://raw.githubusercontent.com/OWNER/REPO/main
        target: String,
    },
    /// Withdraw a published version: add it to yanks.toml with a reason
    /// and a severity, or take it back out with --undo.
    ///
    /// Published files are never deleted, so a version is withdrawn, not
    /// removed. What the page does with a yanked version depends on the
    /// severity. security: the theme list leaves it out, and installing it
    /// fails with "NAME@VERSION was withdrawn for safety: REASON", also for
    /// a player who installed it before, the next time the page reaches the
    /// registry. notice: the page still lists and installs it and shows
    /// "NAME@VERSION was withdrawn: REASON." under the game. Either way
    /// catalog.toml and by-game still name the version until a reindex
    /// leaves it out, and gp game fetch still fetches it. On a git: target
    /// the commit message is "Withdraw NAME@VERSION".
    ///
    /// Example: gp registry yank git:$HOME/gp-registry friend/night@0.9.0
    /// --severity security --reason "withdrawn by the registry owner"
    Yank {
        /// The registry: a folder, git:PATH (commits, then pushes to origin)
        /// or s3://BUCKET. Example: git:$HOME/gp-registry
        target: String,
        /// The published version, as NAME@VERSION. Example:
        /// friend/night@0.9.0
        version: String,
        /// Why it is withdrawn, in words a player reads on the page.
        /// Example: --reason "withdrawn by the registry owner"
        #[arg(long, required_unless_present = "undo", conflicts_with = "undo")]
        reason: Option<String>,
        /// security (the page hides it and refuses to install it) or notice
        /// (the page still offers it, with the reason shown). Example:
        /// --severity security
        #[arg(
            long,
            value_enum,
            required_unless_present = "undo",
            conflicts_with = "undo"
        )]
        severity: Option<Severity>,
        /// Take the version's entry out of yanks.toml instead, so the page
        /// offers it again. Example: --undo
        #[arg(long)]
        undo: bool,
        /// The signing private key file, for a registry whose list files
        /// are signed. Example: --key ~/.ssh/gp-signing
        #[arg(long)]
        key: Option<PathBuf>,
        #[command(flatten)]
        how: Writing,
    },
    /// Test a registry from outside, against its root if --root is given.
    ///
    /// Reads it as a player's browser would: CORS (over https), registry.toml,
    /// yanks.toml, catalog.toml, and every package the catalog lists, fetched
    /// by its exact id with every hash checked. Prints one ok or FAILED line
    /// each and exits 1 on any failure. Writes nothing.
    ///
    /// With --for GAME it also reads the game's theme list as the page does
    /// (the page's own code, gp-web-registry): each theme the page would
    /// offer in Settings, Theme, installed as the page installs it, and
    /// each one it would leave out, with why. With --theme NAME too it says
    /// whether that theme is offered for the game and, if not, why, and
    /// exits 1 when it isn't; when the game's folder and your ROM are here
    /// it also reads the theme's files as the page's lift does (gp theme
    /// look check's "On the page" lines).
    ///
    /// Example: gp registry check
    /// https://raw.githubusercontent.com/lex00/gp-registry/main
    ///
    /// Example: gp registry check $HOME/gp-registry --for
    /// acme/rooftop --theme friend/night
    Check {
        /// The registry: a folder, git:PATH, s3://BUCKET or an https
        /// address. Example: $HOME/gp-registry
        url: String,
        /// A file holding the root's public key, to check signatures and
        /// expiries; without it they aren't looked at. Example: --root
        /// root.pub
        #[arg(long)]
        root: Option<PathBuf>,
        /// A game, NAMESPACE/NAME: read its theme list as the page does,
        /// with what the page offers and what it leaves out and why.
        /// Example: --for acme/rooftop
        #[arg(long = "for", value_name = "GAME")]
        for_game: Option<String>,
        /// A theme, NAMESPACE/NAME, with --for: is it offered for the game,
        /// and if not, why. Exits 1 when it isn't. Example: --theme
        /// friend/night
        #[arg(long, requires = "for_game")]
        theme: Option<String>,
        /// Your ROM of the game, for --theme's reading of the theme's files
        /// as the page's lift does (default: one under $GP_ROM_DIR the game
        /// accepts). Example: --rom "$GP_ROM_DIR/Rooftop (U).nes"
        #[arg(long, requires = "theme")]
        rom: Option<PathBuf>,
    },
    /// Where hosting your own registry on GitHub stands, step by step, and
    /// its raw address for a page.
    ///
    /// Reads the registry's checkout (DIR, a clone of a public GitHub repo)
    /// and says, as next does for a port, which steps of the road are done
    /// and which is now: the repo, the registry in it (gp registry init), your
    /// namespace (grant), your theme (publish and reindex), the check from
    /// outside (gp registry check), the public page with r= and a page of
    /// your own. A page installs games from its own registry and takes only
    /// themes from yours, so yours holds no game. Prints the repo's raw
    /// address (https://raw.githubusercontent.com/OWNER/REPO/BRANCH), which
    /// the public page takes as r= and your own page as its second registry
    /// of themes (GP_PAGE_THEMES_URL), and says whether the checkout has
    /// commits GitHub doesn't have yet. Writes nothing; asks git what origin
    /// holds. Without DIR it gives the first step. docs/host-your-own.md.
    ///
    /// Example: gp registry host ~/acme-registry --theme acme/harbor-lights
    /// --game acme/rooftop
    Host {
        /// The registry's checkout: a clone of its GitHub repo. Example:
        /// ~/acme-registry
        dir: Option<PathBuf>,
        /// The theme to publish, NAMESPACE/NAME; its namespace is the one to
        /// grant. Example: --theme acme/harbor-lights
        #[arg(long)]
        theme: Option<String>,
        /// The theme's folder, for the publish command it shows. Example:
        /// --theme-dir ~/.gameprinter/work/themes/harbor-lights
        #[arg(long)]
        theme_dir: Option<PathBuf>,
        /// The game the theme is for, NAMESPACE/NAME (default: the first
        /// game the published theme names). Example: --game
        /// acme/rooftop
        #[arg(long)]
        game: Option<String>,
        /// The GitHub repo, OWNER/REPO, when the checkout's origin isn't a
        /// GitHub address. Example: --github acme/harbor-registry
        #[arg(long)]
        github: Option<String>,
        /// The page to try it on, with r=. Example: --page
        /// https://gameprinter.intentius.workers.dev
        #[arg(long, default_value = crate::journeys::PUBLIC_PAGE)]
        page: String,
    },
    /// Offer a game or theme to a registry you can't write to that takes
    /// submissions, as a pull request its owner merges or not. Name its repo
    /// with --to; lex00/gp-registry takes no one else's themes (host your
    /// own: docs/host-your-own.md).
    ///
    /// Stages DIR without its hidden files (.git, .gitignore), clones the
    /// registry, and on a branch of its own runs what the owner would run:
    /// gp registry grant for a namespace the registry doesn't grant yet (an
    /// unsigned registry only), gp registry publish, with every check it
    /// runs (each file compared with each ROM the package names, a game with
    /// its own pictures with every ROM in $GP_ROM_DIR), and gp registry
    /// reindex --keep, so the pull request holds the package and its lines
    /// in the list files. Then it pushes the branch to your fork (made with
    /// gh repo fork when you have none) and opens the pull request with gh.
    /// If a check fails, it stops there and says which: nothing is pushed,
    /// no pull request is opened, and the same version can be sent again
    /// once it is fixed. Sending the same version again replaces the branch
    /// and keeps its pull request. Needs gh logged in to GitHub.
    ///
    /// Example: gp registry submit ./my-theme --name friend/night --version
    /// 0.9.0 --title "Night" --for acme/rooftop --to acme/harbor-registry
    /// --dry-run
    Submit {
        /// The folder to submit, as it should arrive. Example: ./my-theme
        dir: PathBuf,
        /// The package's name, NAMESPACE/NAME; the namespace is usually
        /// your GitHub login. Example: --name friend/night
        #[arg(long)]
        name: String,
        /// This version, never published before under the name. Example:
        /// --version 0.9.0
        #[arg(long)]
        version: String,
        /// content (a theme) or game. Example: --kind content
        #[arg(long, default_value = "content")]
        kind: String,
        /// The name players see. Example: --title "Night"
        #[arg(long)]
        title: String,
        /// A game this content is for, as NAMESPACE/NAME; repeat for more.
        /// Example: --for acme/rooftop
        #[arg(long = "for")]
        for_games: Vec<String>,
        /// The registry's GitHub repo, OWNER/REPO; required. Example: --to
        /// acme/harbor-registry
        #[arg(long)]
        to: String,
        /// Your fork to push to, OWNER/REPO (default: YOUR-LOGIN/REPO,
        /// made when it isn't there). Example: --fork friend/gp-registry
        #[arg(long)]
        fork: Option<String>,
        /// Your namespace's signing private key file, for a signed
        /// registry. Example: --key ~/.ssh/gp-friend
        #[arg(long)]
        key: Option<PathBuf>,
        /// Run every check and make the commits in the clone, and stop:
        /// nothing is forked, pushed or opened. Example: --dry-run
        #[arg(long)]
        dry_run: bool,
    },
}

/// How a command that writes to a registry writes.
#[derive(clap::Args, Clone, Copy, Debug, Default)]
pub struct Writing {
    /// Do everything except write: print the files that would be added,
    /// the package id and the list file lines that would change. On a git:
    /// target nothing is pulled, committed or pushed. Example: --dry-run
    #[arg(long)]
    pub dry_run: bool,
    /// On a git:PATH target: commit, and don't push, so the commit can be
    /// looked at first and pushed by hand (gp prints how). The next gp
    /// command on that checkout without it pushes it along with its own.
    /// Example: --no-push
    #[arg(long)]
    pub no_push: bool,
}

/// A yank's severity: the two the page tells apart.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Severity {
    /// The page hides it and refuses to install it.
    Security,
    /// The page still offers it and shows the reason.
    Notice,
}

impl Severity {
    fn word(self) -> &'static str {
        match self {
            Severity::Security => "security",
            Severity::Notice => "notice",
        }
    }
}

pub fn run(cmd: &RegistryCmd) -> Result<Outcome> {
    match cmd {
        RegistryCmd::Serve { dir, page, port } => serve(dir, page, *port),
        RegistryCmd::Init {
            target,
            name,
            root_key,
            signing_key,
            namespace,
            no_push,
        } => init_cmd(
            target,
            name,
            root_key.as_deref(),
            signing_key.as_deref(),
            namespace,
            !*no_push,
        ),
        RegistryCmd::Publish {
            target,
            dir,
            name,
            version,
            kind,
            title,
            for_games,
            key,
            how,
        } => publish_cmd(
            target,
            dir,
            (name, version),
            (kind, title),
            for_games,
            key.as_deref(),
            *how,
        ),
        RegistryCmd::Reindex {
            target,
            versions,
            keep,
            drops,
            key,
            how,
        } => reindex_cmd(target, (versions, *keep, drops), key.as_deref(), *how),
        RegistryCmd::Grant {
            target,
            namespace,
            key,
            root_key,
            how,
        } => grant_cmd(target, namespace, key, root_key.as_deref(), *how),
        RegistryCmd::Grants { target } => grants_cmd(target),
        RegistryCmd::Yank {
            target,
            version,
            reason,
            severity,
            undo,
            key,
            how,
        } => yank_cmd(
            target,
            version,
            reason.as_deref().zip(severity.map(Severity::word)),
            *undo,
            key.as_deref(),
            *how,
        ),
        RegistryCmd::Check {
            url,
            root,
            for_game,
            theme,
            rom,
        } => {
            let mut o = check_cmd(url, root.as_deref())?;
            if let Some(game) = for_game {
                offered_cmd(
                    &mut o,
                    url,
                    root.as_deref(),
                    game,
                    theme.as_deref(),
                    rom.as_deref(),
                )?;
            }
            Ok(o)
        }
        RegistryCmd::Host {
            dir,
            theme,
            theme_dir,
            game,
            github,
            page,
        } => crate::registry_host::host(&crate::registry_host::HostArgs {
            dir: dir.as_deref(),
            github: github.as_deref(),
            theme: theme.as_deref(),
            theme_dir: theme_dir.as_deref(),
            game: game.as_deref(),
            page,
        }),
        RegistryCmd::Submit {
            dir,
            name,
            version,
            kind,
            title,
            for_games,
            to,
            fork,
            key,
            dry_run,
        } => {
            let paths = gp_evidence::Paths::from_env()?;
            let gh = std::env::var("GP_GH").unwrap_or_else(|_| "gh".into());
            let work = paths.home.join("work").join("registry-submit");
            crate::registry_submit::run(
                &crate::registry_submit::Submission {
                    dir,
                    name,
                    version,
                    kind,
                    title,
                    for_games,
                    key: key.as_deref(),
                },
                &crate::registry_submit::Route {
                    upstream: to,
                    fork: fork.as_deref(),
                    dry_run: *dry_run,
                    gh: &gh,
                    work: &work,
                },
            )
        }
    }
}

/// A registry in an S3 bucket, through the AWS CLI and its own login, so
/// gp never holds an AWS key.
pub struct S3 {
    bucket: String,
}

impl S3 {
    fn put(&self, key: &str, bytes: &[u8], once: bool) -> Result<Option<String>, String> {
        let tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        fs::write(tmp.path(), bytes).map_err(|e| e.to_string())?;
        let mut c = Command::new("aws");
        c.args([
            "s3api",
            "put-object",
            "--bucket",
            &self.bucket,
            "--key",
            key,
            "--body",
        ])
        .arg(tmp.path());
        // A write-once key never changes, so a browser keeps it for good. A
        // list file is rewritten, so a browser asks each time.
        if once {
            c.args(["--if-none-match", "*"]);
            c.args(["--cache-control", "public, max-age=31536000, immutable"]);
        } else {
            c.args(["--cache-control", "no-cache"]);
        }
        let o = c.output().map_err(|e| format!("running aws: {e}"))?;
        if o.status.success() {
            return Ok(None);
        }
        Ok(Some(String::from_utf8_lossy(&o.stderr).trim().to_owned()))
    }
}

impl Target for S3 {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let o = Command::new("aws")
            .args([
                "s3api",
                "get-object",
                "--bucket",
                &self.bucket,
                "--key",
                key,
            ])
            .arg(tmp.path())
            .output()
            .map_err(|e| format!("running aws: {e}"))?;
        if o.status.success() {
            return fs::read(tmp.path()).map(Some).map_err(|e| e.to_string());
        }
        let err = String::from_utf8_lossy(&o.stderr);
        if err.contains("NoSuchKey") {
            return Ok(None);
        }
        Err(format!("{key}: {}", err.trim()))
    }

    fn write_once(&self, key: &str, bytes: &[u8]) -> Result<bool, String> {
        match self.put(key, bytes, true)? {
            None => Ok(true),
            Some(e) if e.contains("PreconditionFailed") => Ok(false),
            Some(e) => Err(format!("{key}: {e}")),
        }
    }

    fn replace(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        match self.put(key, bytes, false)? {
            None => Ok(()),
            Some(e) => Err(format!("{key}: {e}")),
        }
    }
}

/// A registry read over HTTPS, as a player reads it.
pub struct Http(pub String);

impl Target for Http {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let url = format!("{}/{key}", self.0.trim_end_matches('/'));
        let tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let o = Command::new("curl")
            .args(["-sS", "--max-time", "60", "-o"])
            .arg(tmp.path())
            .args(["-w", "%{http_code}", &url])
            .output()
            .map_err(|e| format!("running curl: {e}"))?;
        let code = String::from_utf8_lossy(&o.stdout).trim().to_owned();
        match code.as_str() {
            "200" => fs::read(tmp.path()).map(Some).map_err(|e| e.to_string()),
            // A public bucket with no list permission answers 403 for a key
            // it doesn't hold.
            "404" | "403" => Ok(None),
            _ => Err(format!("{url}: HTTP {code}")),
        }
    }

    fn write_once(&self, key: &str, _: &[u8]) -> Result<bool, String> {
        Err(format!("{key}: an https registry is read only"))
    }

    fn replace(&self, key: &str, _: &[u8]) -> Result<(), String> {
        Err(format!("{key}: an https registry is read only"))
    }
}

/// A registry in a git checkout, named `git:PATH`: a folder whose changes each
/// command commits, and pushes when the repo has an `origin`. It's a folder
/// for writing, so a write-once key is refused the same way, and a repo host
/// serves it as static files.
pub struct Repo(PathBuf);

impl Repo {
    fn at(spec: &str) -> Option<Self> {
        spec.strip_prefix("git:").map(|p| Self(home_path(p)))
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let o = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .output()
            .context("running git")?;
        if !o.status.success() {
            anyhow::bail!(
                "git {} in {}: {}",
                args.join(" "),
                self.0.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
    }

    fn has_origin(&self) -> Result<bool> {
        Ok(self.git(&["remote"])?.lines().any(|r| r == "origin"))
    }

    /// Before a command writes: a clean checkout, at the remote's tip, so a
    /// key someone else published counts as taken.
    fn ready(&self) -> Result<()> {
        self.git(&["rev-parse", "--is-inside-work-tree"])?;
        let dirty = self.git(&["status", "--porcelain"])?;
        if !dirty.is_empty() {
            anyhow::bail!(
                "{} has changes not committed, so nothing is published from it",
                self.0.display()
            );
        }
        if self.has_origin()? && !self.git(&["ls-remote", "--heads", "origin"])?.is_empty() {
            self.git(&["pull", "-q", "--ff-only", "origin", "HEAD"])?;
        }
        Ok(())
    }

    /// After it: one commit for what the command wrote, then the push
    /// unless `push` is false. Returns a sentence saying what was committed
    /// and pushed where.
    fn commit(&self, message: &str, push: bool) -> Result<String> {
        let at = self.0.display();
        self.git(&["add", "-A"])?;
        if self.git(&["status", "--porcelain"])?.is_empty() {
            return Ok(format!(
                "Nothing changed in {at}, so nothing was committed or pushed."
            ));
        }
        self.git(&["commit", "-q", "-m", message])?;
        let commit = self.git(&["rev-parse", "--short", "HEAD"])?;
        if !self.has_origin()? {
            return Ok(format!(
                "Committed {commit} \"{message}\" in {at}. It has no origin, so nothing was pushed."
            ));
        }
        if !push {
            return Ok(format!(
                "Committed {commit} \"{message}\" in {at} and did not push it. Look with: git -C {at} show --stat HEAD. Push with: git -C {at} push origin HEAD"
            ));
        }
        self.git(&["push", "-q", "origin", "HEAD"])?;
        let url = self.git(&["remote", "get-url", "origin"])?;
        let branch = self.git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        Ok(format!(
            "Committed {commit} \"{message}\" in {at} and pushed it to origin ({url}), branch {branch}."
        ))
    }
}

/// A path with a leading `~/` read as the home folder, since the shell
/// doesn't expand it inside `git:~/...`.
fn home_path(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(p),
    }
}

/// `f`, with a repo target made ready first, and committed and pushed after.
fn committing(target: &str, message: &str, f: impl FnOnce() -> Result<Outcome>) -> Result<Outcome> {
    committing_as(target, message, true, f)
}

/// [`committing`], pushing only when `push` is true. Refuses `push` false
/// on a target that isn't git:PATH, which is written at once.
fn committing_as(
    target: &str,
    message: &str,
    push: bool,
    f: impl FnOnce() -> Result<Outcome>,
) -> Result<Outcome> {
    let Some(repo) = Repo::at(target) else {
        if !push {
            anyhow::bail!(
                "--no-push is for a git:PATH target; {target} is written at once (use --dry-run to look first)"
            );
        }
        return f();
    };
    repo.ready()?;
    let mut outcome = f()?;
    if outcome.exit == 0 {
        let said = repo.commit(message, push)?;
        outcome.text = format!("{}\n{said}", outcome.text);
        outcome.json["git"] = json!(said);
    }
    Ok(outcome)
}

/// `f` on the target: for real through [`committing`], or with
/// `--dry-run` on a copy held in memory over it, and then what it would
/// have written.
pub(crate) fn writing(
    target: &str,
    message: &str,
    how: Writing,
    f: impl FnOnce(&dyn Target) -> Result<Outcome>,
) -> Result<Outcome> {
    if !how.dry_run {
        return committing_as(target, message, !how.no_push, || f(&*open(target)));
    }
    let inner = open(target);
    let dry = Dry::over(&*inner);
    let mut o = f(&dry)?;
    let (text, would) = dry.report()?;
    let git = if Repo::at(target).is_some() {
        format!(
            " On a real run this would be committed as \"{message}\" and pushed; the checkout was not pulled first."
        )
    } else {
        String::new()
    };
    o.text = format!(
        "dry run: nothing was written.{git}\nIt would have: {}\n{text}",
        o.text
    );
    o.json["dry_run"] = json!(true);
    o.json["would"] = would;
    Ok(o)
}

/// A target that holds every write in memory and reads through to the
/// real one, so a command runs in full and nothing changes.
struct Dry<'a> {
    inner: &'a dyn Target,
    /// Each key written, in order: its bytes and whether it is write-once.
    writes: std::cell::RefCell<Vec<(String, Vec<u8>, bool)>>,
    /// Write-once keys that were already there.
    kept: std::cell::RefCell<Vec<String>>,
}

impl<'a> Dry<'a> {
    fn over(inner: &'a dyn Target) -> Self {
        Self {
            inner,
            writes: Default::default(),
            kept: Default::default(),
        }
    }

    fn held(&self, key: &str) -> Option<Vec<u8>> {
        self.writes
            .borrow()
            .iter()
            .rev()
            .find(|(k, _, _)| k == key)
            .map(|(_, b, _)| b.clone())
    }

    /// What it would have written, as text and as JSON.
    fn report(&self) -> Result<(String, serde_json::Value)> {
        let mut lines = Vec::new();
        let (mut blobs, mut blobs_kept) = (0usize, 0usize);
        let mut added = Vec::new();
        let mut changed = Vec::new();
        for k in self.kept.borrow().iter() {
            if k.starts_with("blobs/") {
                blobs_kept += 1;
            }
        }
        for (key, bytes, once) in self.writes.borrow().iter() {
            if *once {
                if key.starts_with("blobs/") {
                    blobs += 1;
                } else {
                    added.push(key.clone());
                    lines.push(format!("add     {key}"));
                }
                continue;
            }
            let old = self.inner.read(key).map_err(|e| anyhow!(e))?;
            if old.as_deref() == Some(bytes.as_slice()) {
                lines.push(format!("same    {key}"));
                continue;
            }
            let new_text = String::from_utf8_lossy(bytes);
            let old_text = old
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            lines.push(format!(
                "{} {key}",
                if old.is_some() { "change " } else { "add    " }
            ));
            let diff = line_diff(&old_text, &new_text);
            for d in &diff {
                lines.push(format!("          {d}"));
            }
            changed.push(json!({ "key": key, "new": old.is_none(), "lines": diff }));
        }
        if blobs + blobs_kept > 0 {
            lines.insert(
                0,
                format!("add     {blobs} blobs ({blobs_kept} already there)"),
            );
        }
        if lines.is_empty() {
            lines.push("no file would change".into());
        }
        Ok((
            lines.join("\n"),
            json!({ "blobs": blobs, "blobs_there": blobs_kept, "added": added, "rewritten": changed }),
        ))
    }
}

impl Target for Dry<'_> {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match self.held(key) {
            Some(b) => Ok(Some(b)),
            None => self.inner.read(key),
        }
    }

    fn write_once(&self, key: &str, bytes: &[u8]) -> Result<bool, String> {
        if self.read(key)?.is_some() {
            self.kept.borrow_mut().push(key.to_owned());
            return Ok(false);
        }
        self.writes
            .borrow_mut()
            .push((key.to_owned(), bytes.to_vec(), true));
        Ok(true)
    }

    fn replace(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        let mut w = self.writes.borrow_mut();
        w.retain(|(k, _, once)| k != key || *once);
        w.push((key.to_owned(), bytes.to_vec(), false));
        Ok(())
    }
}

/// The lines taken out ("- ") and put in ("+ ") from `old` to `new`, by
/// their longest common run of lines.
fn line_diff(old: &str, new: &str) -> Vec<String> {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let mut len = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            len[i][j] = if a[i] == b[j] {
                len[i + 1][j + 1] + 1
            } else {
                len[i + 1][j].max(len[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || len[i + 1][j] >= len[i][j + 1]) {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    out
}

/// Nothing goes into a repo with a ROM's bytes in it: a repo's history keeps
/// what it's given. Every file of a package that names ROMs is compared with
/// each of them (found under `roms`), and a ROM not found there stops the
/// publish. Returns the line saying what it compared, as gp package scan
/// says it: "ROM comparison: done against" and the entries' ids.
fn no_rom_bytes(dir: &Path, roms: &Path) -> Result<String> {
    if !dir.join("package.toml").exists() {
        return Ok(
            "ROM comparison: this folder has no package.toml, so it names no ROM to compare with"
                .into(),
        );
    }
    let pkg = crate::package::Package::load(dir)?;
    let wanted = pkg.rom_hashes();
    if wanted.is_empty() {
        return Ok("ROM comparison: this package names no ROM".into());
    }
    let found = crate::roms::find(&wanted, &[], roms)?;
    if let Some(gone) = wanted.iter().find(|h| !found.contains_key(*h)) {
        anyhow::bail!(
            "ROM {gone} isn't in $GP_ROM_DIR, so this package can't be compared with it before going into a repo"
        );
    }
    let mut files = 0usize;
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            files += 1;
            let bytes = fs::read(&p)?;
            for (id, rom) in &found {
                if let Some((at, len)) = crate::served::runs_of_rom(&bytes, rom.data()).first() {
                    anyhow::bail!(
                        "{} holds {len} bytes of ROM {} at ROM offset {at:#x}; nothing was published",
                        p.display(),
                        &id[..12]
                    );
                }
            }
        }
    }
    let mut ids: Vec<&str> = pkg.entries.iter().map(|e| e.id.as_str()).collect();
    ids.dedup();
    Ok(format!(
        "ROM comparison: done against {}: none of the {files} files holds a run of their bytes",
        ids.join(", ")
    ))
}

/// A folder, `git:PATH`, `s3://BUCKET` or an https address.
pub fn open(spec: &str) -> Box<dyn Target> {
    if let Some(repo) = Repo::at(spec) {
        Box::new(Folder(repo.0))
    } else if let Some(bucket) = spec.strip_prefix("s3://") {
        Box::new(S3 {
            bucket: bucket.trim_end_matches('/').to_owned(),
        })
    } else if spec.starts_with("https://") || spec.starts_with("http://") {
        Box::new(Http(spec.to_owned()))
    } else {
        Box::new(Folder(home_path(spec)))
    }
}

/// The folder a registry spec reads from on this machine (a folder or
/// git:PATH), or None for s3:// and https, which can't be listed.
pub(crate) fn local_folder(spec: &str) -> Option<PathBuf> {
    if let Some(repo) = Repo::at(spec) {
        Some(repo.0)
    } else if spec.starts_with("s3://")
        || spec.starts_with("https://")
        || spec.starts_with("http://")
    {
        None
    } else {
        Some(home_path(spec))
    }
}

/// The public key in a `--root` file.
pub(crate) fn read_root(file: &Path) -> Result<String> {
    Ok(fs::read_to_string(file)
        .with_context(|| file.display().to_string())?
        .trim()
        .to_owned())
}

pub(crate) fn public_of(key: &Path) -> Result<String> {
    let p = PathBuf::from(format!("{}.pub", key.display()));
    Ok(fs::read_to_string(&p)
        .with_context(|| format!("reading {}", p.display()))?
        .trim()
        .to_owned())
}

fn ssh_sign(key: &Path, data: &[u8]) -> Result<Vec<u8>, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let f = dir.path().join("d");
    fs::write(&f, data).map_err(|e| e.to_string())?;
    let o = Command::new("ssh-keygen")
        .args(["-q", "-Y", "sign", "-n", NAMESPACE, "-f"])
        .arg(key)
        .arg(&f)
        .output()
        .map_err(|e| format!("running ssh-keygen: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_owned());
    }
    fs::read(f.with_extension("sig")).map_err(|e| e.to_string())
}

/// `f` given a signer for the key file, or one that signs nothing.
pub(crate) fn signing<T>(
    key: Option<&Path>,
    f: impl FnOnce(&Signer) -> Result<T, String>,
) -> Result<T> {
    let Some(key) = key else {
        return f(&Signer::none()).map_err(|e| anyhow!(e));
    };
    let public = public_of(key)?;
    let path = key.to_owned();
    let sign = move |d: &[u8]| ssh_sign(&path, d);
    f(&Signer {
        public,
        sign: &sign,
    })
    .map_err(|e| anyhow!(e))
}

/// The UTC time, whole seconds, in the registry's RFC 3339 form.
pub(crate) fn utc(after_days: u32) -> Result<String> {
    let o = Command::new("date")
        .args([
            "-u",
            "-v",
            &format!("+{after_days}d"),
            "+%Y-%m-%dT%H:%M:%SZ",
        ])
        .output();
    let o = match o {
        Ok(o) if o.status.success() => o,
        _ => Command::new("date")
            .args([
                "-u",
                "-d",
                &format!("+{after_days} days"),
                "+%Y-%m-%dT%H:%M:%SZ",
            ])
            .output()
            .context("running date")?,
    };
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

pub fn init_cmd(
    target: &str,
    name: &str,
    root_key: Option<&Path>,
    signing_key: Option<&Path>,
    namespaces: &[String],
    push: bool,
) -> Result<Outcome> {
    committing_as(target, &format!("Make registry {name}"), push, || {
        init_inner(target, name, root_key, signing_key, namespaces)
    })
}

fn init_inner(
    target: &str,
    name: &str,
    root_key: Option<&Path>,
    signing_key: Option<&Path>,
    namespaces: &[String],
) -> Result<Outcome> {
    let t = open(target);
    let mut ns = Vec::new();
    for n in namespaces {
        match n.split_once('=') {
            Some((name, key)) => ns.push((name.to_owned(), vec![public_of(Path::new(key))?])),
            None if root_key.is_none() => ns.push((n.clone(), Vec::new())),
            None => return Err(anyhow!("--namespace wants NS=KEY_FILE, not {n}")),
        }
    }
    let today = utc(0)?[..10].to_owned();
    let (root_key, signing_key) = match (root_key, signing_key) {
        (Some(r), Some(s)) => (r, s),
        (None, None) => {
            init(&*t, name, &Signer::none(), &[], &ns, &today).map_err(|e| anyhow!(e))?;
            // An empty yank list, as a reindex writes it: a reader refuses
            // a registry with none.
            let (now, expires) = (utc(0)?, utc(7)?);
            start_yanks(&*t, &Signer::none(), &now, &expires).map_err(|e| anyhow!(e))?;
            return Ok(Outcome {
                json: json!({ "name": name, "root": null }),
                text: format!("made registry {name} at {target}, not signed"),
                exit: 0,
            });
        }
        _ => return Err(anyhow!("--root-key and --signing-key go together")),
    };
    let index = public_of(signing_key)?;
    signing(Some(root_key), |r| {
        init(&*t, name, r, &[index], &ns, &today)
    })?;
    // The yank list is a list file: the signing key signs it, with the
    // expiry a reindex gives it.
    let (now, expires) = (utc(0)?, utc(7)?);
    signing(Some(signing_key), |s| start_yanks(&*t, s, &now, &expires))?;
    let fp = fingerprint(&public_of(root_key)?).map_err(|e| anyhow!(e))?;
    Ok(Outcome {
        json: json!({ "name": name, "root": fp }),
        text: format!("made registry {name} at {target}; its root is {fp}"),
        exit: 0,
    })
}

/// Publishes a folder of text files as one exact version.
pub fn publish_cmd(
    target: &str,
    dir: &Path,
    (name, version): (&str, &str),
    (kind, title): (&str, &str),
    for_games: &[String],
    key: Option<&Path>,
    how: Writing,
) -> Result<Outcome> {
    let paths = gp_evidence::Paths::from_env()?;
    let compared = if Repo::at(target).is_some() {
        no_rom_bytes(dir, &paths.roms)?
    } else {
        format!(
            "ROM comparison: not run; a package is compared with its ROMs only before it goes into a git: target, and {target} isn't one"
        )
    };
    let own = own_pictures_guard(dir, &paths.roms, &paths.home)?;
    let exact = crate::publish_rule::arcade_exact_guard(dir, kind)?;
    let mut o = writing(target, &format!("Publish {name}@{version}"), how, |t| {
        publish_inner(t, dir, (name, version), (kind, title), for_games, key)
    })?;
    o.text.push('\n');
    o.text.push_str(&compared);
    o.json["rom_comparison"] = json!(compared);
    if let Some(line) = own {
        o.text.push('\n');
        o.text.push_str(&line);
        o.json["own_pictures"] = json!(line);
    }
    if let Some(line) = exact {
        o.text.push('\n');
        o.text.push_str(&line);
        o.json["arcade_exact"] = json!(line);
    }
    Ok(o)
}

/// A game package that says it plays from its own pictures (its
/// play/page.json's `rom.kind` is `none`: a new game, made here) holds
/// nothing of a ROM: it names none, has no lift, and no file of it is a
/// game image, holds a run of a ROM's bytes or is a file lifted from one,
/// compared with every ROM in `roms` and every lifted folder under `home`
/// but its own build's (crate::served::own_pictures). Any of them stops the
/// publish before anything is written. Returns a line saying what it was
/// compared with, or None for a package that makes no such claim.
pub(crate) fn own_pictures_guard(dir: &Path, roms: &Path, home: &Path) -> Result<Option<String>> {
    let Ok(facts) = fs::read(dir.join("play").join("page.json")) else {
        return Ok(None);
    };
    let v: Value = serde_json::from_slice(&facts).unwrap_or_default();
    if v["rom"]["kind"] != "none" {
        return Ok(None);
    }
    let what = v["title"].as_str().unwrap_or("this game").to_owned();
    let stop = |why: String| {
        anyhow!(
            "{what} says it plays from its own pictures (play/page.json's rom.kind is none), but {why}; nothing was published"
        )
    };
    if dir.join("package.toml").is_file() && !crate::package::Package::load(dir)?.entries.is_empty()
    {
        return Err(stop("its package.toml names a ROM".into()));
    }
    if dir.join("roms.toml").is_file() {
        return Err(stop(
            "it has a roms.toml naming the ROMs it is lifted from".into(),
        ));
    }
    if dir.join("play").join("lift").exists() {
        return Err(stop(
            "its play/ holds a lift (play/lift/), which reads a ROM".into(),
        ));
    }
    let mut files = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let e = e?;
            let p = e.path();
            if e.file_type()?.is_dir() {
                if e.file_name() != ".git" {
                    stack.push(p);
                }
            } else {
                let rel = p.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
                files.push((rel, fs::read(&p)?));
            }
        }
    }
    files.sort();
    let (builds, source) = crate::served::own_builds(dir);
    let (problems, compared) =
        crate::served::own_pictures(&files, (&builds, source.as_deref()), roms, home)?;
    if let Some(first) = problems.first() {
        let more = match problems.len() - 1 {
            0 => String::new(),
            1 => " (and 1 more finding)".into(),
            n => format!(" (and {n} more findings)"),
        };
        return Err(stop(format!(
            "{}: {}{more}",
            first["file"].as_str().unwrap_or(""),
            first["message"].as_str().unwrap_or("")
        )));
    }
    Ok(Some(if compared.is_empty() {
        format!(
            "{what} plays from its own pictures; there was no ROM or lifted folder here to compare it with"
        )
    } else {
        format!(
            "{what} plays from its own pictures: none of its {} files is a game image or holds anything of {}",
            files.len(),
            compared.join(", ")
        )
    }))
}

fn publish_inner(
    t: &dyn Target,
    dir: &Path,
    (name, version): (&str, &str),
    (kind, title): (&str, &str),
    for_games: &[String],
    key: Option<&Path>,
) -> Result<Outcome> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
                files.insert(rel, fs::read(&p)?);
            }
        }
    }
    let published = utc(0)?;
    let p = Publish {
        name,
        version,
        kind,
        title,
        for_games,
        published: &published,
    };
    let id = signing(key, |s| publish(t, &p, &files, s))?;
    let yanks_note = no_yank_list(t, key)?;
    let page = if kind == "content" {
        page_would(t, name, version, title, for_games, &id)
    } else {
        Vec::new()
    };
    let mut text = format!("published {name}@{version}: {id} ({} files)", files.len());
    for l in yanks_note.iter().chain(&page) {
        text.push('\n');
        text.push_str(l);
    }
    Ok(Outcome {
        json: json!({ "name": name, "version": version, "id": id, "files": files.len(), "page": page }),
        text,
        exit: 0,
    })
}

/// Leaves the registry with a yank list after a publish: writes an empty
/// one when there is none and `key` may sign it (an unsigned registry, or
/// a key among its signing keys). Otherwise says what writes it, since a
/// reader refuses a registry with none.
fn no_yank_list(t: &dyn Target, key: Option<&Path>) -> Result<Option<String>> {
    if t.read("yanks.toml").map_err(|e| anyhow!(e))?.is_some() {
        return Ok(None);
    }
    let keys = Reader::unsigned(t).map_err(|e| anyhow!(e))?.file.signing;
    let key = match key {
        _ if keys.is_empty() => None,
        Some(k) if keys.contains(&public_of(k)?) => Some(k),
        _ => {
            return Ok(Some(
                "this registry has no yank list (yanks.toml), and the page, gp game fetch and gp registry check refuse a registry without one: gp registry reindex TARGET --key SIGNING_KEY writes it".into(),
            ));
        }
    };
    let (now, expires) = (utc(0)?, utc(7)?);
    signing(key, |s| start_yanks(t, s, &now, &expires))?;
    Ok(Some(
        "wrote an empty yank list (yanks.toml): this registry had none".into(),
    ))
}

/// What the page would make of a content version once a reindex lists it,
/// for each game it is published for: the game's list as it is now with
/// this version on it, chosen by the page's own code
/// (`gp_web_registry::choose`). Unsigned reading: signatures don't change
/// the choice.
fn page_would(
    t: &dyn Target,
    name: &str,
    version: &str,
    title: &str,
    for_games: &[String],
    id: &str,
) -> Vec<String> {
    if for_games.is_empty() {
        return vec![format!(
            "the page won't offer {name} for any game: it is published with no --for GAME"
        )];
    }
    let Ok(now) = utc(0) else { return Vec::new() };
    let r = Reader::unsigned(t).ok();
    let yanks = r
        .as_ref()
        .and_then(|r| r.yanks(&now).ok())
        .map(|y| y.yank)
        .unwrap_or_default();
    let mut out = Vec::new();
    for game in for_games {
        let mut lines = r
            .as_ref()
            .and_then(|r| r.by_game(game, &now).ok())
            .map(|b| b.line)
            .unwrap_or_default();
        lines.retain(|l| l.package != name);
        lines.push(gp_package::registry::ByGameLine {
            package: name.to_owned(),
            version: version.to_owned(),
            id: id.to_owned(),
            title: title.to_owned(),
            recheck: None,
        });
        out.push(match gp_web_registry::choose(game, &lines, &yanks).verdict(name) {
            Some(Ok(_)) => format!(
                "the page will offer it for {game} in Settings, Theme as `{}` once a reindex lists it (gp registry reindex ... --keep {name}@{version}), its namespace granted; gp registry check ... --for {game} --theme {name} then asks",
                gp_web_registry::theme_id(name)
            ),
            Some(Err(l)) => format!(
                "the page would leave it out for {game}, because {} ({})",
                l.reason, l.detail
            ),
            None => format!("the page's list for {game} wouldn't name it"),
        });
    }
    out
}

/// Writes signed attestation files about package `id`, each a statement's
/// text, under `attest/<id>/`. Once written they can't be replaced.
pub(crate) fn attest_cmd(
    target: &str,
    id: &str,
    statements: &[String],
    key: Option<&Path>,
) -> Result<Outcome> {
    committing(
        target,
        &format!("Attest package {}", &id[..12.min(id.len())]),
        || {
            let t = open(target);
            let mut keys = Vec::new();
            for text in statements {
                keys.push(signing(key, |s| attest(&*t, id, text, s))?);
            }
            Ok(Outcome {
                text: format!("wrote {} attestations:\n{}", keys.len(), keys.join("\n")),
                json: json!({ "attestations": keys }),
                exit: 0,
            })
        },
    )
}

/// Rewrites the list files from `versions`, or with `keep` (or no
/// versions) from what they list now plus `versions`, refusing to drop a
/// listed version not named in `drops`.
pub fn reindex_cmd(
    target: &str,
    (versions, keep, drops): (&[String], bool, &[String]),
    key: Option<&Path>,
    how: Writing,
) -> Result<Outcome> {
    let message = if key.is_some() {
        "Re-sign the list files"
    } else {
        "Rewrite the list files"
    };
    writing(target, message, how, |t| {
        reindex_inner(t, (versions, keep || versions.is_empty(), drops), key)
    })
}

/// Two versions in order by their dotted numbers (1.0.10 after 1.0.9), as
/// the page orders them; a part that isn't a number compares as text.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<(u64, String)> {
        v.split('.')
            .map(|p| (p.parse().unwrap_or(u64::MAX), p.to_owned()))
            .collect()
    };
    parts(a).cmp(&parts(b))
}

fn pairs(list: &[String]) -> Result<Vec<(String, String)>> {
    list.iter()
        .map(|v| name_at(v).map(|(n, ver)| (n.to_owned(), ver.to_owned())))
        .collect()
}

fn at(l: &Listed) -> String {
    format!("{}@{}", l.name, l.version)
}

fn reindex_inner(
    t: &dyn Target,
    (versions, keep, drops): (&[String], bool, &[String]),
    key: Option<&Path>,
) -> Result<Outcome> {
    let given = pairs(versions)?;
    let drops = pairs(drops)?;
    let now_listed = listed(t).map_err(|e| anyhow!(e))?;
    let is_listed =
        |(n, v): &(String, String)| now_listed.iter().any(|l| &l.name == n && &l.version == v);
    if let Some((n, v)) = drops.iter().find(|d| !is_listed(d)) {
        anyhow::bail!("{n}@{v} isn't in the list files now, so --drop has nothing to drop");
    }
    if let Some((n, v)) = drops.iter().find(|d| given.contains(d)) {
        anyhow::bail!("{n}@{v} is both given and dropped");
    }
    let list = if keep {
        let mut list: Vec<(String, String)> = now_listed
            .iter()
            .map(|l| (l.name.clone(), l.version.clone()))
            .filter(|p| !drops.contains(p))
            .collect();
        for p in &given {
            if !list.contains(p) {
                list.push(p.clone());
            }
        }
        // The catalog takes the last version of each package: put each
        // package's versions in order, each in a place one of them held.
        let names: Vec<String> = list.iter().map(|(n, _)| n.clone()).collect();
        for name in &names {
            let at: Vec<usize> = (0..list.len()).filter(|&i| &list[i].0 == name).collect();
            let mut vs: Vec<String> = at.iter().map(|&i| list[i].1.clone()).collect();
            vs.sort_by(|a, b| version_cmp(a, b));
            for (i, v) in at.into_iter().zip(vs) {
                list[i].1 = v;
            }
        }
        list
    } else {
        given.clone()
    };
    let after = would_list(t, &list).map_err(|e| anyhow!(e))?;
    // A version only the catalog names, replaced there by a newer one of
    // its package, isn't dropped: the catalog holds each package's latest.
    let dropped: Vec<&Listed> = now_listed
        .iter()
        .filter(|l| {
            !after
                .iter()
                .any(|a| a.name == l.name && a.version == l.version)
        })
        .filter(|l| {
            l.by_game
                || !after
                    .iter()
                    .any(|a| a.name == l.name && version_cmp(&a.version, &l.version).is_gt())
        })
        .collect();
    let unasked: Vec<String> = dropped
        .iter()
        .filter(|l| !drops.contains(&(l.name.clone(), l.version.clone())))
        .map(|l| at(l))
        .collect();
    if !unasked.is_empty() {
        anyhow::bail!(
            "this reindex would drop versions the list files name now: {}. Name each one with --drop NAME@VERSION to drop it, or give --keep to keep what is listed and add the versions given",
            unasked.join(", ")
        );
    }
    let (now, expires) = (utc(0)?, utc(7)?);
    signing(key, |s| reindex(t, &list, s, &now, &expires))?;
    let mut text = vec![if key.is_some() {
        format!("re-signed the list files; they expire at {expires}")
    } else {
        "rewrote the list files, not signed".to_string()
    }];
    let added: Vec<String> = after
        .iter()
        .filter(|a| {
            !now_listed
                .iter()
                .any(|l| l.name == a.name && l.version == a.version)
        })
        .map(at)
        .collect();
    let dropped: Vec<String> = dropped.into_iter().map(at).collect();
    if keep || !added.is_empty() || !dropped.is_empty() {
        text.push(format!(
            "listed: {} versions; added {}; dropped {}",
            after.len(),
            if added.is_empty() {
                "none".into()
            } else {
                added.join(", ")
            },
            if dropped.is_empty() {
                "none".into()
            } else {
                dropped.join(", ")
            },
        ));
    }
    Ok(Outcome {
        json: json!({
            "versions": list.len(), "expires": expires,
            "listed": after.iter().map(at).collect::<Vec<_>>(),
            "added": added, "dropped": dropped,
        }),
        text: text.join("\n"),
        exit: 0,
    })
}

/// A public key file's one line, refusing a private key.
fn public_key_file(file: &Path) -> Result<String> {
    let text = read_root(file)?;
    if text.contains("PRIVATE KEY") {
        anyhow::bail!(
            "{} is a private key: give the public one (the .pub file)",
            file.display()
        );
    }
    Ok(text)
}

/// Grants a namespace: its file and registry.toml's line, one commit on a
/// git: target, with the message a grant has always had.
pub fn grant_cmd(
    target: &str,
    ns: &str,
    keys: &[PathBuf],
    root_key: Option<&Path>,
    how: Writing,
) -> Result<Outcome> {
    writing(target, &format!("Grant the {ns} namespace"), how, |t| {
        let keys = keys
            .iter()
            .map(|k| public_key_file(k))
            .collect::<Result<Vec<_>>>()?;
        let today = utc(0)?[..10].to_owned();
        signing(root_key, |r| grant(t, ns, &keys, r, &today))?;
        Ok(Outcome {
            json: json!({ "namespace": ns, "keys": keys.len() }),
            text: format!(
                "granted {ns}: wrote namespaces/{ns}.toml and added {ns} to registry.toml"
            ),
            exit: 0,
        })
    })
}

/// Each granted namespace, from registry.toml and the namespace files.
pub fn grants_cmd(target: &str) -> Result<Outcome> {
    let t = open(target);
    let list = grants(&*t).map_err(|e| anyhow!(e))?;
    let mut lines = Vec::new();
    let mut rows = Vec::new();
    let mut bad = false;
    for (ns, f) in &list {
        match f {
            Ok(f) => {
                lines.push(format!(
                    "{ns}  {}  {} key{}",
                    f.status,
                    f.keys.len(),
                    if f.keys.len() == 1 { "" } else { "s" }
                ));
                rows.push(json!({ "namespace": ns, "status": f.status, "keys": f.keys.len() }));
            }
            Err(e) => {
                bad = true;
                lines.push(format!("{ns}  listed in registry.toml, but {e}"));
                rows.push(json!({ "namespace": ns, "error": e }));
            }
        }
    }
    if lines.is_empty() {
        lines.push("no namespace is granted".into());
    }
    Ok(Outcome {
        json: json!({ "namespaces": rows }),
        text: lines.join("\n"),
        exit: u8::from(bad),
    })
}

/// `NAME@VERSION`'s two parts.
fn name_at(v: &str) -> Result<(&str, &str)> {
    v.rsplit_once('@')
        .with_context(|| format!("wanted name@version, not {v}"))
}

/// Adds a version to yanks.toml with (reason, severity), or with `undo`
/// takes it out.
pub fn yank_cmd(
    target: &str,
    version: &str,
    why: Option<(&str, &str)>,
    undo: bool,
    key: Option<&Path>,
    how: Writing,
) -> Result<Outcome> {
    let (name, ver) = name_at(version)?;
    let message = if undo {
        format!("Take back the yank of {name}@{ver}")
    } else {
        format!("Withdraw {name}@{ver}")
    };
    writing(target, &message, how, |t| {
        let (now, expires) = (utc(0)?, utc(7)?);
        let y = match (undo, why) {
            (true, _) => signing(key, |s| unyank(t, name, ver, s, &now, &expires))?,
            (false, Some((reason, severity))) => signing(key, |s| {
                yank(t, name, ver, reason, severity, s, &now, &expires)
            })?,
            (false, None) => anyhow::bail!("a yank needs --reason and --severity"),
        };
        let text = if undo {
            format!("took {name}@{ver} out of yanks.toml: the page offers it again")
        } else if y.severity == "security" {
            format!(
                "yanked {name}@{ver} ({}) for security: the page leaves it out and refuses to install it",
                &y.id[..12]
            )
        } else {
            format!(
                "yanked {name}@{ver} ({}) with a notice: the page still offers it and shows the reason",
                &y.id[..12]
            )
        };
        Ok(Outcome {
            json: json!({
                "name": name, "version": ver, "id": y.id, "undo": undo,
                "reason": y.reason, "severity": y.severity,
            }),
            text,
            exit: 0,
        })
    })
}

/// CORS as a browser's fetch from another origin needs it.
fn cors(url: &str) -> Result<bool> {
    let o = Command::new("curl")
        .args([
            "-sS",
            "-o",
            "/dev/null",
            "-D",
            "-",
            "--max-time",
            "60",
            "-H",
        ])
        .arg("Origin: https://gameprinter.invalid")
        .arg(format!("{}/registry.toml", url.trim_end_matches('/')))
        .output()
        .context("running curl")?;
    let head = String::from_utf8_lossy(&o.stdout).to_lowercase();
    Ok(head
        .lines()
        .any(|l| l.starts_with("access-control-allow-origin:")))
}

/// Tests a registry from outside: CORS, signatures, expiry and the catalog,
/// and that every package the catalog lists fetches by its exact id.
pub fn check_cmd(url: &str, root_file: Option<&Path>) -> Result<Outcome> {
    let root = root_file.map(read_root).transpose()?;
    let t = open(url);
    let now = utc(0)?;
    let mut lines = Vec::new();
    let mut failed = false;
    let mut step = |name: &str, r: Result<String, String>| match r {
        Ok(s) => lines.push(format!("ok      {name}: {s}")),
        Err(e) => {
            failed = true;
            lines.push(format!("FAILED  {name}: {e}"));
        }
    };
    if url.starts_with("http") {
        step(
            "cors",
            match cors(url) {
                Ok(true) => Ok("a browser may read it".into()),
                Ok(false) => Err("no Access-Control-Allow-Origin header".into()),
                Err(e) => Err(e.to_string()),
            },
        );
    }
    let opened = match &root {
        Some(root) => Reader::open(&*t, root),
        None => Reader::unsigned(&*t),
    };
    match opened {
        Err(e) => step("registry.toml", Err(e)),
        Ok(r) => {
            step(
                "registry.toml",
                Ok(format!(
                    "{}{}",
                    r.file.name,
                    if root.is_some() {
                        " under the pinned root"
                    } else {
                        ", signatures not looked at"
                    }
                )),
            );
            step(
                "yanks.toml",
                r.yanks(&now).map(|y| format!("{} yanks", y.yank.len())),
            );
            match r.catalog(&now) {
                Err(e) => step("catalog.toml", Err(e)),
                Ok(c) => {
                    step(
                        "catalog.toml",
                        Ok(format!("{} packages, until {}", c.package.len(), c.expires)),
                    );
                    for p in &c.package {
                        step(
                            &p.name,
                            r.fetch(&p.name, &p.latest, &p.id)
                                .map(|(_, f)| format!("{} files by id {}", f.len(), &p.id[..12])),
                        );
                    }
                }
            }
        }
    }
    let _ = &mut step;
    Ok(Outcome {
        json: json!({ "ok": !failed, "lines": lines }),
        text: lines.join("\n"),
        exit: u8::from(failed),
    })
}

/// `gp registry check --for GAME [--theme NAME]`: the game's theme list read
/// by the page's own code (`gp_web_registry::offer_on`), each theme offered
/// installed as the page installs it (`install_on`), and each left out with
/// why, added to `o`. With `theme`, its verdict; when it is offered and the
/// game's folder is here, its files read as the page's lift reads them
/// (`gp theme look check`'s "On the page" findings).
fn offered_cmd(
    o: &mut Outcome,
    url: &str,
    root_file: Option<&Path>,
    game: &str,
    theme: Option<&str>,
    rom: Option<&Path>,
) -> Result<()> {
    let root = root_file.map(read_root).transpose()?;
    let t = open(url);
    let now = utc(0)?;
    let mut lines: Vec<String> = Vec::new();
    let mut failed = false;
    let offer = match gp_web_registry::offer_on(&*t, root.as_deref(), game, &now) {
        Ok(x) => x,
        Err(e) => {
            lines.push(format!(
                "FAILED  themes for {game}: the page can't read the game's list: {e}"
            ));
            failed = true;
            gp_web_registry::Offer::default()
        }
    };
    let mut offered = Vec::new();
    for x in &offer.offered {
        let installed = gp_web_registry::install_on(
            &*t,
            root.as_deref(),
            (&x.name, Some(&x.version)),
            Some(game),
            &now,
        );
        match &installed {
            Ok(r) => lines.push(format!(
                "ok      theme {}@{}: offered for {game} as `{}` ({}), {} files{}{}",
                x.name,
                x.version,
                gp_web_registry::theme_id(&x.name),
                x.title,
                r.files.len(),
                r.yank
                    .as_ref()
                    .map(|y| format!("; withdrawn ({}): {}", y.severity, y.reason))
                    .unwrap_or_default(),
                crate::registry_recheck::said(x.recheck.as_ref())
            )),
            Err(e) => {
                failed = true;
                lines.push(format!(
                    "FAILED  theme {}@{}: listed for {game}, and the page's install fails, so it isn't offered: {e}",
                    x.name, x.version
                ));
            }
        }
        offered.push(json!({
            "name": x.name, "version": x.version, "id": gp_web_registry::theme_id(&x.name),
            "title": x.title, "installs": installed.is_ok(),
            "error": installed.as_ref().err(), "recheck": x.recheck,
        }));
    }
    for l in &offer.left_out {
        lines.push(format!(
            "left    theme {}@{}: left out for {game}, because {} ({})",
            l.name, l.version, l.reason, l.detail
        ));
    }
    if offer.offered.is_empty() && offer.left_out.is_empty() && !failed {
        lines.push(format!("note    themes for {game}: its list names none"));
    }
    let mut verdict = Value::Null;
    if let Some(name) = theme {
        let installs = offered
            .iter()
            .find(|v| v["name"] == name)
            .map(|v| v["installs"] == true);
        let (ok, says) = match (offer.verdict(name), installs) {
            (Some(Ok(x)), Some(true)) => (
                true,
                format!(
                    "{name}@{} is offered for {game} in Settings, Theme as `{}`",
                    x.version,
                    gp_web_registry::theme_id(name)
                ),
            ),
            (Some(Ok(x)), _) => (
                false,
                format!(
                    "{name}@{} is listed for {game} and isn't offered: the page's install of it fails (above)",
                    x.version
                ),
            ),
            (Some(Err(l)), _) => (
                false,
                format!(
                    "{name}@{} isn't offered for {game}, because {} ({})",
                    l.version, l.reason, l.detail
                ),
            ),
            (None, _) => (false, not_listed(&*t, root.as_deref(), name, game, &now)),
        };
        // Offered by the registry: then its files, read as the page's lift
        // reads them over the game's build, when the game is here.
        let mut look = Value::Null;
        let mut after = Vec::new();
        let (mut ok, mut says) = (ok, says);
        if ok {
            match look_check(&*t, root.as_deref(), name, game, rom) {
                Ok(v) => {
                    let found = v["findings"].as_array().cloned().unwrap_or_default();
                    let text = |f: &Value| f["message"].as_str().unwrap_or("").to_owned();
                    if let Some(p) = found.iter().find(|f| f["level"] == "problem") {
                        ok = false;
                        says = format!("{name} is listed and installs for {game}, and the page's lift leaves it out: {}", text(p));
                    } else {
                        for f in found.iter().filter(|f| f["level"] == "warning" || f["level"] == "note") {
                            after.push(format!("{}: {}", f["level"].as_str().unwrap_or(""), text(f)));
                        }
                        if found.iter().any(|f| f["level"] == "ok") {
                            after.push("the page's lift reads its files and draws it over your ROM's game".into());
                        }
                    }
                    look = v;
                }
                Err(e) => after.push(format!(
                    "its files weren't read as the page's lift reads them, so whether your ROM's game draws it isn't known: {e:#}"
                )),
            }
        }
        failed |= !ok;
        lines.push(format!(
            "{}  {says}",
            if ok { "OFFERED" } else { "NOT OFFERED" }
        ));
        for a in after {
            lines.push(format!("        {a}"));
        }
        verdict = json!({ "name": name, "offered": ok, "says": says, "look": look });
    }
    o.text.push('\n');
    o.text.push_str(&lines.join("\n"));
    o.json["themes"] = json!({
        "game": game,
        "offered": offered,
        "left_out": offer.left_out,
        "theme": verdict,
        "lines": lines,
    });
    if failed {
        o.json["ok"] = false.into();
        o.exit = 1;
    }
    Ok(())
}

/// Why a theme the game's list doesn't name isn't offered: not published
/// here, or published for other games, or listed in no list yet.
fn not_listed(t: &dyn Target, root: Option<&str>, name: &str, game: &str, now: &str) -> String {
    let r = match root {
        Some(root) => Reader::open(t, root),
        None => Reader::unsigned(t),
    };
    let entry = r
        .ok()
        .and_then(|r| r.catalog(now).ok())
        .and_then(|c| c.package.into_iter().find(|p| p.name == name));
    match entry {
        None => format!(
            "{name} isn't offered for {game}: the registry's catalog doesn't list it. Publish it (gp registry publish ... --for {game}) and list it (gp registry reindex ... --keep {name}@VERSION)"
        ),
        Some(p) if !p.for_games.iter().any(|g| g == game) => format!(
            "{name} isn't offered for {game}: its latest version, {}, is published for {}, not for {game}. Publish a version with --for {game} and reindex",
            p.latest,
            if p.for_games.is_empty() {
                "no game".to_owned()
            } else {
                p.for_games.join(", ")
            }
        ),
        Some(p) => format!(
            "{name} isn't offered for {game}: the catalog lists {}@{} for it, and the game's list (by-game/) doesn't name it yet. Run gp registry reindex ... --keep",
            p.name, p.latest
        ),
    }
}

/// The offered theme's files, fetched by id, read as the page's lift reads
/// them over the game's build: `gp theme look check`'s "On the page"
/// findings, when the game's folder (the name after the namespace, in the
/// games checkout) is here.
fn look_check(
    t: &dyn Target,
    root: Option<&str>,
    name: &str,
    game: &str,
    rom: Option<&Path>,
) -> Result<Value> {
    let r = match root {
        Some(root) => Reader::open(t, root),
        None => Reader::unsigned(t),
    }
    .map_err(|e| anyhow!(e))?;
    let now = utc(0)?;
    let c = r.catalog(&now).map_err(|e| anyhow!(e))?;
    let p = c
        .package
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| anyhow!("{name} isn't in the catalog"))?;
    look_check_at(&r, (name, &p.latest, &p.id), game, rom)
}

/// [`look_check`] of one exact version, fetched by its id through `r`.
pub(crate) fn look_check_at(
    r: &Reader,
    (name, version, id): (&str, &str, &str),
    game: &str,
    rom: Option<&Path>,
) -> Result<Value> {
    let (_, files) = r.fetch(name, version, id).map_err(|e| anyhow!(e))?;
    let dir = tempfile::tempdir()?;
    for (rel, b) in &files {
        let f = dir.path().join(rel);
        fs::create_dir_all(f.parent().unwrap_or(dir.path()))?;
        fs::write(&f, b)?;
    }
    // The page names the theme by its package name, whatever package.toml
    // says; one is written for a package published without it.
    if !files.contains_key("package.toml") {
        fs::write(
            dir.path().join("package.toml"),
            format!("[package]\nname = \"{name}\"\n"),
        )?;
    }
    let short = game.split_once('/').map_or(game, |(_, n)| n);
    let paths = gp_evidence::Paths::from_env()?;
    let o = crate::theme_look::check(dir.path(), Path::new(short), rom, &paths)?;
    let found: Vec<&Value> = o.json["findings"]
        .as_array()
        .map(|a| a.iter().filter(|f| f["group"] == "offered").collect())
        .unwrap_or_default();
    let left = found.iter().any(|f| f["level"] == "problem");
    Ok(json!({ "offered": !left, "findings": found }))
}

/// The headers file a built page is served under: its own `_headers`
/// (scripts/web/public/_headers, which build-page.sh copies in), else the
/// public page's as this gp was built with it, so no way of serving a page
/// on this machine goes without its policy (gp_mcp::page_headers).
pub(crate) fn page_headers(page: &Path) -> String {
    fs::read_to_string(page.join("_headers"))
        .unwrap_or_else(|_| gp_mcp::page_headers::PUBLIC.to_owned())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        Some("toml" | "txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// The file a request path names, in the first of `roots` that has it (the
/// page's files, then the registry's). None for anything else, and for any
/// path that climbs out.
fn local_file(roots: &[PathBuf], url: &str) -> Option<PathBuf> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let rel = match path {
        "/" | "/p" => "index.html",
        p => p.strip_prefix('/')?,
    };
    if rel
        .split('/')
        .any(|part| part.is_empty() || part == ".." || part == ".")
        || rel.contains('\\')
    {
        return None;
    }
    roots
        .iter()
        .map(|root| root.join(rel))
        .find(|p| p.is_file())
}

/// Answers one request from the files under `roots`, with the headers the
/// rules of `headers` (a page's _headers text) give its path on every
/// answer.
pub(crate) fn answer(mut stream: std::net::TcpStream, roots: &[PathBuf], headers: &str) {
    use std::io::{BufRead as _, BufReader, Write as _};
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line.trim() != "" {
        line.clear();
    }
    let mut parts = first.split_whitespace();
    let (method, url) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let file = matches!(method, "GET" | "HEAD")
        .then(|| local_file(roots, url))
        .flatten();
    let (status, kind, body) = match file.and_then(|p| Some((content_type(&p), fs::read(&p).ok()?)))
    {
        Some((kind, body)) => ("200 OK", kind, body),
        None => ("404 Not Found", "text/plain", Vec::new()),
    };
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    let path = url.split(['?', '#']).next().unwrap_or("");
    for h in gp_mcp::page_headers::lines(&gp_mcp::page_headers::for_path(headers, path)) {
        head.push_str(&h);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    if method == "GET" {
        let _ = stream.write_all(&body);
    }
}

/// The first game the registry's catalog.toml lists, for the address
/// `serve` prints; none when the catalog lists no game.
fn first_game(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("catalog.toml")).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    v.get("package")?
        .as_array()?
        .iter()
        .find(|p| p.get("kind").and_then(|k| k.as_str()) == Some("game"))
        .and_then(|p| p.get("name")?.as_str().map(str::to_owned))
}

/// Serves a folder registry and a built page from one address, on this
/// machine only, until stopped.
pub fn serve(dir: &Path, page: &Path, port: u16) -> Result<Outcome> {
    if !dir.join("registry.toml").is_file() {
        return Err(anyhow!(
            "{} isn't a registry (no registry.toml)",
            dir.display()
        ));
    }
    if !page.join("index.html").is_file() {
        return Err(anyhow!(
            "{} isn't a built page (no index.html)",
            page.display()
        ));
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("listening on 127.0.0.1:{port}"))?;
    let headers = std::sync::Arc::new(page_headers(page));
    let roots = std::sync::Arc::new(vec![page.to_owned(), dir.to_owned()]);
    let at = match first_game(dir) {
        Some(g) => format!("http://127.0.0.1:{port}/p?g={g}"),
        None => format!("http://127.0.0.1:{port}/"),
    };
    println!("playing at {at} (ctrl-C stops it)");
    for stream in listener.incoming().flatten() {
        let (roots, headers) = (roots.clone(), headers.clone());
        std::thread::spawn(move || answer(stream, &roots, &headers));
    }
    Ok(Outcome {
        json: json!({}),
        text: String::new(),
        exit: 0,
    })
}

#[cfg(test)]
mod repo_tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{args:?}: {o:?}");
        String::from_utf8_lossy(&o.stdout).trim().to_owned()
    }

    /// A checkout of a repo with an `origin`, and the origin.
    fn pair() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (origin, work) = (d.path().join("origin.git"), d.path().join("work"));
        git(d.path(), &["init", "-q", "--bare", "origin.git"]);
        git(d.path(), &["clone", "-q", origin.to_str().unwrap(), "work"]);
        git(&work, &["config", "user.email", "t@t"]);
        git(&work, &["config", "user.name", "t"]);
        (d, origin, work)
    }

    fn ok() -> Result<Outcome> {
        Ok(Outcome {
            json: json!({}),
            text: String::new(),
            exit: 0,
        })
    }

    #[test]
    fn a_command_commits_what_it_wrote_and_pushes() {
        let (_d, origin, work) = pair();
        let spec = format!("git:{}", work.display());
        committing(&spec, "first", || {
            open(&spec).write_once("a/b", b"x").unwrap();
            ok()
        })
        .unwrap();
        assert_eq!(git(&origin, &["log", "--format=%s", "HEAD"]), "first");
        assert_eq!(git(&origin, &["show", "HEAD:a/b"]), "x");
    }

    #[test]
    fn no_push_commits_and_leaves_the_push_to_the_person() {
        let (_d, origin, work) = pair();
        git(&work, &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(&work, &["push", "-q", "origin", "HEAD"]);
        let spec = format!("git:{}", work.display());
        let o = committing_as(&spec, "Grant the friend namespace", false, || {
            open(&spec).write_once("a/b", b"x").unwrap();
            ok()
        })
        .unwrap();
        assert!(o.text.contains("did not push it"), "{}", o.text);
        assert!(o.text.contains("push origin HEAD"), "{}", o.text);
        assert_eq!(git(&origin, &["log", "--format=%s", "HEAD"]), "base");
        assert_eq!(
            git(&work, &["log", "-1", "--format=%s"]),
            "Grant the friend namespace"
        );
        // The next command pushes it with its own.
        let o = committing(&spec, "next", || {
            open(&spec).write_once("c", b"y").unwrap();
            ok()
        })
        .unwrap();
        assert!(o.text.contains("pushed it to origin ("), "{}", o.text);
        assert_eq!(
            git(&origin, &["log", "--format=%s", "HEAD"]),
            "next\nGrant the friend namespace\nbase"
        );
        // A folder is written at once, so it has no --no-push.
        let e = committing_as(&work.display().to_string(), "m", false, ok)
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("git:PATH"), "{e}");
    }

    #[test]
    fn a_tilde_is_the_home_folder() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(home_path("~/r"), Path::new(&home).join("r"));
        assert_eq!(home_path("/x/~/r"), Path::new("/x/~/r"));
    }

    #[test]
    fn a_published_key_is_not_written_again() {
        let (_d, _origin, work) = pair();
        let spec = format!("git:{}", work.display());
        assert!(open(&spec).write_once("k", b"one").unwrap());
        assert!(!open(&spec).write_once("k", b"two").unwrap());
        assert_eq!(fs::read(work.join("k")).unwrap(), b"one");
    }

    #[test]
    fn a_dirty_checkout_is_refused_and_a_failed_command_commits_nothing() {
        let (_d, _origin, work) = pair();
        let spec = format!("git:{}", work.display());
        fs::write(work.join("stray"), "x").unwrap();
        let e = committing(&spec, "m", ok).err().unwrap().to_string();
        assert!(e.contains("not committed"), "{e}");
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "base"]);
        let r = committing(&spec, "bad", || {
            open(&spec).write_once("half", b"x").unwrap();
            anyhow::bail!("stopped")
        });
        assert!(r.is_err());
        assert_eq!(git(&work, &["log", "--format=%s"]), "base");
    }

    #[test]
    fn a_key_another_publisher_took_counts_as_taken() {
        let (d, origin, work) = pair();
        git(
            d.path(),
            &["clone", "-q", origin.to_str().unwrap(), "other"],
        );
        let other = d.path().join("other");
        git(&other, &["config", "user.email", "o@o"]);
        git(&other, &["config", "user.name", "o"]);
        let (a, b) = (
            format!("git:{}", work.display()),
            format!("git:{}", other.display()),
        );
        committing(&a, "theirs", || {
            open(&a).write_once("k", b"one").unwrap();
            ok()
        })
        .unwrap();
        committing(&b, "mine", || {
            assert!(!open(&b).write_once("k", b"two").unwrap());
            ok()
        })
        .unwrap();
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    /// A made-up unsigned registry with one namespace, `own`.
    fn scratch() -> (tempfile::TempDir, String) {
        let d = tempfile::tempdir().unwrap();
        let spec = d.path().join("reg").display().to_string();
        init(
            &*open(&spec),
            "scratch",
            &Signer::none(),
            &[],
            &[("own".to_owned(), Vec::new())],
            "2026-01-01",
        )
        .unwrap();
        (d, spec)
    }

    /// An ssh key pair made for the test, its private key's path.
    fn keypair(dir: &Path, name: &str) -> PathBuf {
        let k = dir.join(name);
        let o = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
            .arg(&k)
            .output()
            .unwrap();
        assert!(o.status.success(), "{o:?}");
        k
    }

    /// Made-up "ROM" bytes: an iNES header and 24 KB that are no game's.
    fn made_up_rom(fill: u32) -> Vec<u8> {
        let mut rom = b"NES\x1a\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        rom.extend(
            (0..16384u32 + 8192)
                .map(|i| (i.wrapping_mul(2_654_435_761).wrapping_add(fill) >> 13) as u8),
        );
        rom
    }

    /// A made-up game package that says it plays from its own pictures
    /// (play/page.json's rom.kind none), a ROM folder holding one made-up
    /// ROM and a home with nothing lifted.
    fn own_game() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, Vec<u8>) {
        let d = tempfile::tempdir().unwrap();
        let pkg = d.path().join("own-hop");
        fs::create_dir_all(pkg.join("play/assets")).unwrap();
        fs::write(pkg.join("package.toml"), "[package]\nname = \"own/own-hop\"\nversion = \"0.1.0\"\nkind = \"game\"\ntitle = \"Own Hop\"\n").unwrap();
        fs::write(pkg.join("play/page.json"), r#"{"format":1,"title":"Own Hop","player":"own","rom":{"kind":"none","game":"own-hop"}}"#).unwrap();
        fs::write(pkg.join("play/bundle.json"), "{\"made_up\": [1, 2, 3]}\n").unwrap();
        fs::write(
            pkg.join("play/assets/hero.png"),
            b"\x89PNG made-up picture bytes",
        )
        .unwrap();
        let roms = d.path().join("roms");
        fs::create_dir_all(&roms).unwrap();
        let rom = made_up_rom(7);
        fs::write(roms.join("made-up.nes"), &rom).unwrap();
        let home = d.path().join("home");
        fs::create_dir_all(&home).unwrap();
        (d, pkg, roms, home, rom)
    }

    /// gp template served on such a package, its built play/ given as a
    /// folder (as build-page.sh gives it), reads a person's theme in
    /// play/themes/ too: a picture of it holding a ROM's bytes is found.
    #[test]
    fn template_served_reads_the_theme_in_an_own_games_play_folder() {
        let (_d, pkg, roms, home, rom) = own_game();
        let paths = gp_evidence::Paths {
            home: home.clone(),
            store: home.join("store"),
            roms,
        };
        let play = pkg.join("play");
        let o = crate::served::served(&pkg, std::slice::from_ref(&play), &[], &paths).unwrap();
        assert_eq!(o.exit, 0, "{}", o.text);
        let pic = play.join("themes/night/art/hero.png");
        fs::create_dir_all(pic.parent().unwrap()).unwrap();
        fs::write(&pic, &rom[16 + 0x300..16 + 0x340]).unwrap();
        let o = crate::served::served(&pkg, std::slice::from_ref(&play), &[], &paths).unwrap();
        assert_eq!(o.exit, 1, "{}", o.text);
        assert!(
            o.text
                .contains("play/themes/night/art/hero.png: 64 bytes of made-up.nes"),
            "{}",
            o.text
        );
    }

    /// The registry's publish step refuses a package that claims its own
    /// pictures but holds a run of a ROM's bytes (raw, and as numbers in a
    /// text file), a whole game image, a file lifted from a ROM on this
    /// machine, a lift or a ROM entry; and takes it once it holds none.
    #[test]
    fn a_package_with_its_own_pictures_holding_a_roms_bytes_is_refused() {
        let (_d, pkg, roms, home, rom) = own_game();
        let ok = own_pictures_guard(&pkg, &roms, &home).unwrap().unwrap();
        assert!(
            ok.contains("Own Hop plays from its own pictures") && ok.contains("made-up.nes"),
            "{ok}"
        );
        // 64 bytes of the ROM's program in a picture.
        fs::write(
            pkg.join("play/assets/hero.png"),
            &rom[16 + 0x200..16 + 0x240],
        )
        .unwrap();
        let e = own_pictures_guard(&pkg, &roms, &home)
            .unwrap_err()
            .to_string();
        assert!(
            e.starts_with("Own Hop says it plays from its own pictures")
                && e.contains("play/assets/hero.png: 64 bytes of made-up.nes")
                && e.ends_with("nothing was published"),
            "{e}"
        );
        fs::write(pkg.join("play/assets/hero.png"), b"\x89PNG made-up").unwrap();
        // The same bytes written as numbers in the bundle.
        let nums: Vec<String> = rom[16 + 0x1000..16 + 0x1040]
            .iter()
            .map(u8::to_string)
            .collect();
        fs::write(
            pkg.join("play/bundle.json"),
            format!("{{\"tiles\": [{}]}}\n", nums.join(", ")),
        )
        .unwrap();
        let e = own_pictures_guard(&pkg, &roms, &home)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("play/bundle.json") && e.contains("(numbers)"),
            "{e}"
        );
        fs::write(pkg.join("play/bundle.json"), "{}\n").unwrap();
        // A whole game image.
        fs::write(pkg.join("play/game.bin"), made_up_rom(99)).unwrap();
        let e = own_pictures_guard(&pkg, &roms, &home)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("play/game.bin: starts with an iNES header"),
            "{e}"
        );
        fs::remove_file(pkg.join("play/game.bin")).unwrap();
        // A picture lifted from another game on this machine.
        let lifted = home.join("work/assets/some-game");
        fs::create_dir_all(&lifted).unwrap();
        fs::write(
            lifted.join("tiles.png"),
            b"\x89PNG lifted from a made-up ROM",
        )
        .unwrap();
        fs::write(
            pkg.join("play/assets/tiles.png"),
            b"\x89PNG lifted from a made-up ROM",
        )
        .unwrap();
        let e = own_pictures_guard(&pkg, &roms, &home)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains(
                "play/assets/tiles.png: identical to the lifted file tiles.png of some-game"
            ),
            "{e}"
        );
        fs::remove_file(pkg.join("play/assets/tiles.png")).unwrap();
        // A person's own theme of it (gp game share puts it in themes/ID/
        // and the build in play/themes/ID/): a picture of it holding a
        // ROM's bytes, or lifted from another game, in either folder.
        for at in ["play/themes/night/art", "themes/night/art"] {
            fs::create_dir_all(pkg.join(at)).unwrap();
            let pic = pkg.join(at).join("hero.png");
            fs::write(&pic, &rom[16 + 0x300..16 + 0x340]).unwrap();
            let e = own_pictures_guard(&pkg, &roms, &home)
                .unwrap_err()
                .to_string();
            assert!(
                e.contains(&format!("{at}/hero.png: 64 bytes of made-up.nes")),
                "{e}"
            );
            fs::write(&pic, b"\x89PNG lifted from a made-up ROM").unwrap();
            let e = own_pictures_guard(&pkg, &roms, &home)
                .unwrap_err()
                .to_string();
            assert!(
                e.contains(&format!("{at}/hero.png: identical to the lifted file")),
                "{e}"
            );
            fs::write(&pic, b"\x89PNG the person's own picture").unwrap();
        }
        assert!(own_pictures_guard(&pkg, &roms, &home).unwrap().is_some());
        // A lift, or a ROM named.
        fs::create_dir_all(pkg.join("play/lift")).unwrap();
        let e = own_pictures_guard(&pkg, &roms, &home)
            .unwrap_err()
            .to_string();
        assert!(e.contains("holds a lift"), "{e}");
        fs::remove_dir(pkg.join("play/lift")).unwrap();
        fs::write(pkg.join("roms.toml"), "[[source]]\nid = \"x\"\n").unwrap();
        assert!(
            own_pictures_guard(&pkg, &roms, &home)
                .unwrap_err()
                .to_string()
                .contains("roms.toml")
        );
        fs::remove_file(pkg.join("roms.toml")).unwrap();
        // Clean again: taken, and published.
        assert!(own_pictures_guard(&pkg, &roms, &home).unwrap().is_some());
        let (_r, spec) = scratch();
        let o = publish_inner(
            &*open(&spec),
            &pkg,
            ("own/own-hop", "0.1.0"),
            ("game", "Own Hop"),
            &[],
            None,
        )
        .unwrap();
        assert!(
            o.text.starts_with("published own/own-hop@0.1.0"),
            "{}",
            o.text
        );
    }

    /// The game's own build is its own: a template that builds its ROM from
    /// source (`how = "build"`, its hash in `accepts`) isn't refused for
    /// holding that ROM's bytes, nor for its own lifted folder; a package
    /// that plays from a ROM makes no claim and isn't asked.
    #[test]
    fn a_games_own_build_is_not_someone_elses_rom() {
        let (_d, pkg, roms, home, rom) = own_game();
        let sha = crate::template::sha256_hex(&rom[16..]);
        fs::write(pkg.join("template.toml"), format!("[template]\nid = \"own-hop\"\n\n[source]\nid = \"own-hop\"\nhow = \"build\"\naccepts = [\"{sha}\"]\n")).unwrap();
        fs::write(
            pkg.join("model.toml"),
            "[model]\nname = \"own-hop\"\nsource = \"own-hop\"\n",
        )
        .unwrap();
        fs::write(
            pkg.join("play/assets/hero.png"),
            &rom[16 + 0x200..16 + 0x240],
        )
        .unwrap();
        let own = home.join("work/assets/own-hop");
        fs::create_dir_all(&own).unwrap();
        fs::write(own.join("tiles.png"), b"\x89PNG its own").unwrap();
        fs::write(pkg.join("play/assets/tiles.png"), b"\x89PNG its own").unwrap();
        let ok = own_pictures_guard(&pkg, &roms, &home).unwrap().unwrap();
        assert!(
            !ok.contains("made-up.nes"),
            "its own build isn't compared: {ok}"
        );
        // Another ROM's bytes are still found.
        let other = made_up_rom(1234);
        fs::write(roms.join("other.nes"), &other).unwrap();
        fs::write(
            pkg.join("play/assets/foe.png"),
            &other[16 + 0x80..16 + 0xc0],
        )
        .unwrap();
        assert!(
            own_pictures_guard(&pkg, &roms, &home)
                .unwrap_err()
                .to_string()
                .contains("other.nes")
        );
        fs::write(pkg.join("play/page.json"), r#"{"format":1,"title":"Own Hop","player":"nes","rom":{"kind":"nes","game":"own-hop"}}"#).unwrap();
        assert!(own_pictures_guard(&pkg, &roms, &home).unwrap().is_none());
    }

    /// `gp registry serve` sends the public page's headers, its policy
    /// among them, for a page that has no `_headers` of its own, and the
    /// page's own rules, by path, for one that has.
    #[test]
    fn a_served_page_gets_the_public_pages_headers() {
        use std::io::{Read as _, Write as _};
        let d = tempfile::tempdir().unwrap();
        let page = d.path().join("page");
        fs::create_dir_all(page.join("game")).unwrap();
        fs::write(page.join("index.html"), "<p>made up</p>").unwrap();
        fs::write(page.join("game/x.js"), "1").unwrap();
        let get = |headers: String, path: &str| {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = l.local_addr().unwrap().port();
            let roots = vec![page.clone()];
            let t = std::thread::spawn(move || answer(l.accept().unwrap().0, &roots, &headers));
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            t.join().unwrap();
            out
        };
        let a = get(page_headers(&page), "/p?g=own/x");
        assert!(
            a.starts_with("HTTP/1.1 200")
                && a.contains("Content-Security-Policy: default-src 'none';")
                && a.contains("Cross-Origin-Embedder-Policy: require-corp"),
            "{a}"
        );
        fs::write(page.join("_headers"), "/*\n  A: 1\n/game/*\n  B: 2\n").unwrap();
        let b = get(page_headers(&page), "/game/x.js");
        assert!(
            b.contains("A: 1\r\n")
                && b.contains("B: 2\r\n")
                && !b.contains("Content-Security-Policy"),
            "{b}"
        );
        assert!(!get(page_headers(&page), "/index.html").contains("B: 2"));
    }

    /// A new registry has an empty yank list, so a reader doesn't refuse
    /// it: unsigned, and signed by the signing key with an expiry, as a
    /// reindex writes it.
    #[test]
    fn init_writes_an_empty_yank_list() {
        let d = tempfile::tempdir().unwrap();
        let spec = d.path().join("plain").display().to_string();
        init_cmd(&spec, "plain", None, None, &["own".to_owned()], true).unwrap();
        let y = fs::read_to_string(Path::new(&spec).join("yanks.toml")).unwrap();
        assert!(
            y.ends_with("yank = []\n") && y.contains("expires = "),
            "{y}"
        );
        assert!(!Path::new(&spec).join("yanks.toml.sig").exists());

        let (root, index) = (keypair(d.path(), "root"), keypair(d.path(), "index"));
        let ns = keypair(d.path(), "ns");
        let spec = d.path().join("signed").display().to_string();
        let own = format!("own={}", ns.display());
        init_cmd(&spec, "signed", Some(&root), Some(&index), &[own], true).unwrap();
        let t = open(&spec);
        let r = Reader::open(&*t, &public_of(&root).unwrap()).unwrap();
        let y = r.yanks(&utc(0).unwrap()).unwrap();
        assert!(y.yank.is_empty() && y.registry == "signed", "{y:?}");
    }

    /// A publish onto a registry with no yank list (one made before init
    /// wrote one) writes an empty one when it may sign it, and otherwise
    /// says what writes it.
    #[test]
    fn publish_leaves_no_registry_without_a_yank_list() {
        let (_d, spec) = scratch();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "made up\n").unwrap();
        assert!(!Path::new(&spec).join("yanks.toml").exists());
        let o = publish_cmd(
            &spec,
            dir.path(),
            ("own/night", "0.1.0"),
            ("content", "Night"),
            &["own/game".to_owned()],
            None,
            Writing::default(),
        )
        .unwrap();
        assert!(o.text.contains("wrote an empty yank list"), "{}", o.text);
        let y = fs::read_to_string(Path::new(&spec).join("yanks.toml")).unwrap();
        assert!(y.ends_with("yank = []\n"), "{y}");
        // The next publish keeps it and says nothing of it.
        let o = publish_cmd(
            &spec,
            dir.path(),
            ("own/night", "0.2.0"),
            ("content", "Night"),
            &["own/game".to_owned()],
            None,
            Writing::default(),
        )
        .unwrap();
        assert!(!o.text.contains("yank list"), "{}", o.text);

        // Signed: the namespace's key can't sign a list file.
        let d = tempfile::tempdir().unwrap();
        let (root, index) = (keypair(d.path(), "root"), keypair(d.path(), "index"));
        let ns = keypair(d.path(), "ns");
        let spec = d.path().join("signed").display().to_string();
        let own = format!("own={}", ns.display());
        init_cmd(&spec, "signed", Some(&root), Some(&index), &[own], true).unwrap();
        fs::rename(
            Path::new(&spec).join("yanks.toml"),
            d.path().join("yanks.toml.moved"),
        )
        .unwrap();
        let o = publish_cmd(
            &spec,
            dir.path(),
            ("own/night", "0.1.0"),
            ("content", "Night"),
            &["own/game".to_owned()],
            Some(&ns),
            Writing::default(),
        )
        .unwrap();
        assert!(
            o.text.contains("has no yank list") && o.text.contains("gp registry reindex"),
            "{}",
            o.text
        );
        assert!(!Path::new(&spec).join("yanks.toml").exists());
    }

    #[test]
    fn grant_writes_the_namespace_and_grants_lists_it() {
        let (_d, spec) = scratch();
        let o = grant_cmd(&spec, "friend", &[], None, Writing::default()).unwrap();
        assert!(o.text.contains("granted friend"), "{}", o.text);
        let reg = fs::read_to_string(Path::new(&spec).join("registry.toml")).unwrap();
        assert!(reg.contains("namespaces = [\"own\", \"friend\"]"), "{reg}");
        let l = grants_cmd(&spec).unwrap();
        assert_eq!(l.text, "own  granted  0 keys\nfriend  granted  0 keys");
        let e = grant_cmd(&spec, "friend", &[], None, Writing::default())
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("already granted"), "{e}");
    }

    #[test]
    fn yank_withdraws_a_version_and_undo_takes_it_back() {
        let (_d, spec) = scratch();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "made up\n").unwrap();
        publish_cmd(
            &spec,
            dir.path(),
            ("own/night", "0.1.0"),
            ("content", "Night"),
            &["own/game".to_owned()],
            None,
            Writing::default(),
        )
        .unwrap();
        let o = yank_cmd(
            &spec,
            "own/night@0.1.0",
            Some(("made up", "security")),
            false,
            None,
            Writing::default(),
        )
        .unwrap();
        assert!(o.text.contains("for security"), "{}", o.text);
        let y = fs::read_to_string(Path::new(&spec).join("yanks.toml")).unwrap();
        assert!(
            y.contains("[[yank]]") && y.contains("severity = \"security\""),
            "{y}"
        );
        let o = yank_cmd(
            &spec,
            "own/night@0.1.0",
            None,
            true,
            None,
            Writing::default(),
        )
        .unwrap();
        assert!(o.text.contains("offers it again"), "{}", o.text);
        let y = fs::read_to_string(Path::new(&spec).join("yanks.toml")).unwrap();
        assert!(y.ends_with("yank = []\n"), "{y}");
    }

    /// `check --for` reads the game's theme list as the page does, and
    /// `--theme` says of one theme whether it is offered and why not: a
    /// name the page can't list, one withdrawn for safety, one whose
    /// namespace isn't granted (listed, and the install fails), one
    /// published for another game, and one never published. Made-up
    /// themes, a made-up game.
    #[test]
    fn check_for_a_game_says_what_is_offered_and_why_not() {
        let (_d, spec) = scratch();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "made up\n").unwrap();
        let game = ["own/game".to_owned()];
        for (name, title, games) in [
            ("own/night", "Night", &game[..]),
            ("own/8bit", "Eight", &game[..]),
            ("own/unsafe", "Unsafe", &game[..]),
            ("own/away", "Away", &["own/other".to_owned()][..]),
        ] {
            publish_cmd(
                &spec,
                dir.path(),
                (name, "0.1.0"),
                ("content", title),
                games,
                None,
                Writing::default(),
            )
            .unwrap();
        }
        // A namespace granted for the publish and then taken out of
        // registry.toml's list: the page lists the theme and its install
        // fails, as for one never granted.
        grant_cmd(&spec, "friend", &[], None, Writing::default()).unwrap();
        publish_cmd(
            &spec,
            dir.path(),
            ("friend/clay", "0.1.0"),
            ("content", "Clay"),
            &game,
            None,
            Writing::default(),
        )
        .unwrap();
        let reg = Path::new(&spec).join("namespaces/friend.toml");
        fs::rename(&reg, Path::new(&spec).join("friend.toml.kept")).unwrap();
        let all: Vec<String> = [
            "own/night",
            "own/8bit",
            "own/unsafe",
            "own/away",
            "friend/clay",
        ]
        .iter()
        .map(|n| format!("{n}@0.1.0"))
        .collect();
        reindex_cmd(&spec, (&all, false, &[]), None, Writing::default()).unwrap();
        yank_cmd(
            &spec,
            "own/unsafe@0.1.0",
            Some(("made up", "security")),
            false,
            None,
            Writing::default(),
        )
        .unwrap();
        let ask = |theme: Option<&str>| {
            let mut o = check_cmd(&spec, None).unwrap();
            offered_cmd(&mut o, &spec, None, "own/game", theme, None).unwrap();
            o
        };
        let o = ask(None);
        assert_eq!(
            o.exit, 1,
            "the install that fails fails the check: {}",
            o.text
        );
        assert!(
            o.text.contains(
                "ok      theme own/night@0.1.0: offered for own/game as `night` (Night), 1 files"
            ),
            "{}",
            o.text
        );
        assert!(
            o.text.contains("left    theme own/8bit@0.1.0: left out for own/game, because its name isn't one this page can list"),
            "{}",
            o.text
        );
        assert!(
            o.text.contains("left    theme own/unsafe@0.1.0: left out for own/game, because it was withdrawn for safety: made up"),
            "{}",
            o.text
        );
        assert!(
            o.text.contains(
                "FAILED  theme friend/clay@0.1.0: listed for own/game, and the page's install fails"
            ),
            "{}",
            o.text
        );
        let verdict = |theme: &str| {
            let o = ask(Some(theme));
            (
                o.json["themes"]["theme"]["offered"] == true,
                o.json["themes"]["theme"]["says"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        };
        let (ok, says) = verdict("own/night");
        assert!(ok, "{says}");
        assert_eq!(
            says,
            "own/night@0.1.0 is offered for own/game in Settings, Theme as `night`"
        );
        let (ok, says) = verdict("own/8bit");
        assert!(!ok && says.starts_with("own/8bit@0.1.0 isn't offered for own/game, because its name isn't one this page can list"), "{says}");
        let (ok, says) = verdict("friend/clay");
        assert!(
            !ok && says.contains("the page's install of it fails"),
            "{says}"
        );
        let (ok, says) = verdict("own/away");
        assert!(
            !ok && says.contains("is published for own/other, not for own/game"),
            "{says}"
        );
        let (ok, says) = verdict("own/never");
        assert!(
            !ok && says.contains("the registry's catalog doesn't list it"),
            "{says}"
        );
    }

    /// Every file under `dir` with its bytes.
    fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(p.clone(), fs::read(&p).unwrap());
                }
            }
        }
        out
    }

    /// Before a git: publish the package is compared with each ROM it
    /// names, and the line says which; a file holding a run of a ROM's
    /// bytes stops it.
    #[test]
    fn the_rom_comparison_names_the_roms_it_compared_against() {
        let d = tempfile::tempdir().unwrap();
        let roms = d.path().join("roms");
        fs::create_dir_all(&roms).unwrap();
        let (a, b) = (made_up_rom(3), made_up_rom(11));
        fs::write(roms.join("a.nes"), &a).unwrap();
        fs::write(roms.join("b.nes"), &b).unwrap();
        let pkg = d.path().join("pkg");
        fs::create_dir_all(pkg.join("assets")).unwrap();
        fs::write(
            pkg.join("package.toml"),
            format!(
                "[package]\nname = \"own/two\"\nversion = \"0.1.0\"\nkind = \"game\"\n\n[[rom.entry]]\nid = \"two-a\"\nsha256 = \"{}\"\n\n[[rom.entry]]\nid = \"two-b\"\nsha256 = \"{}\"\n",
                crate::roms::sha256_of(&a).unwrap(),
                crate::roms::sha256_of(&b).unwrap()
            ),
        )
        .unwrap();
        fs::write(pkg.join("assets/map.toml"), "made = \"up\"\n").unwrap();
        let said = no_rom_bytes(&pkg, &roms).unwrap();
        assert_eq!(
            said,
            "ROM comparison: done against two-a, two-b: none of the 2 files holds a run of their bytes"
        );
        fs::write(pkg.join("assets/lifted.bin"), &b[4096..8192]).unwrap();
        let e = no_rom_bytes(&pkg, &roms).unwrap_err().to_string();
        assert!(
            e.contains("lifted.bin holds") && e.contains("nothing was published"),
            "{e}"
        );
        let bare = tempfile::tempdir().unwrap();
        assert!(
            no_rom_bytes(bare.path(), &roms)
                .unwrap()
                .contains("no package.toml")
        );
    }

    #[test]
    fn a_dry_run_writes_nothing_and_says_what_it_would_write() {
        let (_d, spec) = scratch();
        let dry = Writing {
            dry_run: true,
            no_push: false,
        };
        let stage = tempfile::tempdir().unwrap();
        fs::write(stage.path().join("a.txt"), "made up\n").unwrap();
        fs::write(stage.path().join("b.txt"), "made up too\n").unwrap();
        let before = tree(Path::new(&spec));
        let args = (("own/night", "0.1.0"), ("content", "Night"));
        let games = ["own/game".to_owned()];
        let o = publish_cmd(&spec, stage.path(), args.0, args.1, &games, None, dry).unwrap();
        assert_eq!(
            tree(Path::new(&spec)),
            before,
            "a dry publish writes nothing"
        );
        let id = o.json["id"].as_str().unwrap().to_owned();
        assert!(
            o.text.starts_with("dry run: nothing was written."),
            "{}",
            o.text
        );
        assert!(o.text.contains(&id), "{}", o.text);
        assert!(o.text.contains("ROM comparison: not run;"), "{}", o.text);
        assert!(
            o.text.contains("add     2 blobs (0 already there)"),
            "{}",
            o.text
        );
        assert!(
            o.text.contains(&format!("add     packages/{id}")),
            "{}",
            o.text
        );
        assert!(
            o.text.contains("add     index/own/night/0.1.0.toml"),
            "{}",
            o.text
        );
        // The same id as the real publish.
        let real = publish_cmd(
            &spec,
            stage.path(),
            args.0,
            args.1,
            &games,
            None,
            Writing::default(),
        )
        .unwrap();
        assert_eq!(real.json["id"], json!(id));
        let before = tree(Path::new(&spec));
        let o = reindex_cmd(
            &spec,
            (&["own/night@0.1.0".to_owned()], false, &[]),
            None,
            dry,
        )
        .unwrap();
        assert_eq!(
            tree(Path::new(&spec)),
            before,
            "a dry reindex writes nothing"
        );
        assert!(o.text.contains("add     catalog.toml"), "{}", o.text);
        assert!(
            o.text.contains("add     by-game/own/game.toml"),
            "{}",
            o.text
        );
        assert!(o.text.contains("+ package = \"own/night\""), "{}", o.text);
        reindex_cmd(
            &spec,
            (&["own/night@0.1.0".to_owned()], false, &[]),
            None,
            Writing::default(),
        )
        .unwrap();
        let before = tree(Path::new(&spec));
        let o = grant_cmd(&spec, "friend", &[], None, dry).unwrap();
        assert!(
            o.text.contains("add     namespaces/friend.toml"),
            "{}",
            o.text
        );
        assert!(
            o.text.contains("+ namespaces = [\"own\", \"friend\"]"),
            "{}",
            o.text
        );
        let why = Some(("made up", "notice"));
        let o = yank_cmd(&spec, "own/night@0.1.0", why, false, None, dry).unwrap();
        assert!(o.text.contains("change  yanks.toml"), "{}", o.text);
        assert!(o.text.contains("+ severity = \"notice\""), "{}", o.text);
        assert_eq!(
            tree(Path::new(&spec)),
            before,
            "a dry grant and yank write nothing"
        );
    }

    /// Publishes made-up `NAME@VERSION`s into `spec`, each for own/game
    /// unless it is own/game itself.
    fn put(spec: &str, versions: &[&str]) {
        let stage = tempfile::tempdir().unwrap();
        for v in versions {
            let (n, ver) = name_at(v).unwrap();
            fs::write(stage.path().join("a.txt"), format!("made up {v}\n")).unwrap();
            let (kind, games) = if n == "own/game" {
                ("game", Vec::new())
            } else {
                ("content", vec!["own/game".to_owned()])
            };
            publish_cmd(
                spec,
                stage.path(),
                (n, ver),
                (kind, n),
                &games,
                None,
                Writing::default(),
            )
            .unwrap();
        }
    }

    fn listed_now(spec: &str) -> Vec<String> {
        listed(&*open(spec)).unwrap().iter().map(at).collect()
    }

    #[test]
    fn reindex_keeps_what_is_listed_and_names_what_it_would_drop() {
        let (_d, spec) = scratch();
        let real = Writing::default();
        put(
            &spec,
            &["own/game@1.0.9", "own/night@0.9.0", "own/clay@1.0.0"],
        );
        let all = ["own/game@1.0.9", "own/night@0.9.0", "own/clay@1.0.0"].map(String::from);
        // The explicit form, every version named: as before.
        let o = reindex_cmd(&spec, (&all, false, &[]), None, real).unwrap();
        assert!(
            o.text.starts_with("rewrote the list files, not signed"),
            "{}",
            o.text
        );
        assert_eq!(
            listed_now(&spec),
            ["own/night@0.9.0", "own/clay@1.0.0", "own/game@1.0.9"]
        );
        // Adding one with --keep keeps the others, and a new game version
        // replaces the old one in the catalog without a --drop.
        put(&spec, &["own/night@0.9.2", "own/game@1.0.10"]);
        let new = ["own/night@0.9.2", "own/game@1.0.10"].map(String::from);
        let o = reindex_cmd(&spec, (&new, true, &[]), None, real).unwrap();
        assert!(
            o.text
                .contains("added own/night@0.9.2, own/game@1.0.10; dropped none"),
            "{}",
            o.text
        );
        assert_eq!(
            listed_now(&spec),
            [
                "own/night@0.9.0",
                "own/clay@1.0.0",
                "own/night@0.9.2",
                "own/game@1.0.10"
            ]
        );
        let catalog = fs::read_to_string(Path::new(&spec).join("catalog.toml")).unwrap();
        assert!(catalog.contains("latest = \"0.9.2\""), "{catalog}");
        // No versions at all: the same lists again.
        let before = fs::read_to_string(Path::new(&spec).join("by-game/own/game.toml")).unwrap();
        reindex_cmd(&spec, (&[], false, &[]), None, real).unwrap();
        let after = fs::read_to_string(Path::new(&spec).join("by-game/own/game.toml")).unwrap();
        let lines = |t: &str| {
            t.lines()
                .filter(|l| !l.starts_with("signed") && !l.starts_with("expires"))
                .map(String::from)
                .collect::<Vec<_>>()
        };
        assert_eq!(lines(&before), lines(&after));
        // The explicit form that leaves a listed version off is refused,
        // and names it; nothing is written.
        let some = ["own/game@1.0.10", "own/night@0.9.2"].map(String::from);
        let e = reindex_cmd(&spec, (&some, false, &[]), None, real)
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("own/night@0.9.0, own/clay@1.0.0"), "{e}");
        assert_eq!(
            fs::read_to_string(Path::new(&spec).join("by-game/own/game.toml")).unwrap(),
            after
        );
        // With each named in --drop, it goes ahead.
        let drop = ["own/night@0.9.0", "own/clay@1.0.0"].map(String::from);
        let o = reindex_cmd(&spec, (&some, false, &drop), None, real).unwrap();
        assert!(
            o.text.contains("dropped own/night@0.9.0, own/clay@1.0.0"),
            "{}",
            o.text
        );
        assert_eq!(listed_now(&spec), ["own/night@0.9.2", "own/game@1.0.10"]);
        // --keep with --drop takes one out; a --drop of a version not listed is refused.
        let one = ["own/night@0.9.2".to_owned()];
        reindex_cmd(&spec, (&[], true, &one), None, real).unwrap();
        assert_eq!(listed_now(&spec), ["own/game@1.0.10"]);
        assert!(reindex_cmd(&spec, (&[], true, &one), None, real).is_err());
    }

    /// Every argument of every gp registry command says what it is, with
    /// an example, and so does every command.
    #[test]
    fn every_registry_argument_has_help_with_an_example() {
        use clap::Subcommand as _;
        let cmd = RegistryCmd::augment_subcommands(clap::Command::new("registry"));
        for sub in cmd.get_subcommands() {
            let about = sub
                .get_long_about()
                .map(|a| a.to_string())
                .unwrap_or_default();
            assert!(
                about.contains("Example: gp registry"),
                "{}: {about}",
                sub.get_name()
            );
            for a in sub.get_arguments() {
                let help = a.get_help().map(|h| h.to_string()).unwrap_or_default();
                assert!(
                    help.contains("Example:"),
                    "gp registry {} {}: {help:?}",
                    sub.get_name(),
                    a.get_id()
                );
            }
        }
    }

    /// The owner's rule for an arcade game (#3): `gp registry publish`
    /// refuses one unless the score record in its evidence/ for its model
    /// and corpus as they are has every clip exact on state and every
    /// compared screen frame the same on picture, with the package's
    /// mapping and ROM, and says how to get there. A made-up package and
    /// made-up records: no ROM is read.
    #[test]
    fn an_arcade_game_is_published_only_when_every_clip_scores_exact() {
        let (_d, spec) = scratch();
        let g = tempfile::tempdir().unwrap();
        let dir = g.path();
        let rom = "ab".repeat(32);
        let put = |f: &str, t: &str| {
            let p = dir.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, t).unwrap();
        };
        put(
            "package.toml",
            &format!(
                "[package]\nname = \"own/cab\"\nversion = \"0.1.0\"\nkind = \"game\"\ntitle = \"Cab\"\n\n[[rom.entry]]\nid = \"cab\"\nrole = \"scored\"\nsha256 = \"{rom}\"\nplatform = \"arcade\"\nsystem = \"cab\"\nasset_map = \"assets/cab.toml\"\nasset_map_sha256 = \"00\"\n"
            ),
        );
        put("model.toml", "# made up for the test\n");
        put("mapping.toml", "# made up mapping\n");
        put(
            "inputs/a.start.presses",
            "gp-presses 0\nframes 4\nstart@1-2\n",
        );
        put("inputs/a.presses", "gp-presses 0\nframes 10\nright@0-5\n");
        put(
            "corpus.toml",
            &format!(
                "[corpus]\nformat = \"1.0.0\"\nname = \"cab\"\nversion = \"1.0.0\"\nrecipes = \"x\"\n\n[target]\ngame = \"cab\"\nrom = \"{rom}\"\n\n[[clip]]\nid = \"cab.1\"\nname = \"one\"\nsource = \"played\"\nstart = \"inputs/a.start.presses\"\npresses = \"inputs/a.presses\"\nframes = 10\n"
            ),
        );
        let publish_as = |version: &str| {
            publish_cmd(
                &spec,
                dir,
                ("own/cab", version),
                ("game", "Cab"),
                &[],
                None,
                Writing::default(),
            )
        };
        let publish = || publish_as("0.1.0");
        let published = || {
            fs::read_dir(Path::new(&spec).join("packages"))
                .map(|d| d.count())
                .unwrap_or(0)
        };
        // No record: refused, saying how to get one, and nothing written.
        let e = publish().err().unwrap().to_string();
        assert!(
            e.contains("own/cab is an arcade game")
                && e.contains("holds no score record")
                && e.contains("gp corpus score GAME_DIR --run publish --write GAME_DIR")
                && e.ends_with("Nothing was published"),
            "{e}"
        );
        assert_eq!(published(), 0);

        let model = crate::corpus_score::model_key("# made up for the test\n");
        let corpus = crate::corpus_score::load(dir).unwrap().hash;
        let file = crate::corpus_score::record_path(dir, &model, &corpus);
        let h = |t: &str| gp_evidence::Hash::of(t.as_bytes()).to_string();
        // `screen` is the clip's (same, compared) screen frames, on both
        // layers; None for a row with no screen counts.
        let record = |exact: u64, mapping: &str, screen: Option<(u64, u64)>| {
            let mut row = json!({"id": "cab.1", "exact": exact == 1});
            if let Some((same, total)) = screen {
                row["screen"] = json!({
                    "sprites": {"same": same, "total": total},
                    "background": {"same": total, "total": total},
                });
            }
            let v = json!({
                "predicateType": crate::corpus_score::PREDICATE,
                "predicate": {
                    "corpus": {"name": "cab", "version": "1.0.0", "hash": corpus},
                    "rom": {"sha256": rom},
                    "model": {"hash": model},
                    "mapping": {"hash": h(mapping)},
                    "contract": h(""),
                    "gp": {"version": "0.0.0", "build": "gp test"},
                    "results": {"clips": 1, "exact": exact, "errors": 0},
                    "clips": [row],
                },
            });
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, v.to_string()).unwrap();
        };
        // A record with a clip not exact: refused, naming it.
        record(0, "# made up mapping\n", Some((40, 40)));
        let e = publish().err().unwrap().to_string();
        assert!(
            e.contains("has 0 of 1 clips exact on state")
                && e.contains("(not exact: cab.1)")
                && e.contains("picture: 40 of 40 screen frames the same"),
            "{e}"
        );
        assert_eq!(published(), 0);
        // Every clip exact, but scored with another mapping: refused.
        record(1, "# another mapping\n", Some((40, 40)));
        let e = publish().err().unwrap().to_string();
        assert!(e.contains("its mapping.toml isn't the one"), "{e}");
        assert_eq!(published(), 0);
        // Every clip exact on state, but a screen frame differs on the
        // picture: refused, saying state and picture apart and naming it.
        record(1, "# made up mapping\n", Some((39, 40)));
        let e = publish().err().unwrap().to_string();
        assert!(
            e.contains("has 1 of 1 clips exact on state; picture: 39 of 40 screen frames the same")
                && e.contains("1 of 1 clips differ on picture (differ on picture: cab.1)"),
            "{e}"
        );
        assert_eq!(published(), 0);
        // Exact on state with no screen frames compared: refused too.
        record(1, "# made up mapping\n", None);
        let e = publish().err().unwrap().to_string();
        assert!(
            e.contains("picture: 0 of 0 screen frames the same")
                && e.contains("with no screen frames compared"),
            "{e}"
        );
        assert_eq!(published(), 0);
        // Every clip exact on state and every screen frame the same on the
        // package as it is: published, and it says so.
        record(1, "# made up mapping\n", Some((40, 40)));
        let o = publish().unwrap();
        assert!(
            o.text.contains(
                "own/cab is an arcade game: all 1 clips of its corpus cab 1.0.0 score exact on state and all 40 of their screen frames are the same on picture"
            ),
            "{}",
            o.text
        );
        assert_eq!(published(), 1);
        // A game with hooks: a record from before records named the module
        // is refused, one naming another module than play/module.wasm too,
        // and one naming the module it ships is taken.
        let pkg = fs::read_to_string(dir.join("package.toml")).unwrap();
        put(
            "package.toml",
            &format!("{pkg}\n[hooks]\ncrate = \"ext\"\n"),
        );
        let e = publish_as("0.2.0").err().unwrap().to_string();
        assert!(
            e.contains("from before score records named the hooks module"),
            "{e}"
        );
        put("play/module.wasm", "made-up module bytes");
        let with_module = |hash: &str| {
            let mut v: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
            v["predicate"]["module"] = json!({"hash": hash});
            fs::write(&file, v.to_string()).unwrap();
        };
        with_module(&h("another module"));
        let e = publish_as("0.2.0").err().unwrap().to_string();
        assert!(e.contains("and the package ships"), "{e}");
        with_module(&h("made-up module bytes"));
        publish_as("0.2.0").unwrap();
        assert_eq!(published(), 2);
        // A clip changed after the record: the record is for another corpus.
        fs::write(
            dir.join("inputs/a.presses"),
            "gp-presses 0\nframes 10\nright@0-6\n",
        )
        .unwrap();
        let e = publish_as("0.3.0").err().unwrap().to_string();
        assert!(
            e.contains("its one record is for another model or corpus"),
            "{e}"
        );
    }
}
