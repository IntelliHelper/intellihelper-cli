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

| Path | Example body |
|------|----------------|
| `/stable` | `0.1.0` |
| `/alpha` | `0.1.1-alpha.1` |
| `/enterprise` | `0.1.0` |

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

1. Build multi-arch binaries.
2. Upload `intellihelper-{ver}-{platform}` assets.
3. Upload updated `install.sh` / `install.ps1` if scripts changed.
4. Write `/stable` (and `/alpha` if needed) to the new version string.
5. **Delete** older `intellihelper-*` binaries for previous versions (CI and
   `scripts/publish-cli-local.sh --upload` do this automatically after upload).
   Install scripts and channel pointers are never deleted by that prune step.
6. Smoke test: `curl -fsSL https://cli.intellihelper.in/install.sh | bash` then `intelli --version`.
