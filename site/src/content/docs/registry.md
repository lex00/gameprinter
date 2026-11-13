---
title: Run a registry
description: Publish games and themes to a registry of your own, in a folder or a git repo.
---

A registry is a set of static files that holds published games and themes. Each published version is a package: the folder's files, each named by a hash of its bytes, and a list of them, of kind `game` or `content` (a theme). You can run your own on anything that serves files.

## Make one and publish to it

```sh
gp registry init ~/my-registry --name my-games --namespace me
gp registry publish ~/my-registry path/to/my-theme \
  --name me/my-theme --version 1.0.0 --kind content \
  --title "My Theme" --for lex00/donkey-kong
gp registry reindex ~/my-registry me/my-theme@1.0.0
gp registry check ~/my-registry
```

`reindex` rewrites the lists readers use to find things, so name every version that should be listed. The last version of a package you name becomes its latest.

## Where it can live

| Target | Written as | Notes |
|---|---|---|
| A folder | A path | For testing |
| A git repo | `git:PATH` to a checkout | Each command commits and pushes. Published files can't be replaced |
| An S3 bucket | `s3://BUCKET` | Uses your AWS command-line login |

A page reads a registry over https, with CORS allowed. `gp registry check URL` tests a public address the way a browser would. To try one on your machine with a page you've built, use `gp registry serve DIR --page PAGE`.

[Where a registry can live](https://github.com/lex00/gameprinter/blob/main/docs/registry-backends.md) has the details.

## Signing

Signing is off unless you opt in. Every file is still checked against its hash, which catches damage, but anyone who can write to your registry can publish to it. Signing matters once you have other publishers. The same document covers how to turn it on.

## Not built yet

Taking submissions from other people: an inbox, review, accepting and refusing. The player page reads one registry and can't yet add another, so a registry of your own is mainly for your own use today.
