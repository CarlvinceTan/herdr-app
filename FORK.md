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
  `src/bin/herdr-autofill-host.rs`). An entry's own `urls` field adds login origins (a
  Microsoft-hosted NUS sign-in) beside its `URL`. The vault CLI is `vault/`, built into the app by
  `scripts/bundle-cef.sh`; the first thing saved makes a vault if there's none. Its home is the
  author's `cstack/vault`: `scripts/sync-vault.sh` copies it here, and `cargo test --manifest-path
  vault/Cargo.toml` tests it.
- Pages move in step with the window: the Metal layer presents in Core Animation's transaction
  once a page exists (`present_with_native_views` in `vendor/gpui-pre-apple`), and pages whose
  panes aren't drawn hide in the same frame (`set_undrawn`).
- settings.json `"pane_chooser": "<mod>"`: a tab's last pane closing leaves that mod, closing a
  space's last tab gives the space a fresh tab with it, and a person's plain new space starts at
  home with it. Tabs and spaces then close only when closed themselves (`src/model.rs`).
- Agents come back after a reboot: Herdr notes the agent each kept terminal runs (a core label)
  and, when the core gives that terminal a fresh shell, types the agent's `resume` command from
  `defaults/programs.json`. Claude Code returns to its exact conversation when a SessionStart hook
  labels its terminal `herdr.session=<id>` (`src/relaunch.rs`).
- `HERDR_BACKGROUND=1` runs a copy for testing out of the way: no Dock icon, never in front or
  taking the keyboard, still drawing and running pages while covered (`src/main.rs`,
  `draw_when_covered` in `vendor/gpui-pre-macos`). AGENTS.md says how tests use it.

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
   `~/.local/share/cef` first: `cargo install export-cef-dir --version '=<cef crate version>'`, then
   `export-cef-dir --force ~/.local/share/cef`. Keep `cef-dll-sys` in Cargo.lock at the same CEF build
   as `cef` (`cargo update -p cef-dll-sys --precise <version>`): a fresh lock picks a newer one.
4. Build and try it on a scratch home before installing (it opens beside the real one):
   `cargo build --release && scripts/bundle-cef.sh release`, then run
   `target/bundle/Herdr.app/Contents/MacOS/Herdr` with `HERDR_APP_HOME=<short path>` (its socket path
   has a length limit), `XDG_STATE_HOME=<another>`, `HERDR_BROWSER_STATE=<another>` (else Chromium shares
   the installed app's profile and won't start), its own `HC_SESSION`, and `HC_TERMINAL_ID`/`HC_SOCKET`
   unset. A hidden copy (`HERDR_BACKGROUND=1`) doesn't load media; check playback in a visible window.
5. `scripts/install-fork.sh`: builds, replaces `/Applications/Herdr.app`, and reopens it.
   `HERDR_WHILE_CLOSED='<command>'` runs a command while Herdr is closed (for edits to files it
   rewrites while running, such as `shared.json`).
   Terminals and agents keep running; they live in herdr-core.
6. Settings > Updates should now show the new version, with upstream up to date.

`cargo test --bin herdr-app` should pass. In release (`--release`), five tests that read element bounds
fail (`fitted_popups_…`, `popup_at_right_edge_snaps_inside` and three find-bar tests) and pass in debug.

The fork's Chromium is a standard CEF build: it plays VP8/VP9/AV1, Opus, Vorbis, MP3 and FLAC, but not
H.264, HEVC or AAC.

## Going back

The official 0.1.11 app was saved to `~/.local/share/herdr-backup/` before this fork replaced it,
with the personal mods as they were. To return to an earlier fork build, check out that commit and
run `scripts/install-fork.sh`.
