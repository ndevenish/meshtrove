import { lazy, memo, Suspense, useMemo, useState } from 'react'
import {
  Box,
  Typography,
  Stack,
  Button,
  Accordion,
  AccordionSummary,
  AccordionDetails,
  Chip,
  IconButton,
  Tooltip,
  LinearProgress,
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  TextField,
  Autocomplete,
  Alert,
  Checkbox,
  Select,
  MenuItem,
  FormControlLabel,
  Radio,
  RadioGroup,
} from '@mui/material'
import ExpandMoreIcon from '@mui/icons-material/ExpandMore'
import AddIcon from '@mui/icons-material/Add'
import DownloadIcon from '@mui/icons-material/Download'
import UploadFileIcon from '@mui/icons-material/UploadFile'
import FolderIcon from '@mui/icons-material/Folder'
import FolderOpenIcon from '@mui/icons-material/FolderOpen'
import UnfoldMoreIcon from '@mui/icons-material/UnfoldMore'
import UnfoldLessIcon from '@mui/icons-material/UnfoldLess'
import CreateNewFolderIcon from '@mui/icons-material/CreateNewFolder'
import EditIcon from '@mui/icons-material/Edit'
import CheckIcon from '@mui/icons-material/Check'
import CloseIcon from '@mui/icons-material/Close'
import DeleteIcon from '@mui/icons-material/Delete'
import FolderDeleteIcon from '@mui/icons-material/FolderDelete'
import CallSplitIcon from '@mui/icons-material/CallSplit'
import DriveFileMoveIcon from '@mui/icons-material/DriveFileMove'
import PhotoCameraIcon from '@mui/icons-material/PhotoCamera'
import InsertDriveFileIcon from '@mui/icons-material/InsertDriveFile'
import UnarchiveIcon from '@mui/icons-material/Unarchive'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import ImageIcon from '@mui/icons-material/Image'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import {
  api,
  downloadUrl,
  formatBytes,
  variantLabel,
  type FileRecord,
  type ModelDetail,
  type VariantDetail,
} from '../api'
import { changeTags, pasteTags } from '../tags'

// three.js is heavy and only needed when a preview is actually opened, so split
// it out of the main bundle.
const StlPreviewDialog = lazy(() => import('./StlPreviewDialog'))
// Cheap by comparison (an `<img>`), but it only ever opens on a click, so it
// rides the same lazy split rather than sitting in the main bundle.
const ImagePreviewDialog = lazy(() => import('./ImagePreviewDialog'))

/// Raster formats a browser will draw in an `<img>`. SVG is in: an `<img>`
/// renders it inert, so an embedded script never runs.
const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'avif', 'svg']

const isImage = (filename: string) => {
  const ext = filename.toLowerCase().split('.').pop() ?? ''
  return IMAGE_EXTENSIONS.includes(ext)
}

/// What the archive chip says for each unpack state. `none` covers a staged
/// archive with no unpack job behind it at all — which used to be shown as
/// 'extracted', so a format the backend never opened looked dealt with. It
/// isn't an error state on its own: a MeshTrove export waits here for a
/// restore rather than being carved.
const UNPACK_CHIP = {
  pending: {
    label: 'extracting…',
    color: 'info',
    title: 'Waiting for the rest of the drop to be staged, then unpacking into this import.',
  },
  done: {
    label: 'extracted',
    color: 'default',
    title:
      'Already unpacked into this import. The archive is kept as a record of what was dropped, and is never carved into a model.',
  },
  failed: {
    label: 'extract failed',
    color: 'error',
    title:
      'The unpack job gave up — see the Jobs page for why. The archive is still here to download and open by hand.',
  },
  none: {
    label: 'not extracted',
    color: 'warning',
    title:
      'Nothing has unpacked this archive — most likely a format MeshTrove cannot open. Its contents are not staged in this import.',
  },
} as const

/// Wait for one job to settle. The render is the *job's* doing, so the picture is
/// there when the job says so — no inferring it from the shape of the queue.
/// Gives up after ~2 minutes and lets the caller refetch anyway; a render that
/// slow has bigger problems than a stale gallery.
async function waitForJob(jobId: number): Promise<void> {
  for (let i = 0; i < 120; i++) {
    const job = await api.job(jobId)
    if (job.status === 'succeeded' || job.status === 'failed' || job.status === 'cancelled') {
      return
    }
    await new Promise((resolve) => setTimeout(resolve, 1000))
  }
}

export default function VariantSection({
  model,
  canEdit,
  editing = false,
  onChange,
}: {
  model: ModelDetail
  canEdit: boolean
  /** Edit mode: deleting a file or a whole variant only offered here. */
  editing?: boolean
  onChange: () => void
}) {
  const [editingVariant, setEditingVariant] = useState<VariantDetail | 'new' | null>(null)

  return (
    <Box>
      <Stack direction="row" sx={{ alignItems: 'center', mb: 1 }}>
        <Typography variant="h6">Variants</Typography>
        <Box sx={{ flexGrow: 1 }} />
        {editing && (
          <Button startIcon={<AddIcon />} size="small" onClick={() => setEditingVariant('new')}>
            Add variant
          </Button>
        )}
      </Stack>
      {model.variants.length === 0 && (
        <Typography color="text.secondary" variant="body2">
          No variants yet{editing ? ' — add one to attach files' : ''}.
        </Typography>
      )}
      {model.variants.map((variant) => (
        <VariantRow
          key={variant.id}
          variant={variant}
          canEdit={canEdit}
          editing={editing}
          onChange={onChange}
          onEdit={() => setEditingVariant(variant)}
        />
      ))}
      <VariantEditDialog
        open={editingVariant !== null}
        variant={editingVariant === 'new' ? undefined : (editingVariant ?? undefined)}
        model={model}
        onClose={() => setEditingVariant(null)}
        onChange={onChange}
      />
    </Box>
  )
}

function VariantRow({
  variant,
  canEdit,
  editing,
  onChange,
  onEdit,
}: {
  variant: VariantDetail
  canEdit: boolean
  editing: boolean
  onChange: () => void
  onEdit: () => void
}) {
  const queryClient = useQueryClient()
  const [expanded, setExpanded] = useState(true)
  const [uploading, setUploading] = useState(false)
  const [rendering, setRendering] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const { data: files } = useQuery({
    queryKey: ['variant-files', variant.id],
    queryFn: () => api.variantFiles(variant.id),
    enabled: expanded,
  })

  const upload = async (fileList: FileList) => {
    setUploading(true)
    try {
      const form = new FormData()
      for (const file of fileList) form.append('file', file)
      await api.uploadVariantFiles(variant.id, form)
      await queryClient.invalidateQueries({ queryKey: ['variant-files', variant.id] })
      onChange()
    } finally {
      setUploading(false)
    }
  }

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      next.has(id) ? next.delete(id) : next.add(id)
      return next
    })

  const invalidate = async () => {
    await queryClient.invalidateQueries({ queryKey: ['variant-files', variant.id] })
    // The unsorted bucket and variant counts both shift when files move out.
    await queryClient.invalidateQueries({ queryKey: ['model-files', variant.model_id] })
    onChange()
  }

  // Move the selected files back to the model's unsorted bucket. A variant-owned
  // file resolves to model context on the backend, so `unsorted: true` returns it
  // to the model root (see routes/files.rs update_file).
  const moveToUnsorted = async () => {
    await Promise.all([...selected].map((id) => api.updateFile(id, { unsorted: true })))
    setSelected(new Set())
    await invalidate()
  }

  // Fold a folder's files into a new path (empty = strip the folder).
  const renameFolder = async (fileIds: string[], newPath: string) => {
    await Promise.all(fileIds.map((id) => api.updateFile(id, { path: newPath })))
    await queryClient.invalidateQueries({ queryKey: ['variant-files', variant.id] })
  }

  return (
    <Accordion
      variant="outlined"
      expanded={expanded}
      onChange={(_, next) => setExpanded(next)}
      disableGutters
    >
      <AccordionSummary expandIcon={<ExpandMoreIcon />}>
        <Stack direction="row" spacing={1.5} sx={{ alignItems: 'center', width: '100%', pr: 1 }}>
          {variant.name ? (
            <Typography sx={{ fontWeight: 600 }}>{variant.name}</Typography>
          ) : (
            variant.tags.length === 0 && (
              <Typography sx={{ fontWeight: 600, fontStyle: 'italic' }} color="text.secondary">
                Default
              </Typography>
            )
          )}
          {variant.tags.map((tag) => (
            <Chip key={tag} label={tag} size="small" variant="outlined" />
          ))}
          <Box sx={{ flexGrow: 1 }} />
          <Typography variant="body2" color="text.secondary">
            {variant.file_count} file{variant.file_count === 1 ? '' : 's'} ·{' '}
            {formatBytes(variant.total_size)}
          </Typography>
        </Stack>
      </AccordionSummary>
      <AccordionDetails>
        {variant.print_notes && (
          <Alert severity="info" icon={false} sx={{ mb: 1.5, whiteSpace: 'pre-wrap' }}>
            {variant.print_notes}
          </Alert>
        )}
        {(uploading || rendering) && <LinearProgress sx={{ mb: 1 }} />}
        {editing && files && files.length > 0 && (
          <Stack direction="row" spacing={1} sx={{ mb: 1, alignItems: 'center' }}>
            <Button
              size="small"
              startIcon={<DriveFileMoveIcon />}
              disabled={selected.size === 0}
              onClick={moveToUnsorted}
            >
              Move {selected.size || ''} to unsorted
            </Button>
            <Box sx={{ flexGrow: 1 }} />
            <Button
              size="small"
              onClick={() =>
                setSelected(
                  selected.size === files.length ? new Set() : new Set(files.map((f) => f.id)),
                )
              }
            >
              {selected.size === files.length ? 'Clear' : 'Select all'}
            </Button>
          </Stack>
        )}
        {expanded && files && (
          <FileTree
            files={files}
            selectable={editing}
            selected={selected}
            onToggle={toggle}
            onFolderRename={editing ? renameFolder : undefined}
            onDelete={
              editing
                ? async (fileId) => {
                    await api.deleteFile(fileId)
                    await invalidate()
                  }
                : undefined
            }
            onRender={
              canEdit
                ? async (fileId) => {
                    setRendering(true)
                    try {
                      // Wait for *this* job, then refetch. Inferring "my picture
                      // arrived" from a shared queue-watcher is how this broke
                      // twice: a render takes about a second, so whatever edge or
                      // poll the watcher is built on can be over before it looks.
                      // The job id is not ambiguous.
                      const { job_id } = await api.renderFile(fileId)
                      await waitForJob(job_id)
                    } finally {
                      setRendering(false)
                      await queryClient.invalidateQueries({ queryKey: ['model', variant.model_id] })
                      await queryClient.invalidateQueries({
                        queryKey: ['variant-files', variant.id],
                      })
                      await queryClient.invalidateQueries({ queryKey: ['jobs', 'all'] })
                    }
                  }
                : undefined
            }
          />
        )}
        {/* Uploading, retagging and deleting a variant all belong to edit mode;
            browsing a model is read-only. */}
        {editing && (
          <Stack direction="row" spacing={1} sx={{ mt: 1.5 }}>
            <Button component="label" size="small" startIcon={<UploadFileIcon />}>
              Upload files (.zip auto-unpacks)
              <input
                hidden
                multiple
                type="file"
                onChange={(e) => {
                  if (e.target.files?.length) void upload(e.target.files)
                  e.target.value = ''
                }}
              />
            </Button>
            <Button size="small" onClick={onEdit}>
              Edit
            </Button>
            {/* Deleting a variant takes its files with it. */}
            <Button
              size="small"
              color="error"
              onClick={async () => {
                if (confirm(`Delete variant "${variantLabel(variant)}" and its files?`)) {
                  await api.deleteVariant(variant.id)
                  onChange()
                }
              }}
            >
              Delete
            </Button>
          </Stack>
        )}
      </AccordionDetails>
    </Accordion>
  )
}

export const FILE_KINDS: FileRecord['kind'][] = [
  'model',
  'project',
  'raw',
  'document',
  'archive',
  'other',
]

/// Rebuild the kept folder structure from the flat path column. When the
/// optional editing props are supplied (used by the recategorisation UI), each
/// row gains a select checkbox, an inline kind selector, and a delete button.
/// Memoised: the import page mounts this with thousands of rows and re-renders
/// on every form keystroke, while the file list itself only changes when a
/// fetch lands.
export const FileTree = memo(function FileTree({
  files,
  selectable = false,
  selected,
  onToggle,
  onKindChange,
  onDelete,
  onRender,
  archivesExtracted,
  onFolderRename,
  onFolderDiscard,
  onFolderSplit,
}: {
  files: FileRecord[]
  selectable?: boolean
  selected?: Set<string>
  onToggle?: (id: string) => void
  onKindChange?: (id: string, kind: FileRecord['kind']) => void
  onDelete?: (id: string) => void
  /** Force a preview render from this file; it joins the model's images. */
  onRender?: (id: string) => void
  /** Mark `archive` rows as already unpacked — true inside an import, where a
      staged zip's contents are alongside it. */
  archivesExtracted?: boolean
  /** Rename (or, with an empty path, remove) a folder: rewrites the `path` of
      every file in the group. When set, folder headers become editable and the
      unfoldered root group gains an "Add folder" control. */
  onFolderRename?: (fileIds: string[], newPath: string) => void | Promise<void>
  /** Discard a folder outright — delete every file in the group. Distinct from
      `onFolderRename`'s empty-path "remove", which only flattens the folder away
      and keeps the files. Used on the import page to drop chaff before committing.
      When set, real folder headers gain a "Discard folder" control. */
  onFolderDiscard?: (fileIds: string[]) => void | Promise<void>
  /** Lift a folder and everything under it out into an import of its own — one
      drop is often several things. Takes the folder's path and the name for the
      new import; the folder itself becomes its top directory. */
  onFolderSplit?: (dir: string, name: string) => void | Promise<void>
}) {
  const [editingDir, setEditingDir] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [savingDir, setSavingDir] = useState(false)
  const [discardingDir, setDiscardingDir] = useState<string | null>(null)
  const [confirmDiscard, setConfirmDiscard] = useState<{
    dir: string
    entries: FileRecord[]
    /** Files in folders *under* this one — empty for a leaf. */
    nested: FileRecord[]
  } | null>(null)
  // Which of the two a discard means, for a folder that has folders under it.
  const [discardTree, setDiscardTree] = useState(true)
  const [confirmSplit, setConfirmSplit] = useState<{ dir: string; count: number } | null>(null)
  const [splitName, setSplitName] = useState('')
  const [splitting, setSplitting] = useState(false)
  const [previewFile, setPreviewFile] = useState<FileRecord | null>(null)
  // Folders the user has folded shut. Collapsing a folder takes everything
  // under it with it, so this holds only the folders clicked, not their
  // descendants — a descendant keeps whatever state it had for when its parent
  // opens again.
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())

  const startFolder = (dir: string) => {
    setEditingDir(dir)
    setDraft(dir === '/' ? '' : dir)
  }
  const cancelFolder = () => {
    setEditingDir(null)
    setDraft('')
  }
  const commitFolder = async (entries: FileRecord[]) => {
    if (!onFolderRename) return
    // Match the backend's sanitiser: trim surrounding slashes; empty = root.
    const next = draft.trim().replace(/^\/+|\/+$/g, '')
    setSavingDir(true)
    try {
      await onFolderRename(
        entries.map((f) => f.id),
        next,
      )
      cancelFolder()
    } finally {
      setSavingDir(false)
    }
  }
  // Strip the folder outright (files fall back to the root), no edit step.
  const removeFolder = async (entries: FileRecord[]) => {
    if (!onFolderRename) return
    setSavingDir(true)
    try {
      await onFolderRename(
        entries.map((f) => f.id),
        '',
      )
    } finally {
      setSavingDir(false)
    }
  }
  // Discard the folder: delete its files, not just its path. Unlike removeFolder,
  // nothing survives — the files never make it into the library.
  const discardFolder = async (dir: string, entries: FileRecord[]) => {
    if (!onFolderDiscard) return
    setDiscardingDir(dir)
    try {
      await onFolderDiscard(entries.map((f) => f.id))
      setConfirmDiscard(null)
    } finally {
      setDiscardingDir(null)
    }
  }

  // Split the folder out: its files change import, they aren't deleted, so this
  // is the one folder action that loses nothing.
  const splitFolder = async (dir: string) => {
    if (!onFolderSplit) return
    setSplitting(true)
    try {
      await onFolderSplit(dir, splitName.trim() || dir.split('/').pop() || dir)
      setConfirmSplit(null)
    } finally {
      setSplitting(false)
    }
  }

  const groups = useMemo(() => {
    const byDir = new Map<string, FileRecord[]>()
    for (const file of files) {
      const dir = file.path || '/'
      byDir.set(dir, [...(byDir.get(dir) ?? []), file])
    }
    // A header row exists for every path a file sits directly at; the folders
    // in between get one too, empty. Two reasons they can't be left out: a
    // folder action (split, discard) hangs off a header, so `Pack` would be
    // neither the moment every file in it lives in `Pack/supported`; and
    // without the row there is nothing for `Pack/supported` to be drawn under,
    // which is how it used to end up indented beneath an unrelated sibling.
    for (const dir of [...byDir.keys()]) {
      const parts = dir.split('/')
      for (let i = 1; i < parts.length; i++) {
        const ancestor = parts.slice(0, i).join('/')
        // The root group's key is '/', whose first "ancestor" is the empty
        // string — not a folder, and it used to be listed as one.
        if (ancestor && !byDir.has(ancestor)) byDir.set(ancestor, [])
      }
    }
    return [...byDir.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [files])

  /// Where a folder sits in the tree *as drawn*: how many of its ancestors have
  /// rows of their own, and what is left of its path once the deepest of them is
  /// stripped. Indenting by `path.split('/').length` instead would be a lie
  /// wherever an in-between folder holds no files directly and so was never
  /// synthesised (folder actions off): `Legs/Adjustable` would indent two levels
  /// under an `Instructions` it has nothing to do with, because that is simply
  /// the row above it.
  const placement = useMemo(() => {
    const shown = new Set(groups.map(([dir]) => dir))
    const map = new Map<string, { depth: number; label: string }>()
    for (const [dir] of groups) {
      if (dir === '/') {
        map.set(dir, { depth: 0, label: dir })
        continue
      }
      const parts = dir.split('/')
      let depth = 0
      let deepest = ''
      for (let i = 1; i < parts.length; i++) {
        const ancestor = parts.slice(0, i).join('/')
        if (shown.has(ancestor)) {
          depth++
          deepest = ancestor
        }
      }
      map.set(dir, { depth, label: deepest ? dir.slice(deepest.length + 1) : dir })
    }
    return map
  }, [groups])

  /// The files sitting in folders *under* `dir`. A folder here is a shared
  /// `path` string and each group holds only what sits directly at that path, so
  /// discarding `Pack` would leave `Pack/supported` behind, orphaned under a
  /// folder that no longer exists — hence the choice offered when this is
  /// non-empty.
  const under = (dir: string) =>
    groups.filter(([d]) => d.startsWith(`${dir}/`)).flatMap(([, entries]) => entries)

  /// Every folder in the tree, and how many files sit anywhere beneath it —
  /// what a collapsed folder reports it is hiding. Counted by walking each
  /// file's path once and crediting all of its ancestors, so the whole map
  /// costs one pass rather than a scan per folder.
  const subtreeCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const file of files) {
      const dir = file.path || '/'
      if (dir === '/') continue
      const parts = dir.split('/')
      for (let i = 1; i <= parts.length; i++) {
        const ancestor = parts.slice(0, i).join('/')
        counts.set(ancestor, (counts.get(ancestor) ?? 0) + 1)
      }
    }
    return counts
  }, [files])

  const dirs = useMemo(() => groups.map(([dir]) => dir).filter((dir) => dir !== '/'), [groups])
  // A folder is folded away if any ancestor is collapsed. The root group is in
  // no folder, so nothing can fold it away.
  const hidden = (dir: string) => {
    if (dir === '/') return false
    const parts = dir.split('/')
    for (let i = 1; i < parts.length; i++) {
      if (collapsed.has(parts.slice(0, i).join('/'))) return true
    }
    return false
  }
  /// Fold a folder shut or open it again. `deep` (alt/option-click, as in
  /// Finder and VS Code) puts everything under it into the same state, so
  /// opening a folder that way reveals the whole subtree rather than whatever
  /// each descendant was left at.
  const toggleDir = (dir: string, deep: boolean) => {
    setCollapsed((prev) => {
      const next = new Set(prev)
      const collapsing = !prev.has(dir)
      const apply = (d: string) => (collapsing ? next.add(d) : next.delete(d))
      apply(dir)
      if (deep) for (const d of dirs) if (d.startsWith(`${dir}/`)) apply(d)
      return next
    })
  }

  // Reserve the preview column on every row once any file can show it, so the
  // download/render icons stay in aligned columns next to files (projects,
  // documents) that can't be previewed. Nothing previewable → no wasted column.
  const anyPreviewable = useMemo(
    () => files.some((f) => f.filename.toLowerCase().endsWith('.stl') || isImage(f.filename)),
    [files],
  )

  if (files.length === 0)
    return (
      <Typography variant="body2" color="text.secondary">
        No files yet.
      </Typography>
    )

  return (
    <Box>
      {/* Only earns its space once there is a structure to navigate. */}
      {dirs.length > 1 && (
        <Stack direction="row" spacing={0.5} sx={{ alignItems: 'center', mb: 0.5 }}>
          <Button
            size="small"
            startIcon={<UnfoldMoreIcon sx={{ fontSize: 16 }} />}
            onClick={() => setCollapsed(new Set())}
            disabled={collapsed.size === 0}
            sx={{ textTransform: 'none' }}
          >
            Expand all
          </Button>
          <Button
            size="small"
            startIcon={<UnfoldLessIcon sx={{ fontSize: 16 }} />}
            onClick={() => setCollapsed(new Set(dirs))}
            disabled={collapsed.size === dirs.length}
            sx={{ textTransform: 'none' }}
          >
            Collapse all
          </Button>
        </Stack>
      )}
      {groups.map(([dir, entries]) => {
        if (hidden(dir)) return null
        // Nesting is shown by indent, so the header carries only what its
        // indent doesn't already say: the folder's own name under a parent that
        // has a row, the whole path when nothing above it does.
        const { depth, label } = placement.get(dir) ?? { depth: 0, label: dir }
        const shut = collapsed.has(dir)
        return (
          // Indent by depth, capped: past a few levels the rows would run out
          // of width in the import page's side column and the file names, which
          // are the point, would go first.
          <Box key={dir} sx={{ mb: 1, ml: Math.min(depth, 6) * 2 }}>
            {(dir !== '/' || !!onFolderRename) && (
              <Stack
                direction="row"
                spacing={0.75}
                sx={{ alignItems: 'center', mb: 0.25, minHeight: 30 }}
              >
                {dir !== '/' && (
                  <Tooltip
                    title={`${shut ? 'Expand' : 'Collapse'} folder — alt-click for everything below it`}
                  >
                    <IconButton
                      size="small"
                      aria-label={`${shut ? 'Expand' : 'Collapse'} ${dir}`}
                      aria-expanded={!shut}
                      sx={{ p: 0.25 }}
                      onClick={(e) => toggleDir(dir, e.altKey)}
                    >
                      {shut ? (
                        <FolderIcon sx={{ fontSize: 18, opacity: 0.6 }} />
                      ) : (
                        <FolderOpenIcon sx={{ fontSize: 18, opacity: 0.6 }} />
                      )}
                    </IconButton>
                  </Tooltip>
                )}
                {editingDir === dir ? (
                  <>
                    <TextField
                      size="small"
                      variant="standard"
                      autoFocus
                      value={draft}
                      disabled={savingDir}
                      placeholder="(no folder — leave empty for root)"
                      onChange={(e) => setDraft(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') void commitFolder(entries)
                        if (e.key === 'Escape') cancelFolder()
                      }}
                      sx={{ maxWidth: 340, flexGrow: 1 }}
                    />
                    <Tooltip title="Save">
                      <span>
                        <IconButton
                          size="small"
                          disabled={savingDir}
                          onClick={() => void commitFolder(entries)}
                        >
                          <CheckIcon sx={{ fontSize: 18 }} />
                        </IconButton>
                      </span>
                    </Tooltip>
                    <Tooltip title="Cancel">
                      <span>
                        <IconButton size="small" disabled={savingDir} onClick={cancelFolder}>
                          <CloseIcon sx={{ fontSize: 18 }} />
                        </IconButton>
                      </span>
                    </Tooltip>
                  </>
                ) : dir === '/' ? (
                  <Button
                    size="small"
                    startIcon={<CreateNewFolderIcon sx={{ fontSize: 18 }} />}
                    onClick={() => startFolder('/')}
                    sx={{ textTransform: 'none' }}
                  >
                    Add folder
                  </Button>
                ) : (
                  <>
                    <Typography
                      variant="body2"
                      sx={{ fontWeight: 600, cursor: 'pointer' }}
                      onClick={(e) => toggleDir(dir, e.altKey)}
                    >
                      {label}
                    </Typography>
                    {/* What folding it away is hiding — the whole subtree, since
                      that is what went with it. */}
                    {shut && (
                      <Typography variant="caption" color="text.secondary">
                        {subtreeCounts.get(dir) ?? 0} file
                        {(subtreeCounts.get(dir) ?? 0) === 1 ? '' : 's'}
                      </Typography>
                    )}
                    {/* Rename and remove rewrite the paths of the files in the
                      group, so they have nothing to do on a folder that only
                      holds other folders. */}
                    {onFolderRename && entries.length > 0 && (
                      <>
                        <Tooltip title="Remove folder">
                          <span>
                            <IconButton
                              size="small"
                              disabled={savingDir}
                              onClick={() => void removeFolder(entries)}
                            >
                              <CloseIcon sx={{ fontSize: 15 }} />
                            </IconButton>
                          </span>
                        </Tooltip>
                        <Tooltip title="Rename folder">
                          <IconButton size="small" onClick={() => startFolder(dir)}>
                            <EditIcon sx={{ fontSize: 15 }} />
                          </IconButton>
                        </Tooltip>
                      </>
                    )}
                    {onFolderSplit && (
                      <Tooltip title="Split this folder and everything under it into a separate import">
                        <span>
                          <IconButton
                            size="small"
                            onClick={() => {
                              setSplitName(dir.split('/').pop() ?? dir)
                              setConfirmSplit({
                                dir,
                                count: entries.length + under(dir).length,
                              })
                            }}
                          >
                            <CallSplitIcon sx={{ fontSize: 16 }} />
                          </IconButton>
                        </span>
                      </Tooltip>
                    )}
                    {onFolderDiscard && (
                      <Tooltip title="Discard folder — delete its files without importing them">
                        <span>
                          <IconButton
                            size="small"
                            color="error"
                            disabled={discardingDir === dir}
                            onClick={() => {
                              setDiscardTree(true)
                              setConfirmDiscard({ dir, entries, nested: under(dir) })
                            }}
                          >
                            <FolderDeleteIcon sx={{ fontSize: 17 }} />
                          </IconButton>
                        </span>
                      </Tooltip>
                    )}
                  </>
                )}
              </Stack>
            )}
            {!shut &&
              entries.map((file) => (
                <Stack
                  key={file.id}
                  direction="row"
                  spacing={1}
                  sx={{ alignItems: 'center', pl: dir !== '/' ? 3 : 0, py: 0.25 }}
                >
                  {selectable && (
                    <Checkbox
                      size="small"
                      sx={{ p: 0.25 }}
                      checked={selected?.has(file.id) ?? false}
                      onChange={() => onToggle?.(file.id)}
                    />
                  )}
                  <InsertDriveFileIcon sx={{ fontSize: 16, opacity: 0.5 }} />
                  <Typography variant="body2" sx={{ flexGrow: 1 }} noWrap>
                    {file.filename}
                  </Typography>
                  {onKindChange ? (
                    <Select
                      size="small"
                      variant="standard"
                      value={file.kind}
                      onChange={(e) => onKindChange(file.id, e.target.value as FileRecord['kind'])}
                      sx={{ minWidth: 96, fontSize: 13 }}
                    >
                      {FILE_KINDS.map((k) => (
                        <MenuItem key={k} value={k} sx={{ fontSize: 13 }}>
                          {k}
                        </MenuItem>
                      ))}
                    </Select>
                  ) : (
                    <Chip label={file.kind} size="small" variant="outlined" sx={{ height: 20 }} />
                  )}
                  <Typography variant="caption" color="text.secondary" sx={{ width: 64 }}>
                    {formatBytes(file.size)}
                  </Typography>
                  {/* STL is the one format we can render live in the browser
                  (three.js). Give it a viewer; other model formats fall back to
                  the server-rendered picture. Images that came in with the
                  fileset — renders, references — share the column: they were
                  never promoted into the gallery, so this is the only way to
                  look at one short of downloading it. The slot is held open for
                  rows that are neither, so the download/render icons line up
                  down the list. */}
                  {anyPreviewable && (
                    <Box sx={{ width: 30, flexShrink: 0 }}>
                      {file.filename.toLowerCase().endsWith('.stl') ? (
                        <Tooltip title="Preview 3D model">
                          <IconButton size="small" onClick={() => setPreviewFile(file)}>
                            <ViewInArIcon sx={{ fontSize: 18 }} />
                          </IconButton>
                        </Tooltip>
                      ) : (
                        isImage(file.filename) && (
                          <Tooltip title="Preview image">
                            <IconButton size="small" onClick={() => setPreviewFile(file)}>
                              <ImageIcon sx={{ fontSize: 18 }} />
                            </IconButton>
                          </Tooltip>
                        )
                      )}
                    </Box>
                  )}
                  <Tooltip title="Download">
                    <IconButton size="small" component="a" href={downloadUrl(file.id)}>
                      <DownloadIcon sx={{ fontSize: 18 }} />
                    </IconButton>
                  </Tooltip>
                  {/* The carve renders one picture per variant and picks the file
                  itself. This is the override for when it picked the base plate:
                  render *this* one, and it joins the model's images. */}
                  {onRender && (file.kind === 'model' || file.kind === 'project') && (
                    <Tooltip title="Render a preview from this file">
                      <IconButton size="small" onClick={() => onRender(file.id)}>
                        <PhotoCameraIcon sx={{ fontSize: 18 }} />
                      </IconButton>
                    </Tooltip>
                  )}
                  {/* Say where the archive has got to, or it reads as one more thing
                  waiting to be dealt with. Once unpacked it is kept only as the
                  record of what was dropped, and is never carved. A `null`
                  unpack means no job ever ran for it: the chip says so rather
                  than passing it off as extracted. */}
                  {archivesExtracted && file.kind === 'archive' && (
                    <Tooltip title={UNPACK_CHIP[file.unpack ?? 'none'].title}>
                      <Chip
                        icon={<UnarchiveIcon sx={{ fontSize: 14 }} />}
                        label={UNPACK_CHIP[file.unpack ?? 'none'].label}
                        size="small"
                        variant="outlined"
                        color={UNPACK_CHIP[file.unpack ?? 'none'].color}
                        sx={{ height: 20, opacity: file.unpack === 'done' ? 0.7 : 1 }}
                      />
                    </Tooltip>
                  )}
                  {onDelete && (
                    <Tooltip title="Delete file">
                      <IconButton size="small" color="error" onClick={() => onDelete(file.id)}>
                        <DeleteIcon sx={{ fontSize: 18 }} />
                      </IconButton>
                    </Tooltip>
                  )}
                </Stack>
              ))}
          </Box>
        )
      })}
      {previewFile && (
        <Suspense fallback={null}>
          {isImage(previewFile.filename) ? (
            <ImagePreviewDialog
              open
              fileId={previewFile.id}
              filename={previewFile.filename}
              onClose={() => setPreviewFile(null)}
            />
          ) : (
            <StlPreviewDialog
              open
              fileId={previewFile.id}
              filename={previewFile.filename}
              size={previewFile.size}
              onClose={() => setPreviewFile(null)}
            />
          )}
        </Suspense>
      )}
      <Dialog
        open={!!confirmDiscard}
        onClose={() => discardingDir === null && setConfirmDiscard(null)}
      >
        <DialogTitle>Discard folder?</DialogTitle>
        <DialogContent>
          {confirmDiscard &&
          confirmDiscard.nested.length > 0 &&
          confirmDiscard.entries.length > 0 ? (
            // A folder with folders under it: "delete this folder" is two
            // different things, and which one was meant is not ours to guess.
            // The whole subtree goes by default — that is what deleting a folder
            // means everywhere else — but the counts are spelled out either way.
            <>
              <Typography variant="body2" sx={{ mb: 1 }}>
                <strong>{confirmDiscard.dir}</strong> has folders under it. Nothing here is
                imported, and this can't be undone.
              </Typography>
              <RadioGroup
                value={discardTree ? 'tree' : 'folder'}
                onChange={(e) => setDiscardTree(e.target.value === 'tree')}
              >
                <FormControlLabel
                  value="tree"
                  control={<Radio size="small" />}
                  disabled={discardingDir !== null}
                  label={
                    <Typography variant="body2">
                      This folder and everything under it —{' '}
                      {confirmDiscard.entries.length + confirmDiscard.nested.length} files
                    </Typography>
                  }
                />
                <FormControlLabel
                  value="folder"
                  control={<Radio size="small" />}
                  disabled={discardingDir !== null}
                  label={
                    <Typography variant="body2">
                      This folder only — {confirmDiscard.entries.length}{' '}
                      {confirmDiscard.entries.length === 1 ? 'file' : 'files'}, leaving the{' '}
                      {confirmDiscard.nested.length} below it staged
                    </Typography>
                  }
                />
              </RadioGroup>
            </>
          ) : (
            // Either a leaf folder, or one that holds nothing but folders — in
            // both cases there is only one thing "delete this folder" can mean.
            (() => {
              const count = confirmDiscard
                ? confirmDiscard.entries.length + confirmDiscard.nested.length
                : 0
              return (
                <Typography variant="body2">
                  Delete the {count} {count === 1 ? 'file' : 'files'} in{' '}
                  <strong>{confirmDiscard?.dir}</strong> without importing{' '}
                  {count === 1 ? 'it' : 'them'}. This can't be undone.
                </Typography>
              )
            })()
          )}
        </DialogContent>
        <DialogActions>
          <Button disabled={discardingDir !== null} onClick={() => setConfirmDiscard(null)}>
            Cancel
          </Button>
          <Button
            color="error"
            variant="contained"
            disabled={discardingDir !== null}
            data-testid="confirm-discard"
            onClick={() =>
              confirmDiscard &&
              void discardFolder(
                confirmDiscard.dir,
                discardTree || confirmDiscard.entries.length === 0
                  ? [...confirmDiscard.entries, ...confirmDiscard.nested]
                  : confirmDiscard.entries,
              )
            }
          >
            Discard
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={!!confirmSplit}
        fullWidth
        maxWidth="xs"
        onClose={() => !splitting && setConfirmSplit(null)}
      >
        <DialogTitle>Split into a new import?</DialogTitle>
        <DialogContent>
          <Typography variant="body2" sx={{ mb: 2 }}>
            Move <strong>{confirmSplit?.dir}</strong> and everything under it —{' '}
            {confirmSplit?.count} {confirmSplit?.count === 1 ? 'file' : 'files'} — to an import of
            its own. Nothing is deleted, and the folders are kept, with{' '}
            <strong>{confirmSplit?.dir.split('/').pop()}</strong> as the top one.
          </Typography>
          <TextField
            fullWidth
            size="small"
            autoFocus
            label="Name the new import"
            value={splitName}
            disabled={splitting}
            onChange={(e) => setSplitName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && confirmSplit) void splitFolder(confirmSplit.dir)
            }}
          />
        </DialogContent>
        <DialogActions>
          <Button disabled={splitting} onClick={() => setConfirmSplit(null)}>
            Cancel
          </Button>
          <Button
            variant="contained"
            disabled={splitting}
            data-testid="confirm-split"
            onClick={() => confirmSplit && void splitFolder(confirmSplit.dir)}
          >
            Split
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  )
})

/// A variant is its tag set, so the tags are the substantive field here and the
/// name is just a label. Leaving the tags empty is legitimate — it addresses the
/// model's one anonymous variant.
function VariantEditDialog({
  open,
  variant,
  model,
  onClose,
  onChange,
}: {
  open: boolean
  variant?: VariantDetail
  model: ModelDetail
  onClose: () => void
  onChange: () => void
}) {
  const { data: vocabulary } = useQuery({
    queryKey: ['variant-tags'],
    queryFn: () => api.variantTags(),
  })
  const [name, setName] = useState(variant?.name ?? '')
  const [tags, setTags] = useState<string[]>(variant?.tags ?? [])
  const [notes, setNotes] = useState(variant?.print_notes ?? '')
  const [error, setError] = useState('')

  // Reset when target changes (dialog reused between add/edit)
  const [lastKey, setLastKey] = useState<string | null>(null)
  const key = variant?.id ?? 'new'
  if (open && key !== lastKey) {
    setLastKey(key)
    setName(variant?.name ?? '')
    setTags(variant?.tags ?? [])
    setNotes(variant?.print_notes ?? '')
    setError('')
  }

  // Same tags = same variant, so saving onto another variant's tag set folds
  // this one into it. Say so before it happens rather than after.
  const sameSet = (a: string[], b: string[]) =>
    a.length === b.length && [...a].sort().join('\u0000') === [...b].sort().join('\u0000')
  const collision = model.variants.find((v) => v.id !== variant?.id && sameSet(v.tags, tags))

  const submit = async () => {
    try {
      const body = { name: name.trim() || null, tags, print_notes: notes || null }
      if (variant) await api.updateVariant(variant.id, body)
      else await api.createVariant(model.id, body)
      onChange()
      onClose()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>{variant ? 'Edit variant' : 'Add variant'}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          {collision && (
            <Alert severity="warning">
              {`These tags already identify "${variantLabel(collision)}"`}
              {variant
                ? '. Saving merges this variant into it, moving its files across.'
                : '. Its files will be added to that variant.'}
            </Alert>
          )}
          <Autocomplete
            multiple
            freeSolo
            autoFocus
            options={(vocabulary ?? []).map((t) => t.name)}
            value={tags}
            onChange={changeTags(setTags)}
            renderInput={(params) => (
              <TextField
                {...params}
                onPaste={pasteTags(tags, setTags)}
                label="Variant tags"
                placeholder="32mm, supported, lychee…"
                helperText={
                  tags.length
                    ? 'These tags identify the variant. New ones are created as you type.'
                    : "No tags — this is the model's single untagged variant."
                }
              />
            )}
          />
          <TextField
            label="Name (optional)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. My merged remix"
            helperText="A display label only; the tags above are what separate variants."
          />
          <TextField
            label="Print notes"
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            multiline
            minRows={3}
            placeholder="Resin, exposure, orientation, supports…"
          />
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" onClick={submit}>
          {collision ? 'Merge' : variant ? 'Save' : 'Add'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
