# Self-hosted macOS runners

GitHub Team runs at most five hosted macOS jobs at once across the account,
and each merge queue entry asks for up to four (`checks` with default and all
features, and the two `build-macos` release builds). Entries waited one to
three hours for a macOS runner while Linux and Windows finished in about 35
minutes. Our own Macs take those jobs instead.

## What runs where

`CI_MACOS_RUNNER` is a repository variable (Settings > Secrets and variables >
Actions > Variables) holding a JSON list of runner labels:

```json
["self-hosted", "macOS", "ARM64", "sheprd-ci"]
```

When it is set, `.github/workflows/ci.yml` sends these jobs to runners with
every label in it:

- `checks` macOS legs in the merge queue and on `main`
- `build-macos` in the merge queue

Pull requests, including one labeled `macos` or `full-ci`, always use hosted
`macos-15`. A fork's code reaches our machines only after a maintainer queues
it. The release workflow, Nix, and the iOS workflow stay hosted.

Unset or empty, every job uses hosted runners, so deleting the variable is
the way back.

## Preparing a Mac

Use a dedicated macOS user for the runner: queued code runs as that user, and
its build scripts can read whatever that user can.

1. Install the Xcode command line tools, Homebrew, and
   `brew install rustup python@3.12 jq openssl@3`, then `rustup-init -y`.
   The jobs need Python 3.11 or later first on `PATH`, `jq`, and
   `brew --prefix openssl@3`.
2. Install Rosetta (`softwareupdate --install-rosetta --agree-to-license`):
   the x86_64 release build runs its CLI tests under it.
3. Register the runner from Settings > Actions > Runners > New self-hosted
   runner, adding the extra label (`sheprd-ci` above) with `--labels`, and
   install it as a service (`./svc.sh install && ./svc.sh start`) so it
   survives logouts.
4. Set `CI_MACOS_RUNNER` to its labels.

One runner takes one job at a time. A Mac with 32 GB or more can run two or
three runner instances side by side, each registered from its own directory.
With fewer instances than the four jobs an entry asks for, entries wait for
each other, so register enough of them before setting the variable.

## The build directory

A self-hosted job checks out without `git clean` and runs
`scripts/ci-clean-self-hosted.sh`, which removes everything Git does not
track except `target/`. The next build then compiles only what changed
instead of starting cold, and the hosted Rust cache is skipped. The script
deletes `target/` once it grows past `SHEPRD_CI_TARGET_LIMIT_GB` (80 by
default; set it in the runner's `.env` file). Each runner instance has its
own checkout and so its own `target/`.
