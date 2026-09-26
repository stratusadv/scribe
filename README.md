<p align="center">
    <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/scribe-lockup-outline-on-dark.svg">
        <img src="assets/scribe-lockup-outline-on-light.svg" alt="scribe" width="280">
    </picture>
</p>

A desktop app that turns a meeting recording into formatted notes.

## Overview

A recording dropped onto the window is transcribed on an API. A template then shapes the transcript into notes, which are edited in place and exported to Word or PDF. Each template is a short instruction to the AI, so a meeting summary, a policy document, and a handover document are each one template away.

## Setup

A build made from a checkout whose `.env` holds the API address and the two keys needs no setup. Otherwise, open Settings and enter the API address, the transcription key, and the notes key. A value entered there always wins over the built-in one.

## Build

```
bun install
bun tauri dev
bun tauri build
```

## Release

An installed copy checks for a new version at each launch and offers to install it. A push
of a `v*` tag builds the Windows installer and the Linux packages on GitHub Actions, signs
them, and publishes a release with the installers, their signatures, and the `latest.json`
the updater reads.

The signing key is generated once. The private key never enters the repository, and an
installed copy only ever accepts updates signed by it, so back it up.

```
bun tauri signer generate -w ~/.tauri/scribe.key
```

The public key lives in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. The
private key and its password live in the repository's Actions secrets as
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

For each release, bump `version` in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`,
and `package.json`, commit, then tag and push:

```
git tag v0.2.0
git push origin v0.2.0
```

A local build signs the same way with the key contents in the environment:

```
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/scribe.key)" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=... \
bun tauri build
```
