---
name: verify
description: Build/launch/drive recipe for verifying MeshTrove changes end-to-end in the running app.
---

# Verifying MeshTrove

## Stack is usually already running

Check before starting anything: `docker ps` (postgres), `curl -so /dev/null
-w '%{http_code}' http://localhost:3001/api/imports` (backend, 200 = up),
same for `http://localhost:5173/` (Vite). The backend on **:3001** proxies
to Vite, and Vite serves the working tree with HMR — frontend edits are
live at :3001 within a couple of seconds, no rebuild step. `--anonymous`
dev mode means no login: every request is a synthetic admin.

If not running, follow the Run section in CLAUDE.md.

## Driving the UI

No Playwright in the repo. Install it in the session scratchpad
(`npm init -y && npm i playwright`) and launch with
`chromium.launch({ channel: 'chrome' })` — Google Chrome is installed, so
no browser download. Write `.mjs` scripts, top-level await works.

Gotchas:
- MUI Autocomplete: `getByLabel(...)` matches both the input and the open
  listbox → strict-mode violation. Use
  `getByRole('combobox', { name: '...' })` for the input,
  `locator('li[role="option"]')` for options.
- Import page: wait for the file list to actually mount before interacting
  or measuring — the "N files staged" header appears well before the rows:
  `waitForFunction(() => document.querySelectorAll('svg[data-testid="InsertDriveFileIcon"]').length > <most-of-N>)`.

## Useful fixtures

- Staged imports: `GET /api/imports` — an existing large import is the best
  perf fixture. `tools/hollow-zip` clones a real archive's structure with
  empty files; drop the result in `<store>/imports` (the dropbox) and stage
  it from the Importing page.
- Creators/tags: create via `POST /api/creators` etc. There is **no DELETE
  /api/creators/{id}** — clean up test rows via
  `docker exec meshtrove-postgres-1 psql -U meshtrove -d meshtrove -c "DELETE FROM creators WHERE id='...'"`.

## Perf measurements

Long-task counting per interaction works well:
`new PerformanceObserver(...).observe({ entryTypes: ['longtask'] })` then
`pressSequentially(text, { delay: 0 })` and read the entries. Let the page
idle ~3s after mount first, or the list-mount long task pollutes the
numbers. For a before/after: `git stash push -- frontend/src`, wait ~3s for
HMR, measure, `git stash pop`.
