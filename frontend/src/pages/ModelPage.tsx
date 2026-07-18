import { useCallback, useEffect, useRef, useState } from 'react'
import { useParams, useNavigate, Link } from 'react-router-dom'
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
  Snackbar,
  Alert,
  alpha,
} from '@mui/material'
import EditIcon from '@mui/icons-material/Edit'
import StarIcon from '@mui/icons-material/Star'
import StarBorderIcon from '@mui/icons-material/StarBorder'
import DeleteIcon from '@mui/icons-material/Delete'
import AddPhotoAlternateIcon from '@mui/icons-material/AddPhotoAlternate'
import Inventory2Icon from '@mui/icons-material/Inventory2'
import ReactMarkdown from 'react-markdown'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import DownloadIcon from '@mui/icons-material/Download'
import { api, imageUrl, sourceOrigin } from '../api'
import ExportDialog from '../components/ExportDialog'
import { useAuth } from '../main'
import { usePasteImage, useDropImage } from '../imageGestures'
import ModelDetailsEditor, { type DetailsEditorHandle } from '../components/ModelDetailsEditor'
import VariantSection from '../components/VariantSection'
import UnsortedSection from '../components/UnsortedSection'
import DescriptionHistoryDialog from '../components/DescriptionHistoryDialog'

export default function ModelPage() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const { user } = useAuth()
  const queryClient = useQueryClient()
  // Edit *mode*, not an edit dialog: the fields become editable where they sit,
  // and the destructive buttons — delete file, delete variant, delete image —
  // appear only here. Browsing a model should not be one stray click away from
  // deleting a file of it.
  const [editing, setEditing] = useState(false)
  // Save and Cancel replace Edit in the header — leaving the mode belongs where
  // entering it was. The fields live in the editor below, so the page reaches
  // into it to save.
  const editorRef = useRef<DetailsEditorHandle>(null)
  const [saving, setSaving] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [exportOpen, setExportOpen] = useState(false)
  const [selectedImage, setSelectedImage] = useState<string | null>(null)
  const [toast, setToast] = useState('')

  const { data: model } = useQuery({
    queryKey: ['model', id],
    queryFn: () => api.model(id!),
    enabled: !!id,
  })

  // Canonical URL is the slug. Arriving by UUID (a redirect, an old bookmark) —
  // or by a slug that a rename has since moved on from — lands here, resolves,
  // then rewrites the address bar to the slug. Seed the slug's cache with what
  // we already hold so the swap doesn't flash a reload.
  useEffect(() => {
    if (model && id !== model.slug) {
      queryClient.setQueryData(['model', model.slug], model)
      navigate(`/models/${model.slug}`, { replace: true })
    }
  }, [model, id, navigate, queryClient])

  // A render finishing adds a picture to this page, and the page has no way to
  // know: the job writes the image straight to the database. So watch the queue.
  //
  // Watch for renders that *have finished*, not for the queue going idle. A single
  // f3d render takes about a second — usually less than one poll — so waiting for
  // a "was rendering, now isn't" edge misses it entirely: the job is already
  // `succeeded` the first time we look, the edge never happens, and the picture
  // sits in the database until you reload. Instead, remember which finished jobs
  // have been accounted for; any id that shows up finished and unaccounted means a
  // new image may exist, so refetch.
  //
  // (Any render, not just this model's: a job's payload names a file, and knowing
  // whether that file is ours would mean fetching every variant's file list. One
  // model refetch is cheaper than that bookkeeping, and if nothing changed the
  // page simply redraws itself.)
  const { data: jobs } = useQuery({
    queryKey: ['jobs', 'all'],
    queryFn: () => api.jobs(''),
    refetchInterval: (query) =>
      (query.state.data ?? []).some(
        (j) => j.kind === 'render_preview' && (j.status === 'queued' || j.status === 'running'),
      )
        ? 1500
        : false,
  })
  const settledRenders = useRef<Set<number> | null>(null)
  useEffect(() => {
    if (!jobs) return
    const finished = jobs
      .filter(
        (j) => j.kind === 'render_preview' && (j.status === 'succeeded' || j.status === 'failed'),
      )
      .map((j) => j.id)
    // The first payload is the baseline: everything already finished when the page
    // opened is old news, and refetching for it would be a pointless round trip.
    if (settledRenders.current === null) {
      settledRenders.current = new Set(finished)
      return
    }
    const seen = settledRenders.current
    const fresh = finished.filter((jobId) => !seen.has(jobId))
    if (fresh.length > 0) {
      for (const jobId of fresh) seen.add(jobId)
      void queryClient.invalidateQueries({ queryKey: ['model', id] })
    }
  }, [jobs, id, queryClient])

  // A picture that just arrived is the one you want to look at — you pressed
  // Render, or pasted, to see *it*, not to add a thumbnail to a row you then have
  // to hunt through. So the viewer follows anything new into the gallery.
  //
  // Keyed on ids rather than count: a render that replaces an image leaves the
  // count unchanged, and a delete would otherwise look like an arrival.
  const knownImages = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (!model) return
    const ids = model.images.map((image) => image.id)
    const known = knownImages.current
    knownImages.current = new Set(ids)
    // First sight of the model is not an arrival: it is just the page loading.
    if (known === null) return
    const fresh = ids.filter((imageId) => !known.has(imageId))
    // Several at once (an import rendering every variant) — any of them is a
    // better thing to be looking at than the one that was there before.
    if (fresh.length > 0) setSelectedImage(fresh[0])
  }, [model])

  const canEditModel =
    !!model &&
    !!user &&
    (user.role === 'admin' || (user.role === 'editor' && user.id === model.created_by))
  // Both gestures address the model by **UUID**: `id` here is the slug (the
  // canonical URL), and the image routes parse their path segment as a Uuid, so
  // passing it straight through fails the extractor before the handler runs.
  const modelId = model?.id ?? ''
  const imageAdded = useCallback(
    (how: string) => {
      void queryClient.invalidateQueries({ queryKey: ['model', id] })
      setToast(`Image added ${how}`)
    },
    [queryClient, id],
  )
  usePasteImage(canEditModel, 'models', modelId, {
    onUploaded: () => imageAdded('from clipboard'),
    onError: (m) => setToast(`Paste failed: ${m}`),
  })
  const droppingImage = useDropImage(canEditModel, 'models', modelId, {
    onUploaded: () => imageAdded('to this model'),
    onError: (m) => setToast(`Image upload failed: ${m}`),
  })

  if (!model) return null
  const canEdit =
    user && (user.role === 'admin' || (user.role === 'editor' && user.id === model.created_by))
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['model', id] })

  const shownImage = selectedImage ?? model.images[0]?.id ?? null

  const uploadImage = async (file: File) => {
    const form = new FormData()
    form.append('file', file)
    await api.uploadImage('models', model.id, form)
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
              // The drop itself is caught page-wide (imageGestures.ts) — this is
              // only where the page says so, since the gallery is where the
              // picture is going to appear.
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
                alt={model.name}
                sx={{ width: '100%', height: '100%', objectFit: 'contain' }}
              />
            ) : (
              <Typography color="text.secondary" sx={{ textAlign: 'center', px: 2 }}>
                No images yet
                {canEdit && (
                  <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                    Drop one on the page, or paste (⌘V)
                  </Typography>
                )}
              </Typography>
            )}
          </Paper>
          <Stack direction="row" spacing={1} sx={{ mt: 1, flexWrap: 'wrap', gap: 1 }}>
            {model.images.map((image) => {
              // "Primary" on a variant's image means primary *of that variant* —
              // it says nothing about the model. Only the model's own image can
              // be the model's favourite, so that is what the star reflects.
              const isModelPrimary = !image.variant_id && image.is_primary
              return (
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
                      <Tooltip
                        title={
                          isModelPrimary
                            ? 'Primary image'
                            : image.variant_id
                              ? 'Make primary (promotes this variant’s picture to the model)'
                              : 'Make primary'
                        }
                      >
                        <IconButton
                          size="small"
                          sx={{ p: 0.25, bgcolor: 'background.paper' }}
                          onClick={async () => {
                            if (isModelPrimary) return
                            // A variant's picture can't just be flagged: "primary"
                            // on a variant image means primary *of that variant*.
                            // Favouriting it here is a statement about the model, so
                            // the model takes a copy of the blob as its own.
                            if (image.variant_id) await api.promoteImage(model.id, image.id)
                            else await api.markPrimary(image.id)
                            refresh()
                          }}
                        >
                          {isModelPrimary ? (
                            <StarIcon sx={{ fontSize: 16, color: 'primary.main' }} />
                          ) : (
                            <StarBorderIcon sx={{ fontSize: 16 }} />
                          )}
                        </IconButton>
                      </Tooltip>
                      {/* Choosing the favourite is safe and stays; deleting the
                        picture is not, and waits for edit mode. */}
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
              )
            })}
            {/* Same line as the delete button above: adding a picture is an edit,
                so the tile waits for edit mode. Dropping and pasting are
                unaffected — they are caught page-wide (imageGestures.ts) and stay
                available whenever you can edit at all. */}
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

        {/* Details */}
        <Box sx={{ flexGrow: 1, minWidth: 0 }}>
          <Stack sx={{ alignItems: 'flex-start' }} direction="row" spacing={1}>
            <Typography variant="h4" sx={{ fontWeight: 700, flexGrow: 1 }}>
              {model.name}
            </Typography>
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
              <Stack direction="row" spacing={1}>
                <Button
                  variant="contained"
                  disabled={saving}
                  onClick={() => {
                    void editorRef.current?.save().catch(() => {
                      // The editor shows the reason; stay in edit mode so the
                      // half-typed changes are not thrown away.
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
            <ModelDetailsEditor
              key={model.id}
              ref={editorRef}
              model={model}
              onDone={() => setEditing(false)}
              onBusyChange={setSaving}
            />
          )}
          {!editing && (model.creator_name || model.source_url) && (
            <Typography color="text.secondary" sx={{ mb: 1 }}>
              {model.creator_name && (
                <>
                  by{' '}
                  <Link to={`/creators?q=${encodeURIComponent(model.creator_name)}`}>
                    {model.creator_name}
                  </Link>
                </>
              )}
              {model.creator_name && model.source_url && ', '}
              {model.source_url && (
                <>
                  from{' '}
                  <a href={model.source_url} target="_blank" rel="noreferrer">
                    {sourceOrigin(model.source_url)}
                  </a>
                </>
              )}
            </Typography>
          )}
          {model.bundles.length > 0 && (
            <Stack
              direction="row"
              spacing={1}
              sx={{ alignItems: 'center', mb: 1, flexWrap: 'wrap', gap: 1 }}
            >
              <Typography variant="body2" color="text.secondary">
                In bundle:
              </Typography>
              {model.bundles.map((b) => (
                <Chip
                  key={b.id}
                  icon={<Inventory2Icon />}
                  label={b.name}
                  size="small"
                  color="primary"
                  variant="outlined"
                  component={Link}
                  to={`/bundles/${b.slug}`}
                  clickable
                />
              ))}
            </Stack>
          )}
          <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap', gap: 1, mb: 2 }}>
            {!editing &&
              model.tags.map((tag) => (
                <Chip
                  key={tag}
                  label={tag}
                  size="small"
                  component={Link}
                  to={`/?tags=${encodeURIComponent(tag)}`}
                  clickable
                />
              ))}
          </Stack>

          {(model.license || model.purchase_price != null) && (
            <Paper variant="outlined" sx={{ p: 1.5, mb: 2 }}>
              <Stack direction="row" spacing={3} sx={{ flexWrap: 'wrap', gap: 1 }}>
                {model.license && <Typography variant="body2">License: {model.license}</Typography>}
                {model.purchase_price != null && (
                  <Typography variant="body2">Purchased: {model.purchase_price}</Typography>
                )}
              </Stack>
            </Paper>
          )}

          {!editing && (
            <>
              <Stack sx={{ alignItems: 'center' }} direction="row" spacing={1}>
                <Typography variant="h6">Description</Typography>
                <Button size="small" onClick={() => setHistoryOpen(true)}>
                  history
                </Button>
              </Stack>
              <Box sx={{ '& p': { mt: 0.5 }, mb: 2 }}>
                {model.description_md ? (
                  <ReactMarkdown>{model.description_md}</ReactMarkdown>
                ) : (
                  <Typography color="text.secondary" variant="body2">
                    No description.
                  </Typography>
                )}
              </Box>
            </>
          )}

          <Divider sx={{ mb: 2 }} />
          <UnsortedSection model={model} canEdit={!!canEdit} editing={editing} onChange={refresh} />
          <VariantSection model={model} canEdit={!!canEdit} editing={editing} onChange={refresh} />
        </Box>
      </Stack>

      <DescriptionHistoryDialog
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        owner="models"
        entity={model}
        canEdit={!!canEdit}
        onChange={refresh}
      />
      <ExportDialog open={exportOpen} onClose={() => setExportOpen(false)} model={model} />
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
