import type React from 'react'

/// One raw entry → tags: split on commas, semicolons and newlines — except
/// inside double quotes. Some legitimate tags carry commas (MMF categories
/// like "Arduino, Raspberry & DIY"), and quoting is the escape hatch that
/// keeps them whole: `"a, b", c` is two tags. The quotes are dropped from the
/// result; an unbalanced quote simply stops splitting for the rest of the
/// string.
function splitOne(value: string): string[] {
  const parts: string[] = []
  let buf = ''
  let quoted = false
  for (const ch of value) {
    if (ch === '"') {
      quoted = !quoted
    } else if (!quoted && (ch === ',' || ch === ';' || ch === '\n')) {
      parts.push(buf)
      buf = ''
    } else {
      buf += ch
    }
  }
  parts.push(buf)
  return parts
}

/// Trim, drop blanks, dedupe case-insensitively keeping the first spelling.
function dedupe(parts: readonly string[]): string[] {
  const out: string[] = []
  const seen = new Set<string>()
  for (const part of parts) {
    const tag = part.trim()
    if (tag && !seen.has(tag.toLowerCase())) {
      seen.add(tag.toLowerCase())
      out.push(tag)
    }
  }
  return out
}

/// An onChange handler for a `multiple freeSolo` tag Autocomplete: a newly
/// *typed* entry (reason "createOption" — always the last element) may itself
/// be a separated list, so it goes through the splitter; everything else —
/// chips already committed, options picked from the dropdown — passes through
/// untouched, so an existing tag that happens to contain a comma is never
/// shattered by a later edit.
export const changeTags =
  (set: (tags: string[]) => void) => (_e: unknown, value: readonly string[], reason: string) => {
    if (reason === 'createOption') {
      const typed = value[value.length - 1] ?? ''
      set(dedupe([...value.slice(0, -1), ...splitOne(typed)]))
    } else {
      set(dedupe(value))
    }
  }

/// A paste handler for the same inputs: multi-line clipboard text never
/// reaches onChange intact (the browser collapses newlines to spaces on paste
/// into a single-line input), so a list-looking paste is intercepted here,
/// where the clipboard is still whole, and committed as chips directly. Only
/// the pasted text is split — the chips already present pass through as-is.
export const pasteTags =
  (current: readonly string[], set: (tags: string[]) => void) => (e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData('text')
    if (/[,;\n]/.test(text)) {
      e.preventDefault()
      set(dedupe([...current, ...splitOne(text)]))
    }
  }
