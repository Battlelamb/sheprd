# Self-hosted macOS runners

GitHub Team runs at most five hosted macOS jobs at once across the account,
and a pull request or merge queue entry asks for up to four (`checks` with
default and all features, and the two `build-macos` release builds). Runs
waited one to three hours for a macOS runner while Linux and Windows
finished in about 35 minutes. Our own Macs take those jobs instead.

## What runs where

`CI_MACOS_RUNNER` is a repository variable (Settings > Secrets and variables >
Actions > Variables) holding a JSON list of runner labels:

```json
["self-hosted", "macOS", "ARM64", "tart"]
```

When it is set, `.github/workflows/ci.yml` sends the `checks` macOS legs and
`build-macos` to runners with every label in it, for members' code only:

| Run | macOS runner |
|---|---|
| Pull request from a branch here, by the owner, a member, or a collaborator | Our Macs |
| `main`, the merge queue | Our Macs |
| Pull request from a fork | Hosted `macos-15` |
| Pull request a bot opened (Dependabot) | Hosted `macos-15` |

The author must be one GitHub lists as `OWNER`, `MEMBER`, or `COLLABORATOR`
on the pull request, so a fork's code never reaches our machines, nor does a
bot's: Dependabot's branches live here too, but bring in new dependencies'
build scripts. A member whose organization membership is private may be
listed otherwise, and then runs hosted. Linux, Windows, Nix, the
release workflow, and the iOS workflow stay hosted.

Every pull request runs the macOS default-features leg. One that needs every
platform (`scripts/ci-scope.py`) also runs the all-features leg on our Macs;
a fork's skips it, as it runs on the account's few hosted macOS runners.

Unset or empty, every job uses hosted runners, so deleting the variable is
the way back.

## Trust

A runner that outlives its job is shared by every run it takes, and a run's
build scripts and tests execute as the runner's user: they can leave a
doctored `target/`, a binary in `~/.cargo/bin`, or a LaunchAgent for the
next run. Only members' code runs on these runners, the same trust `main`
has. A runner that starts each job in a fresh Tart VM keeps nothing between
jobs, at the cost of building cold.

## Organization runners and a public repository

Runners registered on the organization join a runner group, and a group
serves public repositories only once Settings > Actions > Runner groups >
(the group) > Allow public repositories is on. This repository is public:
without it its jobs wait for a runner forever.

## Preparing a Mac

1. In the image or user the runner works as, install the Xcode command line
   tools, Homebrew, and `brew install rustup python@3.12 jq openssl@3`,
   then `rustup-init -y`. The jobs need Python 3.11 or later first on
   `PATH`, `jq`, and `brew --prefix openssl@3`.
2. Install Rosetta (`softwareupdate --install-rosetta --agree-to-license`):
   the x86_64 release build runs its CLI tests under it.
3. Register the runner with the labels `CI_MACOS_RUNNER` names, and run it
   as a service so it survives logouts.
4. Set `CI_MACOS_RUNNER` to those labels.

One runner takes one job at a time. With fewer runners than the jobs a few
runs ask for at once, runs wait for each other.

## The build directory

A self-hosted job checks out without `git clean` and runs
`scripts/ci-clean-self-hosted.sh`, which removes everything Git does not
track except `target/`. On a runner that outlives its job, the next build
then compiles only what changed, and the hosted Rust cache is skipped. The
script deletes `target/` once it grows past `SHEPRD_CI_TARGET_LIMIT_GB` (80
by default; set it in the runner's `.env` file). In a fresh VM per job there
is no `target/` to keep and the step is harmless.
