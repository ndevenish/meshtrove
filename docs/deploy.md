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

Then build and start:

```bash
docker compose --env-file .env.prod -f docker-compose.prod.yml up -d --build
```

The app is now on <http://localhost:3000> (change the host port with
`MESHTROVE_PORT`). Log in with the admin credentials you set, then **change the
admin password in the UI** (Account menu → Change password) and blank out
`MESHTROVE_CREATE_ADMIN` in `.env.prod` — it re-applies that password on every
start.

Useful follow-ups:

```bash
docker compose --env-file .env.prod -f docker-compose.prod.yml logs -f app
docker compose --env-file .env.prod -f docker-compose.prod.yml down      # stop
docker compose --env-file .env.prod -f docker-compose.prod.yml up -d --build   # redeploy after a code change
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

## Preview rendering (optional)

Model previews are produced by shelling out to an external renderer — `f3d` by
default (configurable in Admin settings). It is **not** installed in the image,
so uploads work but auto-generated thumbnails won't render until you provide a
renderer. To enable it, build a derived image that installs `f3d` (plus the Mesa
GL libraries it needs to run headless) and point the renderer setting at it.
Everything else in the app functions without it.
