# Full-Stack Service Template

A reference for scaffolding a new full-stack service in the mould of **SSX Live**:
a Rust/Axum backend that owns configuration, external-service clients, auth, and
real-time streaming, fronted by a React/Vite SPA. The two are stitched together
so that **one binary serves both development and production** with a single flag.

This document is prescriptive: it captures *why* each choice was made so it can
be reused as a starting template rather than reverse-engineered again.

---

## 1. Top-Level Layout

```
project/
├── backend/                 # Rust/Axum server — owns config, auth, services, API
│   ├── src/
│   │   ├── main.rs          # Entry point: logging, service init, router assembly, serve
│   │   ├── config.rs        # CLI args (clap) + resolved Configuration struct
│   │   ├── state.rs         # AppState: one clonable struct holding every service
│   │   ├── extractors.rs    # Auth extractor (User) + permission model
│   │   ├── routes/
│   │   │   ├── api.rs        # /api/* — data, streaming, OpenAPI
│   │   │   ├── auth.rs       # /auth/* — OIDC login/callback/logout
│   │   │   └── frontend.rs   # Catch-all: static serving OR dev proxy to Vite
│   │   └── services/         # One module per external dependency / capability
│   ├── build.rs             # Stamps APP_VERSION from git describe at compile time
│   └── CLAUDE.md
└── frontend/                # React SPA
    ├── src/
    ├── vite.config.ts       # Dev server on :5173, version stamp, dep pre-bundling
    └── package.json
```

**Guiding principle:** the backend is the single front door. In production it
serves the built SPA; in development it transparently proxies to the Vite dev
server. The frontend never needs to know which mode it is in, and there is no
CORS configuration because everything is same-origin.

---

## 2. Configuration (via ENV + CLI)

Configuration flows through **two types** in `config.rs`:

1. **`Arguments`** — a `clap::Parser` struct. This is the raw, un-validated input
   surface. Every field is both a CLI flag *and* an environment variable.
2. **`Configuration`** — the resolved, validated, ready-to-use struct that the
   rest of the app consumes. Secrets are decoded, URLs are normalized, and
   optional clients (like OIDC) are constructed.

`Configuration::load()` calls `Arguments::parse()` and does the transformation.

### 2.1 Every setting is a flag AND an env var

```rust
#[arg(long, env = "DCSERVER_URL", value_parser = parse_url_and_ensure_trailing_slash)]
dcserver_url: Option<Url>,
```

This gives operators three equivalent ways to configure the service — CLI flag,
environment variable, or `.env` file — with a single declaration. `main.rs` calls
`dotenvy::dotenv()` first thing so a local `.env` is picked up automatically in
development.

### 2.2 Secrets: value-or-file, never logged

Every secret has a paired `_FILE` variant, enforced as a mutually-exclusive,
required `ArgGroup`. This supports both inline env vars (dev) and mounted secret
files (k8s / production) without branching in call sites.

```rust
#[command(group(
    ArgGroup::new("dcserver_secrets").required(true)
        .args(["dcserver_token", "dcserver_token_file"])
))]
```

```rust
#[arg(long, hide = true, env = "DCSERVER_TOKEN", hide_env = true)]
dcserver_token: Option<String>,
#[arg(long, env = "DCSERVER_TOKEN_FILE")]
dcserver_token_file: Option<PathBuf>,
```

- `hide = true` / `hide_env = true` keep secrets out of `--help`.
- A `get_<secret>()` accessor reads whichever was provided (trimming whitespace,
  base64-decoding where needed) and returns the usable value. The `unreachable!`
  in the `else` branch is safe because the `ArgGroup` guarantees exactly one.
- On the `Configuration` struct, secret fields are annotated `#[debug(skip)]`
  (via `derive_more::Debug`) so they never leak into log output.

### 2.3 URL normalization at parse time

Two custom `value_parser`s normalize URLs *before* they reach application code,
so no downstream code has to worry about trailing-slash bugs:

- `parse_url_and_ensure_trailing_slash` — for base URLs you'll join paths onto.
- `parse_url_and_strip_trailing_slash` — for endpoints that must not have one.

### 2.4 Conditional requirements

`clap` expresses cross-field rules declaratively, so invalid combinations fail at
startup with a clear message instead of panicking later:

```rust
#[arg(long, required_unless_present = "keycloak_url")]
pub anonymous: bool,

#[arg(long, env = "KEYCLOAK_URL",
      requires = "client_id",
      requires = "client_secret_kinds",
      required_unless_present = "anonymous")]
keycloak_url: Option<Url>,
```

i.e. "you must either enable `--anonymous` OR supply a full Keycloak config."

### 2.5 The dev-mode settings

Three settings drive the development/production split (see §5):

| Flag / env | Default | Purpose |
|------------|---------|---------|
| `--dev` / `DEV_MODE` | `false` | Proxy frontend to Vite instead of serving static files |
| `--vite-url` / `VITE_URL` | `http://localhost:5173` | Where the Vite dev server lives |
| `--static-dir` / `STATIC_DIR` | `../frontend/dist` | Built SPA assets for production |
| `--bind-addr` / `BIND_ADDR` | `127.0.0.1:3000` | Listen address |

---

## 3. AppState & Service Structure

### 3.1 One clonable struct holds everything

`AppState` is the single dependency container passed to every handler via Axum's
`State` extractor. It is `#[derive(Clone)]` and cheap to clone — each service is
either itself cheaply clonable (an `Arc` internally, or a `reqwest::Client` which
is `Arc`-backed) or wrapped in `Arc`.

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Configuration,
    pub zocalo: ZocaloIntercom,     // message-broker intercom
    pub dcserver: DCServer,         // HTTP client (read)
    pub expeye: Expeye,             // HTTP client (write)
    pub cache: Arc<DataCache>,      // shared cache — wrapped because not internally Arc'd
    pub ldap: LdapService,
    // ...one field per service
    ldap_cache: Cache<String, Vec<String>>,  // private: only reached via a method
}
```

**Rule of thumb:** if a service holds shared mutable state and is not already
`Arc`-backed internally, wrap the whole thing in `Arc<...>` in `AppState`.
Otherwise store it by value and let it manage its own sharing.

### 3.2 Construction order lives in `AppState::new`

`AppState::new` is the composition root. It loads config, then builds each
service, threading shared dependencies in explicitly (e.g. the cache is given the
`DCServer` it fetches through). Anything constructed outside (because it needs to
exist before the state, like the broker connection) is passed in as an argument.

```rust
impl AppState {
    pub async fn new(zocalo: ZocaloIntercom) -> Result<AppState> {
        let config = Configuration::load().await?;
        let dcserver = DCServer::new(config.dcserver_url.clone(), &config.dcserver_token)?;
        let cache = Arc::new(DataCache::new(dcserver.clone()));
        // ...
        Ok(AppState { config, zocalo, dcserver, cache, /* ... */ })
    }
}
```

### 3.3 Service module conventions

Each file in `services/` owns one external dependency or capability and follows
the same shape:

- A public struct (e.g. `DCServer`) that is `#[derive(Clone)]`.
- A `new(...)` constructor taking already-resolved config values (a `Url`, a
  `&str` token) — **not** the whole `Configuration`. Services don't reach back
  into global config; they receive exactly what they need.
- Internally: a `reqwest::Client` (clone-cheap), plus any caches wrapped in
  `Arc<Cache<...>>` (using `moka`) so clones share one cache.
- Response types are `#[derive(Deserialize)]` with `#[serde(rename_all = "camelCase")]`
  to map external JSON to Rust snake_case.
- A shared `json_with_path` helper deserializes with `serde_path_to_error` so
  decode failures report the exact JSON path — invaluable against APIs you don't
  control.

### 3.4 Cross-cutting behavior belongs on `AppState`

Logic that combines a service with config (e.g. caching LDAP lookups, or the
dev-mode shortcut that grants blanket access) lives as a method on `AppState`,
not in a handler. This is also where the dev/prod behavioral differences are
centralized:

```rust
pub async fn get_groups_for_user(&self, username: &str) -> Result<Vec<String>> {
    // In dev (non-anonymous), skip the real LDAP round-trip entirely.
    if self.config.dev_mode && !self.config.anonymous {
        return Ok(vec!["<admin-group>".to_string()]);
    }
    self.ldap_cache.try_get_with(username.to_string(),
        async { self.ldap.user_groups(username).await }).await
        .map_err(|e| anyhow::anyhow!("{}", e))
}
```

### 3.5 Wiring `FromRef` for sub-extractors

Extractors that need a specific piece of state (e.g. the cookie `Key` for signed
cookies) get it via `FromRef`, so they can be used without pulling the whole
`AppState`:

```rust
impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self { state.config.cookie_key.clone() }
}
```

---

## 4. Authentication & Request Pipeline

Auth is a `User` extractor in `extractors.rs`, implemented for both
`FromRequestParts` (required) and `OptionalFromRequestParts` (nullable):

- **Anonymous mode short-circuits** — if `config.anonymous`, return a synthetic
  user with full permissions. This is what makes local dev frictionless.
- Otherwise read the signed `PrivateCookieJar` (keyed via the `FromRef` above),
  deserialize the `User`, then enrich it with permissions derived from LDAP
  groups (parsed with regexes into beamline/proposal/session scopes).
- Rejections implement `IntoResponse` so they map to clean 401/502 status codes.

The router in `main.rs` layers cross-cutting concerns as tower middleware:

- `record_user` middleware extracts the `User` once, stamps the username onto the
  tracing span, and stores it in request extensions.
- A `TraceLayer` builds a per-request span (method, path, client IP, user).
- `/health` is mounted **outside** the trace layer so k8s liveness probes don't
  spam logs.
- `/api/version` is unauthenticated so a client can detect a redeploy and prompt
  a reload.
- OpenAPI: routers return their own `utoipa` spec fragments which are `.merge()`d
  and served at `/docs` (Swagger UI) + `/openapi.json`.

Version is stamped at build time (`build.rs` → `APP_VERSION` from `git describe`)
and exposed as `const VERSION`.

---

## 5. The Development / Production Switch (the key trick)

This is the piece that makes "develop, test, and run live" all easy. A **single
catch-all route** (`routes/frontend.rs`) is mounted last, after all API/auth/docs
routes. Its behavior forks on `config.dev_mode`:

```
                    frontend_handler (catch-all: "/" and "/{*path}")
                                    │
              ┌─────────────────────┴─────────────────────┐
        path is /api, /auth,                          everything else
        /docs, /openapi.json?                              │
              │ yes → 404                    ┌──────────────┴──────────────┐
              └──────────────           dev_mode == true         dev_mode == false
                                              │                          │
                                    proxy_to_vite(VITE_URL)     serve_static(STATIC_DIR)
                                    (+ WebSocket proxy for            (SPA fallback
                                     Vite HMR)                         to index.html)
```

### 5.1 Production: serve static, with SPA fallback

`serve_static` serves the built asset if the exact file exists. For extension-less
paths (client-side routes) it falls back to `index.html` so the SPA router works
on deep links / refresh. Files with an extension that don't exist return 404
(never masking a missing asset as HTML).

### 5.2 Development: transparent passthrough to Vite

When `--dev` is set, **every** non-API request is proxied to the Vite dev server:

- `proxy_to_vite` forwards the path+query with `reqwest`, copies the status and
  `content-type` back. The developer gets Vite's on-the-fly compilation, instant
  source, and correct MIME types through the *same* origin and port as the API.
- **WebSocket upgrade requests are detected and proxied separately**
  (`handle_websocket` / `proxy_websocket`) so **Vite HMR works through the
  backend** — edits hot-reload the browser even though traffic goes through the
  Rust server. WebSocket proxying is refused outside dev mode.
- If Vite isn't running, the proxy returns a friendly HTML hint
  (`cd frontend && npm run dev`) instead of an opaque error.

### 5.3 Why this matters

Because the backend is the only origin in every mode:

- **No CORS** ever needs configuring — API and assets are same-origin.
- The frontend code is **identical** in dev and prod; it just calls `/api/...`.
- You test the *real* auth/API path in development, not a mock.
- Live/production is the same binary with `--dev` omitted and `STATIC_DIR`
  pointing at the built assets.

### 5.4 Frontend side of the contract (`vite.config.ts`)

- Dev server pinned to **port 5173** (matches the backend's default `VITE_URL`).
- **HMR is configured to talk to Vite directly** (`host: localhost`, `port: 5173`,
  `protocol: ws`) — the WebSocket proxy in the backend handles the rest.
- `define.__APP_VERSION__` stamps the same git-derived version into the bundle,
  so client and server versions can be compared (redeploy detection).
- `optimizeDeps.include` pre-bundles heavy dependencies for fast cold starts.

---

## 6. Running It

**Development** (two terminals, same origin):

```bash
# terminal 1 — Vite dev server on :5173
cd frontend && npm install && npm run dev

# terminal 2 — backend in dev mode, proxying to Vite
cd backend && cargo run -- --dev --anonymous \
    --dcserver-url <URL> --dcserver-token <KEY> \
    --expeye-url <URL> --expeye-token <KEY>
# open http://localhost:3000  (backend origin — HMR still works)
```

`--anonymous` skips auth; `--dev` turns on the Vite passthrough. URLs/tokens can
equally come from a `.env` file (auto-loaded) or the shell environment.

**Production** (single binary):

```bash
cd frontend && npm run build            # → frontend/dist
cd backend && cargo run --release        # no --dev; serves STATIC_DIR
```

---

## 7. Checklist for a New Project

- [ ] `config.rs`: split `Arguments` (clap) from resolved `Configuration`; every
      setting is `#[arg(long, env = "...")]`.
- [ ] Every secret has a `_FILE` variant in a required, mutually-exclusive
      `ArgGroup`; secrets are `hide`/`hide_env` and `#[debug(skip)]`.
- [ ] URL fields normalized via custom `value_parser`.
- [ ] `dotenvy::dotenv()` called first in `main`.
- [ ] `AppState`: one `Clone` struct; `Arc`-wrap only services that need it;
      construct in `AppState::new`; cross-cutting logic as methods.
- [ ] One `services/*.rs` per external dependency, each with a `new(resolved config)`
      constructor and camelCase-mapped response types.
- [ ] `User` extractor with an anonymous short-circuit for dev.
- [ ] Catch-all `frontend.rs` route forking on `dev_mode`:
      `proxy_to_vite` + WebSocket proxy vs `serve_static` with SPA fallback.
- [ ] `--dev` / `VITE_URL` / `STATIC_DIR` / `BIND_ADDR` settings.
- [ ] Vite on a fixed port, HMR pointed at Vite directly, version stamp shared
      with the backend (`build.rs` + `define.__APP_VERSION__`).
- [ ] `/health` outside tracing; unauthenticated `/api/version`; OpenAPI at `/docs`.
```
