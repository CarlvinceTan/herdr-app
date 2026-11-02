# vault

A local KeePass (`.kdbx`) vault as a stateless Rust CLI: one short-lived process per command, the
master password in the macOS Keychain (service "OpenCode Vault", account = the database path) or
`VAULT_PASSWORD`. Herdr builds it into the app (`Herdr.app/Contents/MacOS/vault`), and browser
autofill (`src/bin/herdr-autofill-host.rs`) runs it, after Touch ID, for passwords, codes,
addresses, cards and passkeys.

Its home is the author's `cstack/vault`; `scripts/sync-vault.sh` copies the Rust source here.

```sh
cargo test --manifest-path vault/Cargo.toml
vault status                       # where the database is, whether its password is available
vault create --password-stdin      # a new database at that path, its password kept in the Keychain
vault list | get <title> --field password | totp <title> | put <title> ... | fields <title>
vault passkey list [--rp RPID] | create | sign   # create and sign take one JSON request on stdin
```

The database is `~/Library/Application Support/vault/vault.kdbx`, unless
`~/.config/opencode/vault.json` (`{"path": "..."}`) or `VAULT_FILE` names another. Any KeePass app
can open it. Passkeys use KeePassXC's `KPEX_PASSKEY_*` fields.
