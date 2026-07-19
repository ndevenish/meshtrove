import type React from 'react'

/// Normalise a freeSolo tag Autocomplete's value: any entry may itself be a
/// comma/semicolon/newline-separated list (a paste of "bolt, jar; thread" plus
/// Enter arrives as one string), so each is split into real tags — trimmed,
/// blanks dropped, deduped case-insensitively with the first spelling kept.
/// Chips already in the list pass through this too, which is what makes the
/// paste land as several chips instead of one long one.
export function splitTags(values: readonly string[]): string[] {
  return dedupe(values.flatMap((value) => value.split(/[,;\n]/)))
}

/// A paste handler for the same inputs: multi-line clipboard text never
/// reaches onChange intact (the browser collapses newlines to spaces on paste
/// into a single-line input), so a list-looking paste is intercepted here,
/// where the clipboard is still whole, and committed as chips directly.
export const pasteTags =
  (current: readonly string[], set: (tags: string[]) => void) => (e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData('text')
    if (/[,;\n]/.test(text)) {
      e.preventDefault()
      set(splitTags([...current, text]))
    }
  }

function dedupe(parts: string[]): string[] {
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
