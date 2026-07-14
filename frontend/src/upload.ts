// File-first upload: a dropped file *or folder* becomes an *import* — a staging
// object that is neither a model nor a bundle. Nothing has to guess what the
// drop is while it uploads; the import page asks once it has unpacked.

import { api, uploadWithProgress, type FileRecord, type ImportSummary } from './api'

/// One staged file plus the logical folder it sits in ('' at the root). Mirrors
/// the `files` table: the blob is content-addressed, the folder lives in `path`.
export type StagedFile = { file: File; path: string }

/// A drop, resolved: every file it contains, and a name derived from whatever
/// was actually dropped (the folder, or the single file).
export type Drop = { name: string; files: StagedFile[] }

/// Derive a human name: strip a leading "DownloadAll_" prefix, turn separators
/// into spaces, and Title Case. Seeds the import's name, and through it the
/// model/bundle it becomes. Folders keep their dots (a `v1.2` folder is not an
/// extension), so only filenames get the extension stripped.
export function deriveModelName(name: string, stripExtension = true): string {
  const base = (stripExtension ? name.replace(/\.[^.]+$/, '') : name)
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

/// Walk one dropped entry, collecting files with the folder path they sit under.
/// The dropped folder's own name becomes the root of those paths, so an unzipped
/// folder stages identically to the same folder zipped.
async function walk(entry: FileSystemEntry, dir: string, out: StagedFile[]): Promise<void> {
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) =>
      (entry as FileSystemFileEntry).file(resolve, reject),
    )
    out.push({ file, path: dir })
    return
  }
  const reader = (entry as FileSystemDirectoryEntry).createReader()
  const childDir = dir ? `${dir}/${entry.name}` : entry.name
  // readEntries() yields a batch at a time (~100), not the whole directory —
  // keep calling until it comes back empty or deep folders silently truncate.
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) =>
      reader.readEntries(resolve, reject),
    )
    if (!batch.length) break
    for (const child of batch) await walk(child, childDir, out)
  }
}

/// Resolve a drop into files. A dropped *directory* arrives in `dataTransfer.files`
/// looking like a file — same name, a size — and only blows up when something
/// tries to read its bytes (Firefox: NS_ERROR_FILE_IS_DIRECTORY; Chrome/Safari:
/// a silent zero-byte upload). `webkitGetAsEntry()` is the only thing that can
/// tell them apart, and only inside the drop handler, so read the entries
/// synchronously before the first await neuters the DataTransfer.
export async function readDrop(dataTransfer: DataTransfer): Promise<Drop> {
  const entries = Array.from(dataTransfer.items)
    .filter((item) => item.kind === 'file')
    .map((item) => item.webkitGetAsEntry())
    .filter((entry): entry is FileSystemEntry => !!entry)
  const bare = Array.from(dataTransfer.files)

  const files: StagedFile[] = []
  if (entries.length) {
    for (const entry of entries) await walk(entry, '', files)
  } else {
    // No entry API (rare): can't recurse, but these are all plain files anyway.
    for (const file of bare) files.push({ file, path: '' })
  }

  const folder = entries.length === 1 && entries[0].isDirectory ? entries[0].name : null
  const name = folder ? deriveModelName(folder, false) : deriveModelName(files[0]?.file.name ?? '')
  return { name, files }
}

/// Same, for the click-to-browse `<input type="file">` fallback. A directory
/// picker (`webkitdirectory`) sets `webkitRelativePath`; a plain file leaves it
/// empty. Either way the folder is everything but the last segment.
export function readFileList(list: FileList): Drop {
  const files: StagedFile[] = Array.from(list).map((file) => ({
    file,
    path: file.webkitRelativePath.split('/').slice(0, -1).join('/'),
  }))
  const root = files[0]?.path.split('/')[0]
  const name = root ? deriveModelName(root, false) : deriveModelName(files[0]?.file.name ?? '')
  return { name, files }
}

/// Stage a drop: create an import named after it, then upload every file into it
/// with its folder preserved, reporting progress (0..1) across the whole tree. A
/// `.zip` unpacks in the background; the import page waits for that, then offers
/// model / bundle / existing bundle.
export async function startImport(
  drop: Drop,
  onProgress?: (fraction: number) => void,
): Promise<ImportSummary> {
  if (!drop.files.length) throw new Error('Nothing to import — that folder is empty')

  const staged = await api.createImport(drop.name)
  try {
    // The multipart contract: a `path` field applies to every `file` that follows
    // it, so one pair per file carries the whole tree up in a single request.
    const form = new FormData()
    for (const { file, path } of drop.files) {
      form.append('path', path)
      form.append('file', file)
    }
    await uploadWithProgress<FileRecord[]>(
      `/api/imports/${staged.id}/files`,
      form,
      onProgress ?? (() => {}),
    )
  } catch (err) {
    // The import row exists before the bytes do; a failed upload would otherwise
    // strand an empty one in the Importing list, unpackable and uncommittable.
    await api.deleteImport(staged.id).catch(() => {})
    throw err
  }
  return staged
}
