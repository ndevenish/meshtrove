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

/// Mac/Windows filesystem cruft that rides along inside dropped folders but is
/// never a model file. Mirrors the server's own skip (backend routes/files.rs
/// `is_os_junk`), so junk never wastes an upload — and never lingers "pending"
/// in a batch retry, which reconciles against what actually staged.
function isOsJunk({ file, path }: StagedFile): boolean {
  return file.name === '.DS_Store' || path.split('/').includes('__MACOSX')
}

/// Derive a human name: strip a leading "DownloadAll_" prefix, split camel case,
/// turn separators into spaces, and Title Case. Seeds the import's name, and
/// through it the model/bundle it becomes. Folders keep their dots (a `v1.2`
/// folder is not an extension), so only filenames get the extension stripped.
export function deriveModelName(name: string, stripExtension = true): string {
  const base = (stripExtension ? name.replace(/\.[^.]+$/, '') : name)
    .replace(/^DownloadAll[_-]?/i, '') // Loot-style "DownloadAll_32mm"
    // `KnightRider` -> `Knight Rider`, `STLKnight` -> `STL Knight`: a run of
    // capitals is an acronym, and breaks only before the word it runs into.
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
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

  const all: StagedFile[] = []
  if (entries.length) {
    for (const entry of entries) await walk(entry, '', all)
  } else {
    // No entry API (rare): can't recurse, but these are all plain files anyway.
    for (const file of bare) all.push({ file, path: '' })
  }
  const files = all.filter((f) => !isOsJunk(f))

  const folder = entries.length === 1 && entries[0].isDirectory ? entries[0].name : null
  const name = folder ? deriveModelName(folder, false) : deriveModelName(files[0]?.file.name ?? '')
  return { name, files }
}

/// Same, for the click-to-browse `<input type="file">` fallback. A directory
/// picker (`webkitdirectory`) sets `webkitRelativePath`; a plain file leaves it
/// empty. Either way the folder is everything but the last segment.
export function readFileList(list: FileList): Drop {
  const files: StagedFile[] = Array.from(list)
    .map((file) => ({
      file,
      path: file.webkitRelativePath.split('/').slice(0, -1).join('/'),
    }))
    .filter((f) => !isOsJunk(f))
  const root = files[0]?.path.split('/')[0]
  const name = root ? deriveModelName(root, false) : deriveModelName(files[0]?.file.name ?? '')
  return { name, files }
}

// A big drop is uploaded in bounded batches, not one all-or-nothing request: a
// single hiccup partway through a multi-GB tree used to lose the whole thing (and
// tie one request up for minutes). A batch is capped by both a byte budget and a
// file count, whichever fills first.
const MAX_BATCH_BYTES = 128 * 1024 * 1024 // file bodies per request
const MAX_BATCH_FILES = 50 // …or this many files
const BATCH_ATTEMPTS = 3 // tries per batch before the whole import gives up

/// Greedily pack files into batches bounded by a byte budget and a file count. A
/// file bigger than the budget rides in a batch of its own — one file can't be
/// split across requests without a chunked protocol we don't have.
function planBatches(files: StagedFile[]): StagedFile[][] {
  const batches: StagedFile[][] = []
  let batch: StagedFile[] = []
  let bytes = 0
  for (const sf of files) {
    if (
      batch.length > 0 &&
      (batch.length >= MAX_BATCH_FILES || bytes + sf.file.size > MAX_BATCH_BYTES)
    ) {
      batches.push(batch)
      batch = []
      bytes = 0
    }
    batch.push(sf)
    bytes += sf.file.size
  }
  if (batch.length) batches.push(batch)
  return batches
}

// A file's identity within an import: its folder plus its name. The server keeps
// the same pair (its `sanitize_path` only trims slashes, which our paths lack),
// so it is how we tell what already landed from what still has to go up.
const fileKey = (path: string, filename: string) => `${path}\u0000${filename}`

/// Upload one batch as one multipart request: a `path` field precedes each `file`,
/// so the folder tree is preserved (the server applies a `path` to every `file`
/// after it).
async function uploadBatch(
  importId: string,
  files: StagedFile[],
  onFraction: (fraction: number) => void,
): Promise<void> {
  const form = new FormData()
  for (const { file, path } of files) {
    form.append('path', path)
    form.append('file', file)
  }
  await uploadWithProgress<FileRecord[]>(`/api/imports/${importId}/files`, form, onFraction)
}

/// Upload a batch, retrying it on failure. A dropped connection can commit some of
/// a batch's files before it breaks, and the server has no unique key to dedupe
/// on — so before each retry we re-read what is already staged and re-send only
/// the files still missing. That makes a retry idempotent: a file that landed is
/// never uploaded twice, even if it was the *reply* that got lost.
async function uploadBatchResilient(
  importId: string,
  batch: StagedFile[],
  onFraction: (fraction: number) => void,
): Promise<void> {
  let pending = batch
  for (let attempt = 1; ; attempt++) {
    try {
      await uploadBatch(importId, pending, onFraction)
      return
    } catch (err) {
      if (attempt >= BATCH_ATTEMPTS) throw err
      await new Promise((resolve) => setTimeout(resolve, 500 * attempt))
      const staged = await api.importFiles(importId)
      const have = new Set(staged.map((f) => fileKey(f.path, f.filename)))
      pending = batch.filter((sf) => !have.has(fileKey(sf.path, sf.file.name)))
      if (!pending.length) return // the whole batch had landed; only the reply was lost
    }
  }
}

/// Stage a drop: create an import named after it, then upload every file into it
/// with its folder preserved, reporting progress (0..1) across the whole tree.
/// Files go up in bounded, individually-retried batches so a dropped connection
/// costs one batch, not the upload. A `.zip` unpacks in the background; the import
/// page waits for that, then offers model / bundle / existing bundle.
export async function startImport(
  drop: Drop,
  onProgress?: (fraction: number) => void,
): Promise<ImportSummary> {
  if (!drop.files.length) throw new Error('Nothing to import — that folder is empty')

  const staged = await api.createImport(drop.name)
  try {
    const report = onProgress ?? (() => {})
    const batches = planBatches(drop.files)
    // Progress is byte-weighted: files run from a few KB of metadata to hundreds
    // of MB of mesh, so counting files would lurch. Completed batches advance the
    // baseline; the in-flight batch fills its own slice.
    const totalBytes = drop.files.reduce((sum, f) => sum + f.file.size, 0) || 1
    let completedBytes = 0
    for (const batch of batches) {
      const batchBytes = batch.reduce((sum, f) => sum + f.file.size, 0)
      await uploadBatchResilient(staged.id, batch, (fraction) =>
        report((completedBytes + fraction * batchBytes) / totalBytes),
      )
      completedBytes += batchBytes
    }
    report(1)
  } catch (err) {
    // The import row exists before the bytes do; a failed upload would otherwise
    // strand an empty one in the Importing list, unpackable and uncommittable.
    await api.deleteImport(staged.id).catch(() => {})
    throw err
  }
  return staged
}
