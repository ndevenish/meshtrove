import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import {
  Container,
  Box,
  Typography,
  Chip,
  Stack,
  Button,
  Paper,
  IconButton,
  Tooltip,
  Divider,
  Tab,
  Tabs,
  Snackbar,
  Alert,
} from '@mui/material'
import EditIcon from '@mui/icons-material/Edit'
import StarIcon from '@mui/icons-material/Star'
import StarBorderIcon from '@mui/icons-material/StarBorder'
import DeleteIcon from '@mui/icons-material/Delete'
import AddPhotoAlternateIcon from '@mui/icons-material/AddPhotoAlternate'
import RemoveCircleIcon from '@mui/icons-material/RemoveCircle'
import ReactMarkdown from 'react-markdown'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, imageUrl, sourceOrigin } from '../api'
import { useAuth } from '../main'
import { usePasteImage } from '../usePasteImage'
import { useSuppressGlobalDrop } from '../globalDrop'
import { startImport } from '../upload'
import Dropzone from '../components/Dropzone'
import ModelCard from '../components/ModelCard'
import BundleDetailsEditor from '../components/BundleDetailsEditor'
import { type DetailsEditorHandle } from '../components/ModelDetailsEditor'
import BundleUnsortedSection from '../components/BundleUnsortedSection'
import DescriptionHistoryDialog from '../components/DescriptionHistoryDialog'
import ImportErrorDialog from '../components/ImportErrorDialog'
import BundlePatchDialog from '../components/BundlePatchDialog'

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
  const [patchOpen, setPatchOpen] = useState(false)
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

  const canEditBundle =
    !!bundle &&
    !!user &&
    (user.role === 'admin' || (user.role === 'editor' && user.id === bundle.created_by))
  usePasteImage(canEditBundle, 'bundles', id ?? '', {
    onUploaded: () => {
      void queryClient.invalidateQueries({ queryKey: ['bundle', id] })
      setToast('Image added from clipboard')
    },
    onError: (m) => setToast(`Paste failed: ${m}`),
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
            sx={{
              aspectRatio: '1',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              overflow: 'hidden',
            }}
          >
            {shownImage ? (
              <Box
                component="img"
                src={imageUrl(shownImage)}
                alt={bundle.name}
                sx={{ width: '100%', height: '100%', objectFit: 'contain' }}
              />
            ) : (
              <Typography color="text.secondary" sx={{ textAlign: 'center', px: 2 }}>
                No images yet
                {canEdit && (
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    Paste an image (⌘V) to add one
                  </Typography>
                )}
              </Typography>
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
            {canEdit && (
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

        {/* Details */}
        <Box sx={{ flexGrow: 1, minWidth: 0 }}>
          <Stack sx={{ alignItems: 'flex-start' }} direction="row" spacing={1}>
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center', flexGrow: 1 }}>
              <Typography variant="h4" sx={{ fontWeight: 700 }}>
                {bundle.name}
              </Typography>
              <Chip label={bundle.kind} size="small" color="primary" variant="outlined" />
            </Stack>
            {canEdit && !editing && (
              <Button startIcon={<EditIcon />} onClick={() => setEditing(true)}>
                Edit
              </Button>
            )}
            {canEdit && editing && (
              <Stack direction="row" spacing={1}>
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
        </Box>
      </Stack>

      {/* Contents run the full width beneath the gallery/details block, so the
          member grid gets every column it can and its category tabs have room. */}
      <Divider sx={{ my: 3 }} />
      <BundleUnsortedSection
        bundle={bundle}
        canEdit={!!canEdit}
        editing={editing}
        onChange={refresh}
      />
      <MembersSection
        bundleId={bundle.id}
        bundleCreatorId={bundle.creator_id}
        models={bundle.models}
        canEdit={!!canEdit}
        editing={editing}
        onChange={refresh}
      />

      <DescriptionHistoryDialog
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        owner="bundles"
        entity={bundle}
        canEdit={!!canEdit}
        onChange={refresh}
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

/// A bundle's members. Deliberately *not* a place to add a model: membership is
/// something a carve decides, on the way in from an import — picking an existing
/// model out of a search box and dropping it in a box set is how a model ends up
/// in two collections it was never sold with. Removing one stays, because a bad
/// carve has to be undoable.
function MembersSection({
  bundleId,
  bundleCreatorId,
  models,
  canEdit,
  editing,
  onChange,
}: {
  bundleId: string
  /** The bundle's creator: a member sharing it doesn't repeat it on its card. */
  bundleCreatorId: string | null
  models: import('../api').ModelSummary[]
  canEdit: boolean
  /** Edit mode: only here can a model be pulled out of the bundle. */
  editing: boolean
  onChange: () => void
}) {
  const queryClient = useQueryClient()
  // Which category tab is active; null = "All Models".
  const [category, setCategory] = useState<string | null>(null)

  const refreshAll = async () => {
    await queryClient.invalidateQueries({ queryKey: ['bundle', bundleId] })
    onChange()
  }

  // The bundle's primary categories are the import's section tags — the
  // title-cased model tags (Heroes, Enemies, NPC…), as opposed to the lowercase
  // descriptive tags a scrape adds (undead, medium, objects…). Derived from the
  // members so each bundle offers its own sections, with a per-tab count. Ordered
  // by size: the import's folder order (1 - Heroes, 2 - Enemies) isn't preserved.
  const categories = useMemo(() => {
    const counts = new Map<string, number>()
    for (const m of models) {
      for (const tag of m.tags) {
        const first = tag[0]
        if (first && first.toLowerCase() !== first) counts.set(tag, (counts.get(tag) ?? 0) + 1)
      }
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
  }, [models])

  // A re-carve can retire the active category (its last model retagged or
  // removed): fall back to All rather than leaving a dead tab over an empty grid.
  useEffect(() => {
    if (category && !categories.some(([name]) => name === category)) setCategory(null)
  }, [categories, category])

  const shown = category ? models.filter((m) => m.tags.includes(category)) : models

  return (
    <Box>
      <Stack direction="row" sx={{ alignItems: 'center', mb: 1 }} spacing={1}>
        <Typography variant="h6">Models</Typography>
        <Typography variant="body2" color="text.secondary">
          {models.length}
        </Typography>
      </Stack>

      {categories.length > 0 && (
        <Tabs
          value={category ?? 'all'}
          onChange={(_, value) => setCategory(value === 'all' ? null : value)}
          variant="scrollable"
          scrollButtons="auto"
          sx={{ mb: 2, borderBottom: (t) => `1px solid ${t.palette.divider}` }}
        >
          <Tab value="all" label={`All Models (${models.length})`} />
          {categories.map(([name, count]) => (
            <Tab key={name} value={name} label={`${name} (${count})`} />
          ))}
        </Tabs>
      )}

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
          {shown.map((model) => (
            <Box key={model.id} sx={{ position: 'relative' }}>
              <ModelCard model={model} hideCreator={model.creator_id === bundleCreatorId} />
              {canEdit && editing && (
                <Tooltip title="Remove from bundle">
                  <IconButton
                    size="small"
                    sx={{
                      position: 'absolute',
                      top: 4,
                      right: 4,
                      bgcolor: 'background.paper',
                      zIndex: 1,
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
