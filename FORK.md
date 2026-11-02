# This fork

Upstream [herdrdev/herdr-app](https://github.com/herdrdev/herdr-app), plus:

- Browser panes drawn by embedded Chromium (CEF): `vendor/gpui-kit/crates/shell/src/web_view.rs`,
  the `GPUIApplication` hooks in `vendor/gpui-pre-macos/src/platform.rs`, `src/bin/herdr-cef-helper.rs`.
  Chrome's DevTools protocol listens on `127.0.0.1:9333` for agents (`browser_debug_port`).
- An update check that never installs upstream's builds (they'd replace this fork). Settings >
  Updates shows this fork's version beside upstream's latest release, and the sidebar's gear gets a
  badge when upstream is newer (`src/update.rs`, `defaults/frame/settings/main.js`).

- Autofill from a KeePass vault in browser panes: passwords, codes, addresses, cards, the
  save-password offer and passkeys, each after Touch ID (`vendor/gpui-kit/crates/shell/src/autofill/`,
  `src/bin/herdr-autofill-host.rs`). The vault CLI is `vault/`, built into the app by
  `scripts/bundle-cef.sh`; the first thing saved makes a vault if there's none. Its home is the
  author's `cstack/vault`: `scripts/sync-vault.sh` copies it here, and `cargo test --manifest-path
  vault/Cargo.toml` tests it.
- Pages move in step with the window: the Metal layer presents in Core Animation's transaction
  once a page exists (`present_with_native_views` in `vendor/gpui-pre-apple`), and pages whose
  panes aren't drawn hide in the same frame (`set_undrawn`).
- settings.json `"pane_chooser": "<mod>"`: a tab's last pane closing leaves that mod, closing a
  space's last tab gives the space a fresh tab with it, and a person's plain new space starts at
  home with it. Tabs and spaces then close only when closed themselves (`src/model.rs`).

Work happens on branch `cef`. `origin` is the fork, `upstream` is herdrdev. The personal mods that
use the browser (`my-browser`, `my-tabs`, `my-pane` and others) live in `~/.config/herdr-app/mods`,
not here.

## Build needs

- Rust 1.95 (`RUSTUP_TOOLCHAIN=1.95.0`), Zig 0.16 at `~/.local/share/zig-0.16/zig`, cmake, ninja.
- CEF's binary distribution at `~/.local/share/cef`, the same version as the `cef` crate in
  `Cargo.toml` (now `=156.3.0`, CEF 156.0.15).

## Updating to a new upstream release

When Settings > Updates says upstream is newer:

1. `git fetch upstream --tags`, then on `cef`: `git merge vX.Y.Z` (the release tag).
2. Resolve conflicts, keeping both sides: upstream's change and this fork's additions above.
3. If the merge changes the `cef` crate's version, the matching CEF distribution has to be in
   `~/.local/share/cef` first. Agents can't download it here; give the person the command.
4. Build and try it on a scratch home before installing (it opens beside the real one):
   `cargo build --release && scripts/bundle-cef.sh release`, then run
   `target/bundle/Herdr.app/Contents/MacOS/Herdr` with `HERDR_APP_HOME=~/.config/herdr-dev`,
   `XDG_STATE_HOME=~/.local/state/herdr-dev` and the `HC_*` variables unset.
5. `scripts/install-fork.sh`: builds, replaces `/Applications/Herdr.app`, and reopens it.
   `HERDR_WHILE_CLOSED='<command>'` runs a command while Herdr is closed (for edits to files it
   rewrites while running, such as `shared.json`).
   Terminals and agents keep running; they live in herdr-core.
6. Settings > Updates should now show the new version, with upstream up to date.

`cargo test --release --bin herdr-app` should pass, except `fitted_popups_use_their_rendered_size_at_the_window_edge`,
which also fails on upstream on macOS.

## Going back

The official 0.1.11 app was saved to `~/.local/share/herdr-backup/` before this fork replaced it,
with the personal mods as they were. To return to an earlier fork build, check out that commit and
run `scripts/install-fork.sh`.
