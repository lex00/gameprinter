# The official root key ceremony

The official registry's root key is made once, by its owner. Every player trusts this key. It's needed again only to add a namespace or replace a signing key, and if it's lost, players have to move to a new page with a new key.

Run this yourself, in your own terminal, so that the passphrase and private key pass through no tool or agent:

```sh
cd ~/checkouts/lex00/gameprinter && scripts/root-ceremony.sh
```

The script asks for what it needs. It makes the root key and two online keys, creates the registry at `s3://gp-official-354867293429`, reads it back from outside to check the signature, has you attach the key to a password manager entry, then deletes the local copy. If it stops, it's safe to run it again.

Keep the passphrase apart from the password manager, on paper or somewhere else. The key file is useless without it, and one breach shouldn't expose both.

`migration.md` says the key is "held offline with a sealed backup". The owner chose a password manager instead on 2026-11-08, so the encrypted key is stored online and the password manager is its backup.
