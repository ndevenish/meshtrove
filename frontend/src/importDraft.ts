import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'

// An import is a workbench you leave and come back to: the pattern you were
// tuning, the name and metadata you half-typed, the merge choices you made. None
// of it is committed until you press Import, so it lives only in the page's React
// state — and used to die the moment you navigated away. Persist each slice to
// localStorage, keyed by import id, so reopening the import restores exactly where
// you left off. It is cleared (clearImportDraft) once the import commits or is
// discarded — after that the id is gone and the draft is meaningless.
//
// localStorage, not the server: this is per-machine scratch state for an import
// that already lives server-side as staged files. A draft that outlived a browser
// but not a commit is not worth a schema and an endpoint; if you resume on another
// machine you simply start the carve there.

const PREFIX = 'meshtrove:import-draft'
const keyFor = (importId: string, slot: string) => `${PREFIX}:${importId}:${slot}`

/// useState whose value is mirrored to localStorage under (importId, slot), and
/// seeded from there on mount. A drop-in for useState in the import page and its
/// layout panel: same tuple, same setter semantics.
export function useImportDraftState<T>(
  importId: string,
  slot: string,
  initial: T,
): [T, Dispatch<SetStateAction<T>>] {
  const storageKey = keyFor(importId, slot)
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(storageKey)
      return raw !== null ? (JSON.parse(raw) as T) : initial
    } catch {
      // absent, or unparseable from an older shape — fall back to the default
      return initial
    }
  })
  // Mirror on change, but skip the first run: the value we'd write is the one we
  // just read, and rewriting it on every remount is pointless churn.
  const first = useRef(true)
  useEffect(() => {
    if (first.current) {
      first.current = false
      return
    }
    try {
      localStorage.setItem(storageKey, JSON.stringify(value))
    } catch {
      // storage full or disabled — persistence is best-effort, never fatal
    }
  }, [storageKey, value])
  return [value, setValue]
}

/// Drop every slot of an import's draft — call on commit or discard, when the
/// import (and so its draft) ceases to exist.
export function clearImportDraft(importId: string): void {
  try {
    const prefix = `${PREFIX}:${importId}:`
    const stale: string[] = []
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i)
      if (key && key.startsWith(prefix)) stale.push(key)
    }
    for (const key of stale) localStorage.removeItem(key)
  } catch {
    // ignore — a browser with storage disabled had nothing to clear
  }
}
