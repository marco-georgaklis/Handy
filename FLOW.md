# Handy Flow

A fork of [Handy](https://github.com/cjpais/Handy) tuned to match Wispr Flow:
formatted dictation on the main shortcut, reliable paste, recoverable
dictations. It installs as its own app ("Handy Flow", `com.handyflow.app`)
next to or instead of upstream Handy. What is measured and how is in
[bench/README.md](bench/README.md).

## Releasing a change

On the build Mac (the one with the signing certificate):

```bash
git commit ...                 # a release is built from a commit
scripts/flow-release.sh        # build, sign, publish a GitHub release, install here
```

Add `--test` to run the Rust unit tests first. The signing identity is pinned
in `.flow-deploy.env` (`HANDYFLOW_SIGN_ID=<sha1>`, not committed). Keep it the
same forever: macOS ties the Accessibility grant to the signature.

## Updating the other Macs

```bash
curl -fsSL https://raw.githubusercontent.com/marco-georgaklis/Handy/flow-parity/scripts/flow-install.sh | bash
```

It installs the latest release if it is newer than what is installed, after
checking the download is signed by the expected team. Pass `--schedule` once
(`... | bash -s -- --schedule`) and that Mac checks by itself daily at 04:15.

The first install on a Mac needs three things done by hand, once: turn on
Handy Flow under System Settings > Privacy & Security > Accessibility, allow
the microphone, and add the post-processing API key in the app's settings.
Updates keep all three.

## Pulling in upstream

```bash
git fetch origin && git rebase origin/main    # origin = cjpais/Handy
scripts/flow-release.sh --test
```

The self-updater is disabled in this fork; it would replace the app with the
upstream release.
