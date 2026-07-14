// File-first upload helpers: turn a dropped archive into a freshly-created,
// auto-named model whose contents unpack into the model's "unsorted" bucket.

import { api, uploadWithProgress, type FileRecord, type ModelDetail } from './api'

/// Derive a human model name from an archive filename:
/// strip the extension and a leading "DownloadAll_" prefix, turn separators
/// into spaces, and Title Case. Never returns an empty string.
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
  return titled || 'Untitled model'
}

/// Create a model named after the file, then upload the file to it, reporting
/// upload progress (0..1). A `.zip` unpacks in the background (import job).
export async function importArchiveAsModel(
  file: File,
  onProgress?: (fraction: number) => void,
): Promise<ModelDetail> {
  const model = await api.createModel({ name: deriveModelName(file.name) })
  const form = new FormData()
  form.append('file', file)
  await uploadWithProgress<FileRecord[]>(
    `/api/models/${model.id}/files`,
    form,
    onProgress ?? (() => {}),
  )
  return model
}
