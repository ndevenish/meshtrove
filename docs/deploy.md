# Deploying MeshTrove

MeshTrove ships as a single container: one Rust binary that serves both the API
and the built React SPA. It needs a PostgreSQL database and a directory (volume)
for the content-addressed blob store. The binary applies its own schema
migrations at startup, so there is no separate migrate step to run.

The quickest path is the bundled `docker-compose.prod.yml`, which runs Postgres
and the app together. You can also run the image against your own Postgres.

---

## Quick start (Docker Compose)

From the repo root:

```bash
cp .env.prod.example .env.prod
```

Edit `.env.prod` and set at least:

- `POSTGRES_PASSWORD` — any strong random string.
- `MESHTROVE_COOKIE_KEY` — the session signing key. Generate one with:
  ```bash
  openssl rand -base64 64 | tr -d '\n'
  ```
  Keep it stable: if it changes, everyone is logged out.
- `MESHTROVE_CREATE_ADMIN` — `username:password` for the first admin.

Then start it:

```bash
docker compose --env-file .env.prod -f docker-compose.prod.yml up -d
```

This pulls the prebuilt `ghcr.io/ndevenish/meshtrove:latest` image — nothing is
built locally. `pull_policy: always` means every `up` re-pulls, so a redeploy
picks up the newest published image.

The app is now on <http://localhost:3000> (change the host port with
`MESHTROVE_PORT`). Log in with the admin credentials you set, then **change the
admin password in the UI** (Account menu → Change password) and blank out
`MESHTROVE_CREATE_ADMIN` in `.env.prod` — it re-applies that password on every
start.

Useful follow-ups:

```bash
docker compose --env-file .env.prod -f docker-compose.prod.yml logs -f app
docker compose --env-file .env.prod -f docker-compose.prod.yml down      # stop
docker compose --env-file .env.prod -f docker-compose.prod.yml up -d      # redeploy on the latest image
```

Data lives in two named volumes and survives `down`/`up`:

- `pgdata` — the Postgres database.
- `store` — uploaded model/blob files (`/app/store` in the container).

`store/imports` is the **dropbox**: drop an archive or a model folder in there on
the host and an admin can stage it from the Importing page with a button, instead
of uploading bytes that are already on the machine. It is created at startup, and
must be readable by the UID the container runs as (see `PUID`/`PGID` below).

---

## ZFS-backed storage (optional)

If the host runs ZFS, back the two persistent stores with dedicated datasets
instead of Docker named volumes. They have opposite I/O profiles — Postgres does
small random 8 KB page I/O with its own WAL, while the blob store holds large,
immutable, content-addressed files — so give each its own dataset and tune it
accordingly. Substitute your pool name for `tank`:

```bash
# Parent — organizational only, not mounted itself; children inherit the defaults.
zfs create -o canmount=off -o mountpoint=/srv/meshtrove \
           -o atime=off -o xattr=sa -o compression=lz4 \
           tank/meshtrove

# Postgres: 16k records suit its 8 KB page workload; it has its own cache + WAL.
zfs create -o recordsize=16k \
           -o logbias=throughput \
           -o primarycache=metadata \
           tank/meshtrove/pgdata

# Blob store: large immutable archives → big records, minimal metadata overhead.
zfs create -o recordsize=1M \
           tank/meshtrove/store
```

Why these settings:

- **`recordsize=16k` (pgdata)** — Postgres does 8 KB page I/O. `8k` matches it
  exactly (no random-access amplification) but bloats metadata and hurts
  compression; `16k` is the common compromise. Drop to `8k` only for heavy OLTP.
- **`recordsize=1M` (store)** — blobs are written once and read sequentially, so
  a large record minimizes metadata and maximizes throughput.
- **`compression=lz4`** — nearly free and it early-aborts on incompressible data.
  The store is mostly already-compressed archives so it gains little there, but
  Postgres data compresses well. Use `zstd` on the store only if you archive lots
  of raw STL/ASCII and want the ratio.
- **`primarycache=metadata` + `logbias=throughput` (pgdata)** — Postgres caches
  data in its own `shared_buffers`, so double-caching file data in ARC is wasteful.
  These are the standard Postgres-on-ZFS tunings.
- **No dedup** — the store is already content-addressed (sha256-keyed), so it
  dedupes at the application layer; ZFS dedup would just burn RAM.

Set ownership to match the container users (bind mounts pass host UIDs straight
through): Postgres runs as UID `999`, the app as UID `10001`.

```bash
chown 999:999     /srv/meshtrove/pgdata
chown 10001:10001 /srv/meshtrove/store
```

Then point the containers at the dataset mountpoints. In
`docker-compose.prod.yml`, replace the two volume lines with bind mounts (commented
hints are already in the file) and delete the bottom `volumes:` block:

```yaml
  postgres:
    volumes:
      - /srv/meshtrove/pgdata:/var/lib/postgresql/data
  app:
    volumes:
      - /srv/meshtrove/store:/app/store
```

Bind mounts are the simplest, most robust way to land the containers on ZFS — no
ZFS Docker volume driver required.

### Snapshots and backups

A ZFS snapshot of `pgdata` is crash-consistent: Postgres replays its WAL on the
next start and comes up clean, so periodic `zfs snapshot` / `zfs send` is a valid
backup (keep `pg_dump` around for a portable logical dump).

The datasets reference each other — DB rows point at blobs in the store — so when
snapshotting or restoring, take **`pgdata` first, then `store`**. That keeps the
store a superset of what the DB references; the worst case is a few orphan blobs
(harmless), never a DB row pointing at a missing file.

---

## Building the image on its own

```bash
docker build -t meshtrove:latest --build-arg APP_VERSION="$(git describe --tags --always --dirty)" .
```

`APP_VERSION` is optional — it's only the string reported by
`meshtrove --version`; it defaults to `docker`. The build uses the committed
sqlx offline cache (`backend/.sqlx`, built with `SQLX_OFFLINE=true`), so **no
database is needed at build time**.

> If you change any SQL query, regenerate the cache and commit it, otherwise the
> image build will fail to compile:
> ```bash
> cd backend
> DATABASE_URL=postgres://meshtrove:meshtrove@localhost:5432/meshtrove cargo sqlx prepare
> ```

## Running the image against your own Postgres

```bash
docker run -d --name meshtrove \
  -p 3000:3000 \
  -v meshtrove-store:/app/store \
  -e DATABASE_URL="postgres://user:pass@db-host:5432/meshtrove" \
  -e MESHTROVE_COOKIE_KEY="$(openssl rand -base64 64 | tr -d '\n')" \
  -e MESHTROVE_CREATE_ADMIN="admin:change-me" \
  meshtrove:latest
```

The database must exist; the app creates and migrates the schema inside it on
startup.

---

## Configuration reference

Every setting is both a CLI flag and an environment variable (`meshtrove --help`
lists them all). The ones that matter for a deployment:

| Variable | Default | Notes |
|---|---|---|
| `DATABASE_URL` | — (required) | Postgres connection string. |
| `MESHTROVE_COOKIE_KEY` | ephemeral | Base64 of ≥64 bytes. **Set it** in production, or every restart logs everyone out. |
| `MESHTROVE_CREATE_ADMIN` | — | `username:password`; ensures that admin exists at startup (re-applies the password each start). Empty = skip. |
| `MESHTROVE_BIND_ADDR` | `0.0.0.0:3000` (in image) | Listen address. |
| `MESHTROVE_STORE_DIR` | `/app/store` (in image) | Blob store dir — put a volume here. |
| `MESHTROVE_STATIC_DIR` | `/app/static` (in image) | Built SPA; already populated in the image. |
| `RUST_LOG` | `info` | Log verbosity. |

`MESHTROVE_COOKIE_KEY_FILE` is also accepted (path to a file holding the key),
which pairs well with Docker/Kubernetes secrets.

Do **not** set `MESHTROVE_DEV_MODE` or `MESHTROVE_ANONYMOUS` in production —
dev mode proxies the frontend to a Vite server, and anonymous mode makes every
request a synthetic admin with no login at all.

---

## Reverse proxy / TLS

The container speaks plain HTTP on port 3000. For a public deployment put it
behind a TLS-terminating reverse proxy (Caddy, nginx, Traefik) and forward to
the app. Session cookies are `SameSite=Lax` and `HttpOnly`; serving the site
over HTTPS is strongly recommended.

Large uploads stream end-to-end, so make sure the proxy does not impose a small
request-body limit (e.g. nginx `client_max_body_size`).

---

## Automatic redeploy on a new image

`docker-compose.prod.yml` already sets `pull_policy: always`, so a manual
`docker compose … up -d` always picks up the newest `:latest`. This section wires
that up to happen automatically whenever CI publishes a new image — no cron, no
polling, and nothing exposed to the public internet except one authenticated
endpoint.

The mechanism has three parts:

1. **A [Watchtower][wt] sidecar** (the actively-maintained `nickfedor` fork) runs
   next to the app with the Docker socket mounted. It has no schedule, so it does
   nothing until it receives an authenticated `POST /v1/update`; then it pulls the
   newest image for the labelled `app` container and recreates it with identical
   config. It is already in `docker-compose.prod.yml` — delete the `watchtower`
   service if you'd rather redeploy by hand.
2. **The CI workflow** (`.github/workflows/docker.yml`) sends that POST from its
   `merge` job, but only *after* the multi-arch `:latest` manifest is published —
   so the host is never told to pull an image that isn't fully live yet.
3. **A Cloudflare tunnel + Access policy** exposes the Watchtower endpoint to
   GitHub's runners and authenticates the caller at the edge.

[wt]: https://github.com/nicholas-fedor/watchtower

Watchtower binds `:8080` but the compose file does **not** publish it to the
host — reaching it is the tunnel's job. Two independent secrets gate every call:
a Cloudflare Access **service token**, checked at the edge, and Watchtower's own
**Bearer token** (`WATCHTOWER_TOKEN`), checked by the container. A request must
carry both.

### 1. Set the Watchtower token

Generate a token and put it in `.env.prod`:

```bash
echo "WATCHTOWER_TOKEN=$(openssl rand -base64 48 | tr -d '\n')" >> .env.prod
```

Bring the stack up as usual (`docker compose --env-file .env.prod -f
docker-compose.prod.yml up -d`). The `watchtower` service starts alongside the
app; `docker compose logs watchtower` should show it listening with the update
endpoint enabled.

### 2. Route the tunnel to Watchtower

Add a public hostname (e.g. `meshtrove-deploy.example.com`) to your Cloudflare
tunnel that points at the Watchtower service. **How you name the service depends
on where `cloudflared` runs:**

- **`cloudflared` as a container on this compose network** — attach it to the
  same network and target `http://watchtower:8080`. No host port is published;
  the two containers talk over the Docker network. This is the tightest setup.
- **`cloudflared` on the host** — it can't resolve `watchtower`, so publish the
  port to loopback only by adding to the `watchtower` service:

  ```yaml
      ports:
        - "127.0.0.1:8080:8080"
  ```

  and target `http://localhost:8080` from the tunnel. Loopback-only means the
  port is still not reachable from the LAN or internet.

The public URL to give GitHub is that hostname plus the endpoint path, e.g.
`https://meshtrove-deploy.example.com/v1/update`.

### 3. Protect it with a Cloudflare Access service token

In the Cloudflare Zero Trust dashboard:

1. **Access → Service Auth → Service Tokens**: create one (e.g.
   `meshtrove-ci`). Copy the **Client ID** and **Client Secret** — the secret is
   shown once.
2. **Access → Applications**: add a self-hosted application for the hostname
   (optionally scoped to the `/v1/update` path) with a single policy: *Action:
   Service Auth*, include the service token above. This makes Cloudflare reject
   any request that doesn't present valid `CF-Access-Client-Id` /
   `CF-Access-Client-Secret` headers, before it ever reaches your host.

### 4. Add the GitHub Actions secrets

In the repository's **Settings → Secrets and variables → Actions**, add:

| Name | Kind | Value |
| --- | --- | --- |
| `DEPLOY_WEBHOOK_URL` | **Variable** | `https://meshtrove-deploy.example.com/v1/update` |
| `DEPLOY_WEBHOOK_TOKEN` | Secret | the same value as `WATCHTOWER_TOKEN` in `.env.prod` |
| `CF_ACCESS_CLIENT_ID` | Secret | the service token's Client ID |
| `CF_ACCESS_CLIENT_SECRET` | Secret | the service token's Client Secret |

`DEPLOY_WEBHOOK_URL` is a **repository variable** (the "Variables" tab), read as
`vars.DEPLOY_WEBHOOK_URL` — the hostname isn't sensitive, and the CF Access and
Bearer checks are what gate the call. The other three are **secrets**. Setting
the URL as a secret instead leaves `vars.DEPLOY_WEBHOOK_URL` empty, and the
guarded step silently skips.

The "Trigger production redeploy" step is guarded on `DEPLOY_WEBHOOK_URL`, so
until you add these secrets — and in forks — the build behaves exactly as before
and simply skips the notify step.

### Verifying and operating it

- **Test the path by hand** (from a machine that can reach the tunnel):

  ```bash
  curl -fsS -X POST \
    -H "Authorization: Bearer $WATCHTOWER_TOKEN" \
    -H "CF-Access-Client-Id: <client-id>" \
    -H "CF-Access-Client-Secret: <client-secret>" \
    https://meshtrove-deploy.example.com/v1/update
  ```

  A `200` with a JSON result means the whole chain works. Drop the Bearer header
  and you should get `401` from Watchtower; drop the CF-Access headers and
  Cloudflare should reject it before the host.
- **`docker compose logs -f watchtower`** on the host shows each triggered update
  and what it recreated.
- **Rotating secrets**: change `WATCHTOWER_TOKEN` in `.env.prod` and the
  `DEPLOY_WEBHOOK_TOKEN` GitHub secret together (then `up -d`), or roll the
  Cloudflare service token and update the two `CF_ACCESS_*` secrets.
- **Postgres is never updated** by this — only the container carrying the
  `com.centurylinklabs.watchtower.enable=true` label is in scope. A schema change
  still arrives the normal way: the new app image applies its own migrations at
  startup when Watchtower recreates it.

---

## Preview rendering

Model previews are produced by shelling out to an external renderer — `f3d` by
default (configurable in Admin settings). **It ships in the image**, so previews
render out of the box with no extra setup.

That is most of the image's size, and it is not a small amount: f3d pulls in
VTK, Mesa/LLVM and OpenCASCADE, which takes the image from **200 MB to 1.31 GB**
(measured, arm64). If you would rather have the small image and no previews,
drop `f3d xvfb xauth` from the runtime stage of the `Dockerfile` and rebuild —
everything else functions without a renderer, and render jobs just fail.

Two details worth knowing if you change any of this:

- **The runtime base is `debian:trixie-slim`, for f3d.** Bookworm only packages
  f3d 1.3.1 (2021). Upstream's own `.deb` releases are not an option either —
  they are x86_64-only, and this image is built for arm64.
- **f3d runs under `xvfb-run`.** It needs a GL context and a container has no
  display; both of f3d's headless backends are dead ends on this base (EGL
  reports "Cannot use a EGL context on this platform", OSMesa is refused because
  Debian's VTK is not built with it). So `/usr/local/bin/f3d` is a small wrapper
  that shadows the real binary on `PATH` and execs it under a virtual X server.
  The renderer setting stays the plain `f3d` that works on a developer's
  machine — the display plumbing belongs to the image, not to the app config.
- **`XDG_CACHE_HOME` is set to `/var/cache/meshtrove` (mode 1777).** f3d requires
  a writable cache directory and has no flag to disable it. It derives one from
  `XDG_CACHE_HOME`, else `$HOME/.cache` — and when the container runs as a uid
  with no passwd entry, which is exactly what `MESHTROVE_UID` does, Docker sets
  `HOME=/` and f3d dies trying to create `/.cache/f3d`. The dedicated directory
  is world-writable so any uid works, sticky so they cannot delete each other's
  files.
