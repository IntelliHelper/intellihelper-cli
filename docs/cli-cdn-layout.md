# cli.intellihelper.in CDN layout

Host these objects at the origin behind **https://cli.intellihelper.in**
(Cloudflare, R2, S3, etc.). Paths are relative to the host root.

## Installers (required)

| Path | Content |
|------|---------|
| `/install.sh` | Unix installer (from `crates/codegen/intellihelper-pager/scripts/install.sh`) |
| `/install.ps1` | Windows installer |
| `/enterprise-install.sh` | Optional enterprise channel |
| `/enterprise-install.ps1` | Optional enterprise channel |

Public one-liners:

```sh
curl -fsSL https://cli.intellihelper.in/install.sh | bash
irm https://cli.intellihelper.in/install.ps1 | iex   # PowerShell
intelli --version
```

## Channel pointers (required for “latest”)

Plain-text files with a single version string, no trailing junk preferred:

| Path | Example body | Current policy |
|------|----------------|----------------|
| `/stable` | `0.1.2` | **Every pipeline run** writes this (clean `X.Y.Z` only) |
| `/alpha` | — | Not used for now |
| `/enterprise` | `0.1.0` | Enterprise installs only |

### Stable-only publish (current)

**Publish CLI** and `scripts/publish-cli-local.sh`:

1. Version = `version_override` or Cargo.toml — must be clean **`X.Y.Z`** (no `-ci` / `-alpha`).
2. Build **all platforms** under that version.
3. Write **`/stable`** only.
4. **Delete** older `intellihelper-*` binaries; keep only the version just published.

Stable clients reject pre-releases. Never put `0.1.2-ci.…` on `/stable`.

Cache: short TTL or `no-cache` so users get new versions quickly.

## Binaries (required)

Name pattern used by installers and the in-app updater:

```
intellihelper-{version}-{os}-{arch}[.exe]
```

| Platform key | Example asset |
|--------------|---------------|
| `macos-aarch64` | `intellihelper-0.1.0-macos-aarch64` |
| `macos-x86_64` | `intellihelper-0.1.0-macos-x86_64` |
| `linux-aarch64` | `intellihelper-0.1.0-linux-aarch64` |
| `linux-x86_64` | `intellihelper-0.1.0-linux-x86_64` |
| `windows-x86_64` | `intellihelper-0.1.0-windows-x86_64.exe` |

These are the **download** names. After install, users run the command **`intelli`**.

Build tip:

```sh
cargo build -p intellihelper-pager-bin --release
# rename target/release/intellihelper-pager → intellihelper-$VER-$platform
```

## Optional

| Path | Purpose |
|------|---------|
| `/changelogs/{version}.md` | In-app / docs changelog fetch |
| `/` | Simple landing page with the install one-liner |

## DNS

1. Create subdomain `cli` on `intellihelper.in` (CNAME or A to your CDN/bucket).
2. Enable HTTPS (Cloudflare proxy or cert on the origin).
3. CORS not required for curl install; keep objects **public-read**.

## Publish checklist per release

1. Bump to a **higher** clean version (`Cargo.toml` or `version_override=0.1.3`) so
   clients already on the previous version will update.
2. Run **Publish CLI** (or local `--upload`) — builds all platforms → `/stable`.
3. Confirm older binaries were pruned (only `intellihelper-{new}-*` remain).
4. Smoke: `curl -fsSL https://cli.intellihelper.in/stable` → clean `X.Y.Z`, then
   `curl -fsSL https://cli.intellihelper.in/install.sh | bash` → `intelli --version`.
