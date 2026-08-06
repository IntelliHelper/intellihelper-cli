<div align="center">

# IntelliHelper CLI (`intelli` / `intellihelper`)

**IntelliHelper CLI** is a terminal-based AI coding agent. It runs as a
full-screen TUI that understands your codebase, edits files, executes shell
commands, searches the web, and manages long-running tasks — interactively,
headlessly for scripting/CI, or embedded in editors via the Agent Client
Protocol (ACP).

[Installing the released binary](#installing-the-released-binary) ·
[Updating](#updating) ·
[Building from source](#building-from-source) ·
[Documentation](#documentation) ·
[Repository layout](#repository-layout) ·
[Development](#development) ·
[Contributing](#contributing) ·
[License](#license)

**Install & downloads: [cli.intellihelper.in](https://cli.intellihelper.in)**

This repository contains the Rust source for the IntelliHelper CLI/TUI and its
agent runtime. Both command names are supported after install: **`intelli`**
(primary) and **`intellihelper`** (alias).

</div>

---

## Installing the released binary

Prebuilt binaries are published for macOS, Linux, and Windows. The installer
shows **download progress** in interactive terminals and installs both
`intelli` and `intellihelper` on your `PATH`.

```sh
curl -fsSL https://cli.intellihelper.in/install.sh | bash   # macOS / Linux / Git Bash
irm https://cli.intellihelper.in/install.ps1 | iex          # Windows PowerShell
intelli --version
# same binary:
intellihelper --version
```

See GitHub [Releases](https://github.com/IntelliHelper/intellihelper-cli/releases)
for fixes and features in each version.

## Updating

Managed installs (installer / npm / GitHub Releases) check for new versions
automatically by default. You will see when an update is available in the TUI
or terminal; the new binary can download in the background, or you can update
explicitly:

```sh
intelli update              # or: intellihelper update
intelli update --check      # report only
```

Download progress is shown during interactive install and update. Configure
behavior in `~/.intellihelper/config.toml`:

```toml
[cli]
auto_update = true          # default when unset
# channel = "stable"        # or "alpha" / "enterprise"
```

## Building from source

Requirements:

- **Rust** — the toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml);
  `rustup` installs it automatically on first build.
- **[DotSlash](https://dotslash-cli.com)** — required so hermetic tools under
  [`bin/`](bin/) (notably [`bin/protoc`](bin/protoc)) can download and run.
  Install it and ensure `dotslash` is on your `PATH` **before** building:

  ```sh
  cargo install dotslash
  # or: prebuilt packages — https://dotslash-cli.com/docs/installation/
  /usr/bin/env dotslash --help   # sanity check
  ```

- **protoc** — proto codegen resolves [`bin/protoc`](bin/protoc) via DotSlash,
  or falls back to a `protoc` on `PATH` / `$PROTOC`.
- macOS and Linux are supported build hosts; Windows builds are best-effort
  and not currently tested from this tree.

```sh
cargo run -p intellihelper-pager-bin              # build + launch the TUI
cargo build -p intellihelper-pager-bin --release  # release binary: target/release/intellihelper-pager
cargo check -p intellihelper-pager-bin            # fast validation
```

The binary artifact is named `intellihelper-pager`; official installs ship it as
`intelli` and `intellihelper`. Configure models via `~/.intellihelper/config.toml`
(BYOK) — see the
[authentication guide](crates/codegen/intellihelper-pager/docs/user-guide/02-authentication.md).

## Documentation

Install endpoint: [cli.intellihelper.in](https://cli.intellihelper.in)
(CDN layout: [`docs/cli-cdn-layout.md`](docs/cli-cdn-layout.md)).

The user guide ships with the pager crate:
[`crates/codegen/intellihelper-pager/docs/user-guide/`](crates/codegen/intellihelper-pager/docs/user-guide/)
— getting started, keyboard shortcuts, slash commands, configuration, theming,
MCP servers, skills, plugins, hooks, headless mode, sandboxing, and more.

## Repository layout

| Path | Contents |
|------|----------|
| `crates/codegen/intellihelper-pager-bin` | Composition-root package; builds the `intellihelper-pager` binary |
| `crates/codegen/intellihelper-pager` | The TUI: scrollback, prompt, modals, rendering |
| `crates/codegen/intellihelper-shell` | Agent runtime + leader/stdio/headless entry points |
| `crates/codegen/intellihelper-tools` | Tool implementations (terminal, file edit, search, ...) |
| `crates/codegen/intellihelper-workspace` | Host filesystem, VCS, execution, checkpoints |
| `crates/codegen/...` | The rest of the CLI crate closure (config, MCP, markdown, sandbox, ...) |
| `crates/common/`, `crates/build/`, `prod/mc/` | Small shared leaf crates pulled in by the closure |
| `third_party/` | Vendored upstream source (Mermaid diagram stack) — see below |

> [!IMPORTANT]
> The root `Cargo.toml` (workspace members, dependency versions, lints,
> profiles) is **generated** — treat it as read-only. Prefer editing per-crate
> `Cargo.toml` files.

## Development

```sh
cargo check -p <crate>        # always target specific crates; full-workspace builds are slow
cargo test -p intellihelper-config # per-crate tests
cargo clippy -p <crate>       # lint config: clippy.toml at the repo root
cargo fmt --all               # rustfmt.toml at the repo root
```

## Contributing

> [!NOTE]
> External contributions are not accepted. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

First-party code in this repository is licensed under the **Apache License,
Version 2.0** — see [`LICENSE`](LICENSE).

Third-party and vendored code remains under its original licenses. See:

- [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES) — crates.io / git dependencies,
  bundled UI themes, and **in-tree source ports** (including openai/codex and
  sst/opencode tool implementations)
- [`crates/codegen/intellihelper-tools/THIRD_PARTY_NOTICES.md`](crates/codegen/intellihelper-tools/THIRD_PARTY_NOTICES.md)
  — crate-local notice for the codex and opencode ports (license texts +
  Apache §4(b) change notice)
- [`third_party/NOTICE`](third_party/NOTICE) — vendored Mermaid-stack index
