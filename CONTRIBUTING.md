# Contributing to Mayak

Thanks for wanting to help. Issues and pull requests are welcome at
[mahach666/mayak](https://github.com/mahach666/mayak).

## Getting started

You need [Rust](https://rustup.rs), [Node 20+](https://nodejs.org) and, on Windows, the MSVC build tools
(Visual Studio Build Tools with "Desktop development with C++"); on a Mac, Xcode's command-line tools.

```bash
cd windows
npm install
npm run tauri dev
```

Before a pull request:

```bash
npx tsc --noEmit
npm run check            # the front end's checks
cargo test --workspace   # the Rust tests and the relay's
```

`npm run check -- <name>` runs only the checks whose name contains `<name>`.

## Good first contributions

- A new service integration: a small poller and a tile, with its key in the credential store.
- Support for another agent CLI. Say which version you ran it with: what was checked against a real CLI is
  written down as such, and an adapter written from documentation only is marked unverified.
- A translation fix. The dictionaries are in `windows/src/core/i18n/`, one file per language.
- Bug fixes. Please describe how to reproduce.

## Rules of the house

- No third-party dependencies unless there is really no other way.
- Secrets go in the credential store, never on disk or in git.
- No telemetry; network calls only to services the user configured.
- Never block an agent, never write an agent's config without a backup, a diff and the user's click, never
  approve anything without a click.
- Do not restyle what already ships unless the issue asks for it.
- Keep a change small, with a test for what it changes. `CLAUDE.md` has the same rules for AI coding agents.
