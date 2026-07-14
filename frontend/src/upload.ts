// File-first upload: a dropped archive becomes an *import* — a staging object
// that is neither a model nor a bundle. Nothing has to guess what the archive
// is while it uploads; the import page asks once it has unpacked.

import { api, uploadWithProgress, type FileRecord, type ImportSummary } from './api'

/// Derive a human name from an archive filename: strip the extension and a
/// leading "DownloadAll_" prefix, turn separators into spaces, and Title Case.
/// Seeds the import's name, and through it the model/bundle it becomes.
export function deriveModelName(filename: string): string {
  const base = filename
    .replace(/\.[^.]+$/, '') // drop extension
    .replace(/^DownloadAll[_-]?/i, '') // Loot-style "DownloadAll_32mm"
    .replace(/[_-]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
  const titled = base
    .split(' ')
    .map((w) => (w ? w[0].toUpperCase() + w.slice(1).toLowerCase() : w))
    .join(' ')
    .trim()
  return titled || 'Untitled import'
}

/// Stage a dropped file: create an import named after it, then upload into it,
/// reporting upload progress (0..1). A `.zip` unpacks in the background; the
/// import page waits for that, then offers model / bundle / existing bundle.
export async function startImport(
  file: File,
  onProgress?: (fraction: number) => void,
): Promise<ImportSummary> {
  const staged = await api.createImport(deriveModelName(file.name))
  const form = new FormData()
  form.append('file', file)
  await uploadWithProgress<FileRecord[]>(
    `/api/imports/${staged.id}/files`,
    form,
    onProgress ?? (() => {}),
  )
  return staged
}
