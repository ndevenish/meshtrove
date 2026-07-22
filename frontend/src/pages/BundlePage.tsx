import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router-dom'
import {
  Autocomplete,
  Container,
  Box,
  Typography,
  Chip,
  Stack,
  Button,
  Paper,
  IconButton,
  TextField,
  Tooltip,
  Checkbox,
  Divider,
  Tab,
  Tabs,
  Snackbar,
  Alert,
  alpha,
} from '@mui/material'
import EditIcon from '@mui/icons-material/Edit'
import StarIcon from '@mui/icons-material/Star'
import StarBorderIcon from '@mui/icons-material/StarBorder'
import DeleteIcon from '@mui/icons-material/Delete'
import AddPhotoAlternateIcon from '@mui/icons-material/AddPhotoAlternate'
import RemoveCircleIcon from '@mui/icons-material/RemoveCircle'
import DragHandleIcon from '@mui/icons-material/DragHandle'
import CloseIcon from '@mui/icons-material/Close'
import CallSplitIcon from '@mui/icons-material/CallSplit'
import MergeIcon from '@mui/icons-material/Merge'
import ReactMarkdown from 'react-markdown'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import DownloadIcon from '@mui/icons-material/Download'
import SellIcon from '@mui/icons-material/Sell'
import { api, imageUrl, sourceOrigin, type BundleDetail } from '../api'
import ExportDialog from '../components/ExportDialog'
import { useAuth } from '../main'
import { usePasteImage, useDropImage } from '../imageGestures'
import { useSuppressGlobalDrop } from '../globalDrop'
import { startImport } from '../upload'
import Dropzone from '../components/Dropzone'
import ModelCard from '../components/ModelCard'
import { CustomFieldReadout } from '../components/CustomFieldControl'
import BundleDetailsEditor from '../components/BundleDetailsEditor'
import { type DetailsEditorHandle } from '../components/ModelDetailsEditor'
import BundleUnsortedSection from '../components/BundleUnsortedSection'
import DescriptionHistoryDialog from '../components/DescriptionHistoryDialog'
import ImportErrorDialog from '../components/ImportErrorDialog'
import BundlePatchDialog from '../components/BundlePatchDialog'
import BundleRetagDialog from '../components/BundleRetagDialog'
import BundleDeleteDialog from '../components/BundleDeleteDialog'
import BundleSplitDialog from '../components/BundleSplitDialog'
import BundleMergeDialog from '../components/BundleMergeDialog'

export default function BundlePage() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const { user } = useAuth()
  const queryClient = useQueryClient()
  // Edit *mode*: the fields become editable in place, and the destructive
  // controls — remove a model from the bundle, delete a file, delete an image —
  // appear only here.
  const [editing, setEditing] = useState(false)
  // Save and Cancel take the Edit button's place in the header.
  const editorRef = useRef<DetailsEditorHandle>(null)
  const [saving, setSaving] = useState(false)
  const [exportOpen, setExportOpen] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [patchOpen, setPatchOpen] = useState(false)
  const [retagOpen, setRetagOpen] = useState(false)
  const [mergeOpen, setMergeOpen] = useState(false)
  // The zip dropped on the inline importer box, handed to the dialog to preview.
  const [patchFile, setPatchFile] = useState<File | null>(null)
  // A patch was applied and its post-apply reload is owed. We hold it until the
  // dialog closes: that reload navigates to the UUID, which blanks `bundle`
  // mid-fetch — and a blank page (return null below) unmounts the still-open
  // dialog, which then remounts and re-previews the same zip, popping itself
  // back up. Deferring to close keeps the navigation out of the open dialog's way.
  const patchReloadOwed = useRef(false)
  // Upload progress (0..1) for the "merge files into this bundle" drop box, while
  // it stages the drop before jumping to the import page.
  const [mergePct, setMergePct] = useState<number | null>(null)
  // Edit mode shows the inline patch drop box, so the app-wide overlay must stand
  // aside while editing or it swallows the zip meant for that box.
  useSuppressGlobalDrop(editing)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [selectedImage, setSelectedImage] = useState<string | null>(null)
  const [uploadError, setUploadError] = useState('')
  const [toast, setToast] = useState('')

  const { data: bundle } = useQuery({
    queryKey: ['bundle', id],
    queryFn: () => api.bundle(id!),
    enabled: !!id,
  })

  // Canonical URL is the slug; arriving by UUID or a stale slug redirects here
  // (see ModelPage). Seed the slug's cache so the swap doesn't flash a reload.
  useEffect(() => {
    if (bundle && id !== bundle.slug) {
      queryClient.setQueryData(['bundle', bundle.slug], bundle)
      navigate(`/bundles/${bundle.slug}`, { replace: true })
    }
  }, [bundle, id, navigate, queryClient])

  // One BundlePage serves every bundle, so a navigation between two of them
  // keeps this component (and its state) mounted. Edit mode is about the bundle
  // you were editing — landing on a different one still in it, as a split does
  // when it jumps to the bundle it just made, is a mode nobody asked for.
  useEffect(() => setEditing(false), [bundle?.id])

  const canEditBundle =
    !!bundle &&
    !!user &&
    (user.role === 'admin' || (user.role === 'editor' && user.id === bundle.created_by))
  // By UUID, not the slug in `id`: the image routes parse their path segment as
  // a Uuid and reject a slug before the handler runs (see ModelPage).
  const bundleId = bundle?.id ?? ''
  const imageAdded = useCallback(
    (how: string) => {
      void queryClient.invalidateQueries({ queryKey: ['bundle', id] })
      setToast(`Image added ${how}`)
    },
    [queryClient, id],
  )
  usePasteImage(canEditBundle, 'bundles', bundleId, {
    onUploaded: () => imageAdded('from clipboard'),
    onError: (m) => setToast(`Paste failed: ${m}`),
  })
  const droppingImage = useDropImage(canEditBundle, 'bundles', bundleId, {
    onUploaded: () => imageAdded('to this bundle'),
    onError: (m) => setToast(`Image upload failed: ${m}`),
  })

  if (!bundle) return null
  const canEdit =
    user && (user.role === 'admin' || (user.role === 'editor' && user.id === bundle.created_by))
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['bundle', id] })

  const shownImage = selectedImage ?? bundle.images[0]?.id ?? null

  const uploadImage = async (file: File) => {
    const form = new FormData()
    form.append('file', file)
    await api.uploadImage('bundles', bundle.id, form)
    refresh()
  }

  return (
    <Container maxWidth="lg" sx={{ py: 3 }}>
      <Stack direction={{ xs: 'column', md: 'row' }} spacing={3}>
        {/* Gallery */}
        <Box sx={{ width: { md: 460 }, flexShrink: 0 }}>
          <Paper
            variant="outlined"
            sx={(theme) => ({
              aspectRatio: '1',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              overflow: 'hidden',
              position: 'relative',
              // Caught page-wide (imageGestures.ts); this is only where the page
              // says where the picture is going to land.
              ...(droppingImage && {
                borderColor: theme.palette.primary.main,
                borderStyle: 'dashed',
                borderWidth: 2,
              }),
            })}
          >
            {droppingImage && (
              <Stack
                sx={(theme) => ({
                  position: 'absolute',
                  inset: 0,
                  zIndex: 1,
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: 1,
                  pointerEvents: 'none',
                  backgroundColor: alpha(theme.palette.background.paper, 0.9),
                })}
              >
                <AddPhotoAlternateIcon sx={{ fontSize: 48, color: 'primary.main' }} />
                <Typography sx={{ fontWeight: 600 }}>Drop to add image</Typography>
              </Stack>
            )}
            {shownImage ? (
              <Box
                component="img"
                src={imageUrl(shownImage)}
                alt={bundle.name}
                sx={{ width: '100%', height: '100%', objectFit: 'contain' }}
              />
            ) : (
              <Box sx={{ textAlign: 'center', px: 2 }}>
                {/* A Box wrapping two Typographies, not a Typography wrapping
                    one: both render a <p>, and a <p> inside a <p> is invalid
                    HTML that React refuses to hydrate. */}
                <Typography color="text.secondary">No images yet</Typography>
                {canEdit && (
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    Drop one on the page, or paste (⌘V)
                  </Typography>
                )}
              </Box>
            )}
          </Paper>
          <Stack direction="row" spacing={1} sx={{ mt: 1, flexWrap: 'wrap', gap: 1 }}>
            {bundle.images.map((image) => (
              <Box key={image.id} sx={{ position: 'relative' }}>
                <Box
                  component="img"
                  src={imageUrl(image.id)}
                  onClick={() => setSelectedImage(image.id)}
                  sx={{
                    width: 72,
                    height: 72,
                    objectFit: 'cover',
                    borderRadius: 1,
                    cursor: 'pointer',
                    border: (theme) =>
                      `2px solid ${shownImage === image.id ? theme.palette.primary.main : 'transparent'}`,
                  }}
                />
                {canEdit && (
                  <Stack direction="row" sx={{ position: 'absolute', top: -6, right: -6 }}>
                    <Tooltip title={image.is_primary ? 'Primary image' : 'Make primary'}>
                      <IconButton
                        size="small"
                        sx={{ p: 0.25, bgcolor: 'background.paper' }}
                        onClick={async () => {
                          if (!image.is_primary) {
                            await api.markPrimary(image.id)
                            refresh()
                          }
                        }}
                      >
                        {image.is_primary ? (
                          <StarIcon sx={{ fontSize: 16, color: 'primary.main' }} />
                        ) : (
                          <StarBorderIcon sx={{ fontSize: 16 }} />
                        )}
                      </IconButton>
                    </Tooltip>
                    {/* Picking the favourite is safe; deleting the picture waits
                        for edit mode. */}
                    {editing && (
                      <Tooltip title="Delete image">
                        <IconButton
                          size="small"
                          sx={{ p: 0.25, bgcolor: 'background.paper' }}
                          onClick={async () => {
                            await api.deleteImage(image.id)
                            setSelectedImage(null)
                            refresh()
                          }}
                        >
                          <DeleteIcon sx={{ fontSize: 16 }} />
                        </IconButton>
                      </Tooltip>
                    )}
                  </Stack>
                )}
              </Box>
            ))}
            {/* As on a model: adding a picture is an edit, so the tile waits for
                edit mode. Drop and paste are page-wide and unaffected. */}
            {canEdit && editing && (
              <Button
                component="label"
                variant="outlined"
                sx={{ width: 72, height: 72, minWidth: 0 }}
              >
                <AddPhotoAlternateIcon />
                <input
                  hidden
                  type="file"
                  accept="image/*"
                  onChange={(e) => {
                    const file = e.target.files?.[0]
                    if (file) void uploadImage(file)
                    e.target.value = ''
                  }}
                />
              </Button>
            )}
          </Stack>
        </Box>

        {/* Details. A flex column so the unsorted files can be pushed to its
            foot — the row stretches both columns to the taller of the two, which
            is normally the gallery, and that slack is what the gap eats. */}
        <Box sx={{ flexGrow: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
          <Stack sx={{ alignItems: 'flex-start' }} direction="row" spacing={1}>
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center', flexGrow: 1 }}>
              <Typography variant="h4" sx={{ fontWeight: 700 }}>
                {bundle.name}
              </Typography>
            </Stack>
            {canEdit && !editing && (
              <>
                <Button startIcon={<DownloadIcon />} onClick={() => setExportOpen(true)}>
                  Export
                </Button>
                <Button startIcon={<EditIcon />} onClick={() => setEditing(true)}>
                  Edit
                </Button>
              </>
            )}
            {canEdit && editing && (
              <Stack direction="row" spacing={1} sx={{ flexShrink: 0 }}>
                {/* Delete leads, kept clear of Save/Cancel: it sits at the far
                    end from where a double-click on the primary action would
                    land, so a stray second click can't fall on it. */}
                <Button
                  color="error"
                  startIcon={<DeleteIcon />}
                  disabled={saving}
                  onClick={() => setDeleteOpen(true)}
                  sx={{ whiteSpace: 'nowrap' }}
                >
                  Delete bundle
                </Button>
                <Button
                  startIcon={<MergeIcon />}
                  disabled={saving}
                  onClick={() => setMergeOpen(true)}
                  sx={{ whiteSpace: 'nowrap' }}
                >
                  Merge in…
                </Button>
                <Button
                  startIcon={<SellIcon />}
                  disabled={saving || bundle.models.length === 0}
                  onClick={() => setRetagOpen(true)}
                  sx={{ whiteSpace: 'nowrap' }}
                >
                  Tag all models
                </Button>
                <Button
                  variant="contained"
                  disabled={saving}
                  onClick={() => {
                    void editorRef.current?.save().catch(() => {
                      // The editor reports why; stay in edit mode rather than
                      // discarding what was typed.
                    })
                  }}
                >
                  Save
                </Button>
                <Button disabled={saving} onClick={() => setEditing(false)}>
                  Cancel
                </Button>
              </Stack>
            )}
          </Stack>

          {editing && (
            <>
              <BundleDetailsEditor
                key={bundle.id}
                ref={editorRef}
                bundle={bundle}
                onDone={() => setEditing(false)}
                onBusyChange={setSaving}
              />
              <Stack direction={{ xs: 'column', sm: 'row' }} spacing={2} sx={{ mb: 2 }}>
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Dropzone
                    label={
                      mergePct === null
                        ? 'Merge files into this bundle'
                        : mergePct < 100
                          ? `Uploading ${mergePct}%…`
                          : 'Staging…'
                    }
                    hint="Drop an archive or folder — opens the import set to add to this bundle"
                    multiple
                    busy={mergePct !== null}
                    progress={mergePct !== null && mergePct < 100 ? mergePct : undefined}
                    onDrop={(drop) => {
                      if (!drop.files.length) return
                      setMergePct(0)
                      void startImport(drop, (f) => setMergePct(Math.round(f * 100)))
                        .then(async (staged) => {
                          await queryClient.invalidateQueries({ queryKey: ['imports'] })
                          navigate(`/imports/${staged.id}?bundle=${bundle.id}`)
                        })
                        .catch((err) =>
                          setUploadError(err instanceof Error ? err.message : String(err)),
                        )
                        .finally(() => setMergePct(null))
                    }}
                  />
                </Box>
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Dropzone
                    label="Import scraped metadata"
                    hint="Drop a bundle-patch zip — patch.json + images"
                    accept=".zip"
                    onDrop={(drop) => {
                      const file = drop.files[0]?.file
                      if (file) {
                        setPatchFile(file)
                        setPatchOpen(true)
                      }
                    }}
                  />
                </Box>
              </Stack>
            </>
          )}
          {!editing && (bundle.creator_name || bundle.source_url) && (
            <Typography color="text.secondary" sx={{ mb: 1 }}>
              {bundle.creator_name && <>by {bundle.creator_name}</>}
              {bundle.creator_name && bundle.source_url && ', '}
              {bundle.source_url && (
                <>
                  from{' '}
                  <a href={bundle.source_url} target="_blank" rel="noreferrer">
                    {sourceOrigin(bundle.source_url)}
                  </a>
                </>
              )}
            </Typography>
          )}
          {!editing &&
            bundle.custom_fields.map((entry) => (
              <CustomFieldReadout key={entry.field.id} entry={entry} />
            ))}
          <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap', gap: 1, mb: 2 }}>
            {!editing && bundle.tags.map((tag) => <Chip key={tag} label={tag} size="small" />)}
          </Stack>

          {!editing && (
            <>
              <Stack sx={{ alignItems: 'center' }} direction="row" spacing={1}>
                <Typography variant="h6">Description</Typography>
                <Button size="small" onClick={() => setHistoryOpen(true)}>
                  history
                </Button>
              </Stack>
              <Box sx={{ '& p': { mt: 0.5 }, mb: 2 }}>
                {bundle.description_md ? (
                  <ReactMarkdown>{bundle.description_md}</ReactMarkdown>
                ) : (
                  <Typography color="text.secondary" variant="body2">
                    No description.
                  </Typography>
                )}
              </Box>
            </>
          )}

          {/* Loose files sit with the description rather than full-width below:
              they are a property of the bundle itself, like its tags and its
              text, and the full width beneath belongs to the member grid.
              `mt: auto` sinks them to the bottom of the column, so a short
              description leaves a gap rather than stranding them mid-air. */}
          <Box sx={{ mt: 'auto' }}>
            <BundleUnsortedSection
              bundle={bundle}
              canEdit={!!canEdit}
              editing={editing}
              onChange={refresh}
            />
          </Box>
        </Box>
      </Stack>

      {/* Members run the full width beneath the gallery/details block, so the
          grid gets every column it can and its category tabs have room. */}
      <Divider sx={{ my: 3 }} />
      <MembersSection bundle={bundle} canEdit={!!canEdit} editing={editing} onChange={refresh} />

      <DescriptionHistoryDialog
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        owner="bundles"
        entity={bundle}
        canEdit={!!canEdit}
        onChange={refresh}
      />
      <ExportDialog open={exportOpen} onClose={() => setExportOpen(false)} bundle={bundle} />
      <BundleDeleteDialog
        open={deleteOpen}
        onClose={() => setDeleteOpen(false)}
        bundle={bundle}
        onDeleted={async () => {
          setDeleteOpen(false)
          await queryClient.invalidateQueries()
          navigate('/')
        }}
      />

      <BundleMergeDialog
        open={mergeOpen}
        onClose={() => setMergeOpen(false)}
        bundle={bundle}
        onMerged={async (_merged, from, other) => {
          setMergeOpen(false)
          await queryClient.invalidateQueries()
          setToast(
            other === 'delete'
              ? `Merged “${from.name}” in and deleted it`
              : `Merged “${from.name}” in — it still stands on its own`,
          )
        }}
      />

      <BundleRetagDialog
        open={retagOpen}
        onClose={() => setRetagOpen(false)}
        bundle={bundle}
        onDone={(message) => {
          void queryClient.invalidateQueries({ queryKey: ['bundle', id] })
          // Member tags feed the tag cloud's counts and the browse filters.
          void queryClient.invalidateQueries({ queryKey: ['tags'] })
          setToast(message)
        }}
      />

      <BundlePatchDialog
        bundleId={bundle.id}
        open={patchOpen}
        initialFile={patchFile}
        onApplied={() => {
          // Don't reload here — the dialog is still open showing its summary, and
          // the reload's navigation would blank the page and unmount it, popping a
          // fresh preview back up. Remember the reload is owed and run it on close.
          patchReloadOwed.current = true
          // The in-place editor seeds its fields once at mount, so it would keep
          // showing the pre-patch name/tags/description — and saving that stale
          // state would revert the patch we just applied. Leave edit mode so the
          // refreshed values show through the read view.
          setEditing(false)
        }}
        onClose={() => {
          setPatchOpen(false)
          setPatchFile(null)
          if (!patchReloadOwed.current) return
          patchReloadOwed.current = false
          // A patch can rename the bundle (when its name was still autogenerated),
          // which moves the slug — and the URL. The old slug in the address bar no
          // longer resolves, so refetching it in place would 404 and nothing would
          // reload. Drop its cache and jump to the stable UUID; the canonical-slug
          // redirect (the effect above) then lands us on the new slug with freshly
          // fetched data.
          queryClient.removeQueries({ queryKey: ['bundle', bundle.id] })
          navigate(`/bundles/${bundle.id}`, { replace: true })
        }}
      />
      <ImportErrorDialog error={uploadError} onClose={() => setUploadError('')} />
      <Snackbar
        open={!!toast}
        autoHideDuration={4000}
        onClose={() => setToast('')}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert
          severity={toast.startsWith('Paste failed') ? 'error' : 'success'}
          onClose={() => setToast('')}
        >
          {toast}
        </Alert>
      </Snackbar>
    </Container>
  )
}

/// Return a copy of `arr` with the item at `from` moved to index `to`.
function moveItem<T>(arr: T[], from: number, to: number): T[] {
  const next = [...arr]
  const [item] = next.splice(from, 1)
  next.splice(to, 0, item)
  return next
}

/// A bundle's members. A carve on the way in from an import is the usual way a
/// model lands here, but not the only one: in edit mode a search box adds an
/// existing model, so a bundle can be curated after the fact rather than only
/// assembled from the archive it arrived in.
function MembersSection({
  bundle,
  canEdit,
  editing,
  onChange,
}: {
  bundle: BundleDetail
  canEdit: boolean
  /** Edit mode: only here can a model be pulled out of the bundle, its
      categories curated, or a group of members split off. */
  editing: boolean
  onChange: () => void
}) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const bundleId = bundle.id
  // The bundle's creator: a member sharing it doesn't repeat it on its card.
  const bundleCreatorId = bundle.creator_id
  const models = bundle.models
  // The bundle's defined categories (import sections), in tab order.
  const categories = bundle.categories
  // Which category tab is active; null = "All Models". Held in the URL rather
  // than component state so opening a member and coming back restores the tab
  // you were on. `replace` keeps re-tabbing on the same page to one history
  // entry, so Back leaves the bundle in one step rather than walking every tab
  // you tried.
  const [params, setParams] = useSearchParams()
  const category = params.get('cat')
  const setCategory = (next: string | null) => {
    const p = new URLSearchParams(params)
    if (next) p.set('cat', next)
    else p.delete('cat')
    setParams(p, { replace: true })
  }
  const [addValue, setAddValue] = useState('')
  const [savingCats, setSavingCats] = useState(false)
  // Which members are picked out for a split. Edit-mode only, and dropped on the
  // way out of it: a stale selection would arm the next split with models
  // someone chose for a different reason.
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [splitOpen, setSplitOpen] = useState(false)
  // The last card clicked, so shift-click can select the run between them —
  // picking twelve of twenty knights one checkbox at a time is not picking.
  const anchor = useRef<string | null>(null)
  useEffect(() => {
    if (!editing) {
      setSelected(new Set())
      anchor.current = null
    }
  }, [editing])
  // Drag-to-reorder the category rows: the row being dragged, and the row the
  // pointer is currently over (for the drop-line cue).
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const [overIndex, setOverIndex] = useState<number | null>(null)

  const refreshAll = async () => {
    await queryClient.invalidateQueries({ queryKey: ['bundle', bundleId] })
    onChange()
  }

  // The add-an-existing-model picker. `useDeferredValue` is the debounce: the
  // input stays responsive while the search lags a keystroke behind it.
  const [memberQuery, setMemberQuery] = useState('')
  const [addingMember, setAddingMember] = useState(false)
  const [addError, setAddError] = useState('')
  const search = useDeferredValue(memberQuery.trim())
  const { data: found, isFetching: searching } = useQuery({
    queryKey: ['model-search', search],
    enabled: canEdit && editing && search.length > 0,
    queryFn: () =>
      api.searchModels(new URLSearchParams({ q: search, per_page: '20' })).then((r) => r.models),
  })
  // Members can't be added twice — `ON CONFLICT DO NOTHING` would swallow it
  // silently, which reads as the picker having done nothing.
  const memberIds = useMemo(() => new Set(models.map((m) => m.id)), [models])
  const candidates = useMemo(
    () => (found ?? []).filter((m) => !memberIds.has(m.id)),
    [found, memberIds],
  )

  const addMember = async (model: import('../api').ModelSummary) => {
    setAddingMember(true)
    setAddError('')
    try {
      await api.addModelToBundle(bundleId, model.id)
      setMemberQuery('')
      await refreshAll()
    } catch (e) {
      setAddError(e instanceof Error ? e.message : String(e))
    } finally {
      setAddingMember(false)
    }
  }

  // Members carrying each tag — for per-category counts (and the add picker).
  const tagCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const m of models) for (const tag of m.tags) counts.set(tag, (counts.get(tag) ?? 0) + 1)
    return counts
  }, [models])

  // For a bundle with no categories defined yet, fall back to the old heuristic
  // so it still gets tabs: the title-cased member tags (Heroes, Enemies…) rather
  // than the lowercase descriptive ones (undead, medium…), ordered by size.
  const derived = useMemo(
    () =>
      [...tagCounts.entries()]
        .filter(([tag]) => {
          const first = tag[0]
          return first && first.toLowerCase() !== first
        })
        .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
        .map(([tag]) => tag),
    [tagCounts],
  )
  const tabList = categories.length ? categories : derived

  // Reset the active tab if its category is gone (removed, or retired by a carve).
  useEffect(() => {
    if (category && !tabList.includes(category)) setCategory(null)
  }, [tabList, category])

  const shown = category ? models.filter((m) => m.tags.includes(category)) : models
  const picked = models.filter((m) => selected.has(m.id))

  /// Tick one card. Shift extends from the last one ticked, so a run of members
  /// is two clicks rather than twelve.
  const toggle = (index: number, extend: boolean) => {
    const from = extend ? shown.findIndex((m) => m.id === anchor.current) : -1
    const run =
      from < 0 ? [shown[index]] : shown.slice(Math.min(from, index), Math.max(from, index) + 1)
    // A shift-click paints the run with the state the *clicked* card is taking,
    // so dragging back over a run you just selected clears it.
    const on = !selected.has(shown[index].id)
    const next = new Set(selected)
    for (const model of run) {
      if (on) next.add(model.id)
      else next.delete(model.id)
    }
    anchor.current = shown[index].id
    setSelected(next)
  }
  const selectAlso = (more: import('../api').ModelSummary[]) =>
    setSelected(new Set([...selected, ...more.map((m) => m.id)]))

  const saveCategories = async (next: string[]) => {
    setSavingCats(true)
    try {
      await api.setBundleCategories(bundleId, next)
      await refreshAll()
    } finally {
      setSavingCats(false)
    }
  }

  const dropAt = (to: number) => {
    const from = dragIndex
    setDragIndex(null)
    setOverIndex(null)
    if (from === null || from === to) return
    void saveCategories(moveItem(categories, from, to))
  }

  return (
    <Box>
      {editing ? (
        <Paper variant="outlined" sx={{ p: 1.5, mb: 2 }}>
          <Typography variant="subtitle2" sx={{ mb: 1 }}>
            Categories — the bundle's sections and their tab order
          </Typography>
          <Stack spacing={0} sx={{ mb: 1.5 }}>
            {categories.map((cat, i) => (
              <Stack
                key={cat}
                direction="row"
                spacing={0.5}
                draggable={!savingCats}
                onDragStart={() => setDragIndex(i)}
                onDragEnd={() => {
                  setDragIndex(null)
                  setOverIndex(null)
                }}
                onDragOver={(e) => {
                  e.preventDefault()
                  if (overIndex !== i) setOverIndex(i)
                }}
                onDrop={() => dropAt(i)}
                sx={{
                  alignItems: 'center',
                  py: 0.25,
                  opacity: dragIndex === i ? 0.4 : 1,
                  // Drop-line cue on the row the pointer is over.
                  borderTop: (t) =>
                    overIndex === i && dragIndex !== null && dragIndex !== i
                      ? `2px solid ${t.palette.primary.main}`
                      : '2px solid transparent',
                }}
              >
                <Tooltip title="Drag to reorder">
                  <DragHandleIcon
                    fontSize="small"
                    sx={{ color: 'text.disabled', cursor: savingCats ? 'default' : 'grab' }}
                  />
                </Tooltip>
                <Typography variant="body2" sx={{ flexGrow: 1, ml: 0.5 }}>
                  {cat}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {tagCounts.get(cat) ?? 0} model{(tagCounts.get(cat) ?? 0) === 1 ? '' : 's'}
                </Typography>
                <Tooltip title="Remove this category (doesn't untag any model)">
                  <IconButton
                    size="small"
                    disabled={savingCats}
                    onClick={() => saveCategories(categories.filter((c) => c !== cat))}
                  >
                    <CloseIcon fontSize="small" />
                  </IconButton>
                </Tooltip>
              </Stack>
            ))}
            {categories.length === 0 && (
              <Typography variant="body2" color="text.secondary">
                No categories defined — add one below to give this bundle section tabs.
              </Typography>
            )}
          </Stack>
          <Autocomplete
            freeSolo
            size="small"
            options={[...tagCounts.keys()].filter((t) => !categories.includes(t)).sort()}
            value={null}
            inputValue={addValue}
            disabled={savingCats}
            onInputChange={(_, v) => setAddValue(v)}
            onChange={(_, v) => {
              const name = (typeof v === 'string' ? v : '').trim()
              if (!name || categories.some((c) => c.toLowerCase() === name.toLowerCase())) return
              setAddValue('')
              void saveCategories([...categories, name])
            }}
            renderInput={(props) => (
              <TextField {...props} placeholder="Add a category (a model tag)…" />
            )}
            sx={{ maxWidth: 340 }}
          />
        </Paper>
      ) : (
        tabList.length > 0 && (
          <Tabs
            value={category ?? 'all'}
            onChange={(_, value) => setCategory(value === 'all' ? null : value)}
            variant="scrollable"
            scrollButtons="auto"
            sx={{ mb: 2, borderBottom: (t) => `1px solid ${t.palette.divider}` }}
          >
            <Tab value="all" label={`All Models (${models.length})`} />
            {tabList.map((name) => (
              <Tab key={name} value={name} label={`${name} (${tagCounts.get(name) ?? 0})`} />
            ))}
          </Tabs>
        )
      )}

      {canEdit && editing && (
        <Paper variant="outlined" sx={{ p: 1.5, mb: 2 }}>
          <Typography variant="subtitle2" sx={{ mb: 1 }}>
            Add an existing model to this bundle
          </Typography>
          <Autocomplete
            size="small"
            options={candidates}
            loading={searching}
            value={null}
            inputValue={memberQuery}
            disabled={addingMember}
            filterOptions={(x) => x}
            getOptionLabel={(m) => m.name}
            noOptionsText={memberQuery.trim() ? 'No matching models' : 'Type to search models'}
            onInputChange={(_, v, reason) => {
              if (reason !== 'reset') setMemberQuery(v)
            }}
            onChange={(_, model) => {
              if (model) void addMember(model)
            }}
            renderInput={(props) => <TextField {...props} placeholder="Search models by name…" />}
            sx={{ maxWidth: 340 }}
          />
          {addError && (
            <Alert severity="error" sx={{ mt: 1 }}>
              {addError}
            </Alert>
          )}
        </Paper>
      )}

      {/* Group-select and split. A carve that read one Patreon month as a single
          bundle got the grouping wrong, not the models — so the fix is to pick
          the ones that belong together and lift them out, not to re-import. */}
      {canEdit && editing && models.length > 0 && (
        <Paper variant="outlined" sx={{ p: 1.5, mb: 2 }}>
          <Stack
            direction="row"
            spacing={1}
            sx={{ alignItems: 'center', flexWrap: 'wrap', gap: 1 }}
          >
            <Typography variant="subtitle2" sx={{ mr: 0.5 }}>
              {selected.size
                ? `${selected.size} model${selected.size === 1 ? '' : 's'} selected`
                : 'Select models to split out'}
            </Typography>
            <Button size="small" onClick={() => selectAlso(shown)}>
              All
            </Button>
            <Button size="small" disabled={!selected.size} onClick={() => setSelected(new Set())}>
              None
            </Button>
            {/* One click per section: the categories are already how this bundle
                thinks about its members. */}
            {tabList.map((tag) => (
              <Chip
                key={tag}
                size="small"
                variant="outlined"
                clickable
                label={`${tag} (${tagCounts.get(tag) ?? 0})`}
                onClick={() => selectAlso(models.filter((m) => m.tags.includes(tag)))}
              />
            ))}
            <Box sx={{ flexGrow: 1 }} />
            <Button
              size="small"
              variant="contained"
              startIcon={<CallSplitIcon />}
              disabled={!selected.size}
              onClick={() => setSplitOpen(true)}
            >
              Split into a new bundle
            </Button>
          </Stack>
          <Typography variant="caption" color="text.secondary">
            Shift-click a card to select the run from the last one.
          </Typography>
        </Paper>
      )}

      <BundleSplitDialog
        open={splitOpen}
        onClose={() => setSplitOpen(false)}
        bundle={bundle}
        models={picked}
        onSplit={async (created) => {
          setSplitOpen(false)
          setSelected(new Set())
          await queryClient.invalidateQueries()
          navigate(`/bundles/${created.slug}`)
        }}
      />

      {models.length === 0 ? (
        <Typography color="text.secondary" variant="body2">
          No models in this bundle yet
          {canEdit ? ' — drop an archive or folder above, then carve it into models' : ''}.
        </Typography>
      ) : (
        <Box
          sx={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))',
            gap: 2,
          }}
        >
          {shown.map((model, index) => (
            <Box key={model.id} sx={{ position: 'relative' }}>
              <ModelCard model={model} hideCreator={model.creator_id === bundleCreatorId} />
              {canEdit && editing && (
                <Checkbox
                  size="small"
                  checked={selected.has(model.id)}
                  // onClick, not onChange: the shift key is what makes this a
                  // range, and only a mouse event carries it. Deliberately no
                  // preventDefault — cancelling a checkbox's own activation
                  // leaves React's value tracker believing the DOM already
                  // holds the new state, and the tick then never renders. The
                  // clicked box always lands on the state the browser just
                  // flipped it to, so letting the flip stand is also correct.
                  onClick={(e) => {
                    e.stopPropagation()
                    toggle(index, e.shiftKey)
                  }}
                  sx={{
                    position: 'absolute',
                    top: 4,
                    left: 4,
                    // Above the card's own click targets (ModelCard lifts its
                    // action area to 2), or the card swallows the tick and
                    // opens the model instead.
                    zIndex: 3,
                    bgcolor: 'background.paper',
                    p: 0.25,
                  }}
                />
              )}
              {canEdit && editing && (
                <Tooltip title="Remove from bundle">
                  <IconButton
                    size="small"
                    sx={{
                      position: 'absolute',
                      top: 4,
                      right: 4,
                      bgcolor: 'background.paper',
                      // Same reason as the tick opposite: below 3 the card's
                      // action area covers this and the click opens the model.
                      zIndex: 3,
                    }}
                    onClick={async () => {
                      await api.removeModelFromBundle(bundleId, model.id)
                      await refreshAll()
                    }}
                  >
                    <RemoveCircleIcon fontSize="small" color="error" />
                  </IconButton>
                </Tooltip>
              )}
            </Box>
          ))}
        </Box>
      )}
    </Box>
  )
}
