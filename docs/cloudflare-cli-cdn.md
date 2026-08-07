# Cloudflare setup for cli.intellihelper.in

Goal: users run:

```bash
curl -fsSL https://cli.intellihelper.in/install.sh | bash
intelli --version
```

Use **Cloudflare R2** as the file host (binaries + install scripts).  
DNS for `cli.intellihelper.in` is managed in the same Cloudflare zone as `intellihelper.in`.

---

## 1. Confirm the domain is on Cloudflare

1. [Cloudflare Dashboard](https://dash.cloudflare.com) → select **intellihelper.in**.
2. **DNS** should show nameservers as Cloudflare (orange cloud optional for most records; R2 custom domains handle `cli` for you).

If the domain is not yet on Cloudflare: **Add site** → enter `intellihelper.in` → set your registrar nameservers to the ones Cloudflare gives you → wait until status is **Active**.

---

## 2. Create an R2 bucket

1. Sidebar → **R2 Object Storage** → **Create bucket**.
2. Name (example): `intellihelper-cli`
3. Location: **Automatic** is fine.
4. Create bucket.

You do **not** need a public “allow anyone” bucket policy if you attach a **custom domain** (recommended below).

---

## 3. Attach custom domain `cli.intellihelper.in`

1. Open the bucket → **Settings** → **Custom Domains** (or **Public access** → **Connect domain**).
2. Domain: `cli.intellihelper.in`
3. Cloudflare will create the DNS record and TLS certificate.
4. Wait until status is **Active** / **Connected**.

Test (will 404 until you upload files — that’s OK):

```bash
curl -I https://cli.intellihelper.in/
```

You want HTTPS **200/404**, not connection errors or cert errors.

---

## 4. Upload the installers (minimum)

From this repo root:

```bash
# Unix installer
npx wrangler r2 object put intellihelper-cli/install.sh \
  --file=crates/codegen/intellihelper-pager/scripts/install.sh \
  --content-type=text/x-shellscript \
  --remote

# Windows installer
npx wrangler r2 object put intellihelper-cli/install.ps1 \
  --file=crates/codegen/intellihelper-pager/scripts/install.ps1 \
  --content-type=text/plain \
  --remote
```

Or use the dashboard: bucket → **Upload** → choose those two files with **exact** keys:

- `install.sh`
- `install.ps1`

Keys must be at the **bucket root** (not nested), so URLs are:

- `https://cli.intellihelper.in/install.sh`
- `https://cli.intellihelper.in/install.ps1`

Verify:

```bash
curl -fsSL https://cli.intellihelper.in/install.sh | head -5
```

---

## 5. Publish a first binary + channel pointer

### 5a. Build (on your Mac)

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p intellihelper-pager-bin --release

# Detect platform for this machine
# Apple Silicon example:
VER=0.1.0
PLATFORM=macos-aarch64   # or macos-x86_64 / linux-x86_64 / linux-aarch64

cp target/release/intellihelper-pager \
  "/tmp/intellihelper-${VER}-${PLATFORM}"
chmod +x "/tmp/intellihelper-${VER}-${PLATFORM}"
```

Platform names the installer expects:

| Your machine | `PLATFORM` |
|--------------|------------|
| Mac Apple Silicon | `macos-aarch64` |
| Mac Intel | `macos-x86_64` |
| Linux x86_64 | `linux-x86_64` |
| Linux ARM64 | `linux-aarch64` |
| Windows x64 | `windows-x86_64` (+ `.exe` suffix) |

### 5b. Upload binary

```bash
npx wrangler r2 object put \
  "intellihelper-cli/intellihelper-${VER}-${PLATFORM}" \
  --file="/tmp/intellihelper-${VER}-${PLATFORM}" \
  --content-type=application/octet-stream \
  --remote
```

### 5c. Channel pointer `stable`

Create a tiny file containing **only** the version (one line):

```bash
echo -n "$VER" > /tmp/stable
npx wrangler r2 object put intellihelper-cli/stable \
  --file=/tmp/stable \
  --content-type=text/plain \
  --remote
```

Dashboard: upload object key `stable` with body `0.1.0` (no quotes).

Verify:

```bash
curl -fsSL https://cli.intellihelper.in/stable
# → 0.1.0

curl -fsSL -o /tmp/t "https://cli.intellihelper.in/intellihelper-${VER}-${PLATFORM}"
file /tmp/t
```

---

## 6. Smoke-test install

```bash
curl -fsSL https://cli.intellihelper.in/install.sh | bash
intelli --version
```

Expect:

- Download of `intellihelper-{ver}-{platform}`
- Symlink `~/.intellihelper/bin/intelli`
- `intelli` on PATH (or restart shell / ensure `~/.intellihelper/bin` is on PATH)

---

## 7. Optional: Wrangler login (CLI uploads)

```bash
npm i -g wrangler
# or: npx wrangler ...
wrangler login
```

Account ID: R2 overview page. You can put a minimal `wrangler.toml` later; CLI `--remote` + bucket name is enough for one-off puts.

---

## 8. Optional: Cache rules

In Cloudflare for the zone:

| Path | Cache |
|------|--------|
| `/stable`, `/alpha` | Short TTL or bypass (users should see new versions fast) |
| `/intellihelper-*` versioned binaries | Long cache (immutable per version) |
| `/install.sh`, `/install.ps1` | Short/medium (scripts change occasionally) |

---

## 9. GitHub Actions manual publish

Workflow: [`.github/workflows/publish-cli.yml`](../.github/workflows/publish-cli.yml)

**Manual only** (Actions → **Publish CLI** → **Run workflow**). Does not run on push.

1. Builds `intellihelper-pager` for linux/mac/windows
2. Uploads install scripts + binaries + `stable` pointer to R2
3. **Deletes previous** `intellihelper-*` binaries (only the newly published
   version’s platform assets are kept; installers and `stable` stay)
4. Public URL remains `https://cli.intellihelper.in/...`

### Secrets to add (repo → Settings → Secrets and variables → Actions)

| Secret | Where to get it |
|--------|------------------|
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare dashboard right sidebar / R2 overview |
| `R2_ACCESS_KEY_ID` | R2 → **Manage R2 API Tokens** → Create → Object Read & Write |
| `R2_SECRET_ACCESS_KEY` | Same token create dialog (shown once) |
| `R2_BUCKET_NAME` | Optional; default `intellihelper-cli` |

### One-time Cloudflare setup before the first green run

1. R2 bucket exists (name matches `R2_BUCKET_NAME` or `intellihelper-cli`)
2. Custom domain **`cli.intellihelper.in`** connected to that bucket and **Active**
3. Secrets above saved on the **IntelliHelper/intellihelper-cli** GitHub repo

### Versioning and channels (stable only for now)

| How you run Publish CLI | Version string | CDN |
|-------------------------|----------------|-----|
| Default (no override) | Cargo.toml `X.Y.Z` | **`/stable`** + all platforms |
| `version_override=0.1.3` | that clean version | same |
| Pre-release (`0.1.2-ci…`) | **rejected** | job fails |

Every run:

1. Builds all platforms under that version  
2. Writes **`/stable`**  
3. **Deletes** older `intellihelper-*` binaries  

Bump the version each release so users on the previous `X.Y.Z` pick up the update.

### After secrets are set

```bash
# GitHub → Actions → Publish CLI → Run workflow
# Optional: version_override = 0.1.2  (else uses Cargo.toml)
```

Then:

```bash
curl -fsSL https://cli.intellihelper.in/install.sh | bash
intelli --version
intelli update
```

## 10. Manual per-release checklist (without CI)

1. Build multi-arch binaries with a **clean** `VERSION=X.Y.Z` (higher than previous).
2. Upload each `intellihelper-{ver}-{platform}`.
3. Set **`/stable`** to that version.
4. Re-upload install scripts only if they changed.
5. Delete older `intellihelper-*` binaries (`publish-cli-local.sh --upload` does this).
6. Smoke: `curl -fsSL https://cli.intellihelper.in/stable` → clean `X.Y.Z`, then install.

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| DNS / SSL error | Custom domain not Active yet; wait or re-connect domain on the bucket |
| `install.sh` 404 | Wrong object key (must be exactly `install.sh` at root) |
| Version fetch fails | Missing `stable` object or it has HTML/error body |
| Binary download 404 | Platform string mismatch (e.g. you uploaded `darwin-arm64` but installer wants `macos-aarch64`) |
| Binary fails to run | Wrong arch, or unsigned macOS binary blocked — run once from Terminal or `xattr -cr` on the binary for local testing |
| Install succeeds but `intelli` not found | `export PATH="$HOME/.intellihelper/bin:$PATH"` or open a new terminal |

---

## What the code already expects

From this repo:

- Primary CDN: `https://cli.intellihelper.in`
- Installer + updater download: `{base}/stable` and `{base}/intellihelper-{version}-{platform}`
- User command after install: **`intelli`**

See also: [cli-cdn-layout.md](./cli-cdn-layout.md).

## 11. First-time R2 API token (screenshot path)

1. Cloudflare → **R2** → **Overview** → **Manage R2 API Tokens**
2. **Create API token**
3. Permissions: **Object Read & Write**
4. Apply to bucket: `intellihelper-cli` (or Allow all buckets for simplicity)
5. Create → copy **Access Key ID** and **Secret Access Key** into GitHub secrets
6. Copy **Account ID** from the R2 overview page into `CLOUDFLARE_ACCOUNT_ID`
