# Where a registry can live

A registry is static files, so the same format works on four kinds of target. `gp registry init`, `publish` and `reindex` take any of them.

| Target | Named | Write-once |
|---|---|---|
| A folder | `PATH` | The file system refuses a key that exists |
| A git repo | `git:PATH`, a checkout | The same, in the checkout, which is brought up to the remote's tip first |
| An S3 bucket | `s3://BUCKET` | S3's `--if-none-match` |
| A web address | `https://...` | Read only, for checking one |

## A registry in a git repo

Make a repo, clone it, and point the commands at the checkout. Each command that writes makes one commit, and pushes when the repo has an `origin`. It refuses a checkout with uncommitted changes, and a command that fails commits nothing.

A repo's history keeps everything it's given, so before a package goes in, `gp` compares every one of its files with each ROM the package names. Thirty-two matching bytes stop the publish, and so does a ROM it can't find in `$GP_ROM_DIR`.

For players to reach it, the repo must be public. A page built for it names the raw address, in `scripts/web/sources.toml`:

```toml
[repo]
name = "gp-repo"
url = "https://raw.githubusercontent.com/OWNER/REPO/main"
root = "ssh-ed25519 ..."
```

GitHub's raw host answers with `Access-Control-Allow-Origin: *` and `Cross-Origin-Resource-Policy: cross-origin`, both of which the page needs. It caches each file for five minutes, so a new version can take that long to show.

The root key works as on any other target: the page trusts the key, wherever the files are.

## Signing is opt-in

A reader checks signatures only for a source it holds a root key for. For the page that is the `root` line of the source in `scripts/web/sources.toml`. For `gp` it is `--root`. The choice belongs to the reader, so a registry can't turn its own checks off.

With no root, signatures and expiries aren't looked at, and the list files never need refreshing. Every package manifest and file is still checked against its hash. That catches damage. It doesn't catch someone who can write to the repo, who could publish a new version, so with no signing the repo's write access is the protection.

To make a registry with no signing, leave out `--root-key`, `--signing-key` and `--key`, and give `--namespace NS` with no key file. To opt in later, make keys, run `scripts/root-ceremony.sh`, sign every file again and add `root` to the source.
