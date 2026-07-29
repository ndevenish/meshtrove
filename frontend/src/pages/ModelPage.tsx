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
import MergeIcon from '@mui/icons-material/Merge'
import ContentCutIcon from '@mui/icons-material/ContentCut'
import AddPhotoAlternateIcon from '@mui/icons-material/AddPhotoAlternate'
import Inventory2Icon from '@mui/icons-material/Inventory2'
import ReactMarkdown from 'react-markdown'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import DownloadIcon from '@mui/icons-material/Download'
import { api, imageUrl, sourceOrigin, uploadWithProgress, type FileRecord } from '../api'
import { CustomFieldReadout } from '../components/CustomFieldControl'
import ExportDialog from '../components/ExportDialog'
import { useAuth } from '../main'
import { usePasteImage, useDropImage } from '../imageGestures'
import { useDocumentTitle } from '../documentTitle'
import ModelDetailsEditor, { type DetailsEditorHandle } from '../components/ModelDetailsEditor'
import VariantSection from '../components/VariantSection'
import UnsortedSection from '../components/UnsortedSection'
import DescriptionHistoryDialog from '../components/DescriptionHistoryDialog'
import ModelDeleteDialog from '../components/ModelDeleteDialog'
import ModelMergeDialog from '../components/ModelMergeDialog'
import ModelPatchDialog from '../components/ModelPatchDialog'
import ImageLightbox from '../components/ImageLightbox'
import RenderOrientation, { canReorient } from '../components/RenderOrientation'
import LikeButton from '../components/LikeButton'
import Dropzone from '../components/Dropzone'
import { useSuppressGlobalDrop } from '../globalDrop'

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
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [mergeOpen, setMergeOpen] = useState(false)
  const [selectedImage, setSelectedImage] = useState<string | null>(null)
  const [lightboxOpen, setLightboxOpen] = useState(false)
  const [toast, setToast] = useState('')
  // Where the tags control draws: under the gallery, in the other column from the
  // rest of the form, while the editor keeps the draft. State rather than a ref,
  // so attaching the node re-renders and the portal finds its target.
  const [tagsSlot, setTagsSlot] = useState<HTMLDivElement | null>(null)
  const [uploadPct, setUploadPct] = useState<number | null>(null)
  // Import scraped metadata: the inline drop box lives in edit mode, so the
  // app-wide drop overlay must stand aside while editing or it swallows the zip.
  const [patchOpen, setPatchOpen] = useState(false)
  const [patchFile, setPatchFile] = useState<File | null>(null)
  // A patch was applied and its reload is owed; held until the dialog closes so
  // the reload's navigation doesn't unmount the dialog mid-summary.
  const patchReloadOwed = useRef(false)
  useSuppressGlobalDrop(editing)

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

  useDocumentTitle(model?.name)

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
  const shownRecord = model.images.find((image) => image.id === shownImage) ?? null

  const uploadImage = async (file: File) => {
    const form = new FormData()
    form.append('file', file)
    await api.uploadImage('models', model.id, form)
    refresh()
  }

  /// Files dropped here go straight onto *this* model — no import to stage and
  /// commit, because the question an import exists to ask ("model or bundle?") is
  /// already answered: you are standing on the model. They land in its unsorted
  /// bucket with their folders intact, and a .zip unpacks in the background.
  const uploadStraightIn = async (files: { file: File; path: string }[]) => {
    setUploadPct(0)
    try {
      const form = new FormData()
      for (const { file, path } of files) {
        form.append('path', path) // applies to the file part that follows
        form.append('file', file)
      }
      await uploadWithProgress<FileRecord[]>(`/api/models/${model.id}/files`, form, (f) =>
        setUploadPct(Math.round(f * 100)),
      )
      await refresh()
      await queryClient.invalidateQueries({ queryKey: ['model-files', model.id] })
      await queryClient.invalidateQueries({ queryKey: ['jobs', 'all'] })
    } catch (err) {
      setToast(`Upload failed: ${err instanceof Error ? err.message : String(err)}`)
    } finally {
      setUploadPct(null)
    }
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
              <>
                <Box
                  component="img"
                  src={imageUrl(shownImage, shownRecord?.blob_sha256)}
                  alt={model.name}
                  onClick={() => setLightboxOpen(true)}
                  sx={{ width: '100%', height: '100%', objectFit: 'contain', cursor: 'zoom-in' }}
                />
                {/* Orientation belongs where the picture is big enough to judge
                    it: a render that came out on its side is obvious here and a
                    guess at 72px. The thumbnails carry the same control for the
                    ones you are not currently looking at. */}
                {canEdit && shownRecord && canReorient(shownRecord) && (
                  <RenderOrientation image={shownRecord} onRendered={refresh} edge />
                )}
              </>
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
            {model.images.map((image) => {
              // "Primary" on a variant's image means primary *of that variant* —
              // it says nothing about the model. Only the model's own image can
              // be the model's favourite, so that is what the star reflects.
              const isModelPrimary = !image.variant_id && image.is_primary
              return (
                <Box key={image.id} sx={{ position: 'relative' }}>
                  <Box
                    component="img"
                    src={imageUrl(image.id, image.blob_sha256)}
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
                      {/* Same tier as the star: it adjusts the picture rather
                        than removing it, and anything it does can be undone by
                        setting the axis back. */}
                      {canReorient(image) && (
                        <RenderOrientation image={image} onRendered={refresh} />
                      )}
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
          {/* Tags live under the pictures in both modes — they are how you leave
              this model for another like it, not part of what it says about
              itself, and reading them in one place and editing them in another
              would be two layouts to learn. The column has the width for a strip
              of chips; the details column did not. While editing,
              ModelDetailsEditor keeps the draft and portals its control into this
              node: the layout is the page's, the state stays with the form. */}
          {editing ? (
            <Box ref={setTagsSlot} sx={{ mt: 2 }} />
          ) : (
            model.tags.length > 0 && (
              <Stack direction="row" spacing={1} sx={{ mt: 2, flexWrap: 'wrap', gap: 1 }}>
                {model.tags.map((tag) => (
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
            )
          )}
        </Box>

        {/* Details */}
        <Box sx={{ flexGrow: 1, minWidth: 0 }}>
          <Stack sx={{ alignItems: 'flex-start' }} direction="row" spacing={1}>
            <Typography variant="h4" sx={{ fontWeight: 700, flexGrow: 1 }}>
              {model.name}
            </Typography>
            <LikeButton
              kind="model"
              id={model.id}
              liked={model.liked}
              likeCount={model.like_count}
            />
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
          </Stack>

          {/* Six buttons on the title's line left the title a word wide and
              wrapping. In edit mode they get a line of their own, under the name
              rather than squeezing it. */}
          {canEdit && editing && (
            <Stack
              direction="row"
              spacing={1}
              sx={{ justifyContent: 'flex-end', flexWrap: 'wrap', gap: 1, mt: 1, mb: 1 }}
            >
              {/* Merge and Delete both sit at the far end from Save, clear of
                  where a double-click on the primary action would land, so a
                  stray second click can't fall on either. */}
              {/* The mirror of Merge, and a workbench rather than a dialog:
                  it needs the rule editor and the whole file list, so it gets
                  a page of its own. */}
              <Button
                component={Link}
                to={`/models/${model.slug}/carve`}
                startIcon={<ContentCutIcon />}
                disabled={saving}
                sx={{ whiteSpace: 'nowrap' }}
              >
                Carve…
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
                color="error"
                startIcon={<DeleteIcon />}
                disabled={saving}
                onClick={() => setDeleteOpen(true)}
                sx={{ whiteSpace: 'nowrap' }}
              >
                Delete model
              </Button>
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

          {editing && (
            <>
              <ModelDetailsEditor
                key={model.id}
                ref={editorRef}
                model={model}
                onDone={() => setEditing(false)}
                onBusyChange={setSaving}
                tagsSlot={tagsSlot}
              />
              {/* Two ways of putting something into this model, side by side as
                  on a bundle: the files themselves, or someone else's notes about
                  them. */}
              <Stack direction={{ xs: 'column', sm: 'row' }} spacing={2} sx={{ mb: 2 }}>
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Dropzone
                    label={
                      uploadPct === null
                        ? 'Upload files to this model'
                        : uploadPct < 100
                          ? `Uploading ${uploadPct}%…`
                          : 'Unpacking…'
                    }
                    hint="Straight into this model’s unsorted files · .zip auto-unpacks"
                    multiple
                    busy={uploadPct !== null}
                    progress={uploadPct !== null && uploadPct < 100 ? uploadPct : undefined}
                    onDrop={(drop) => void uploadStraightIn(drop.files)}
                  />
                </Box>
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Dropzone
                    label="Import scraped metadata"
                    hint="Drop a bundle-patch zip — its metadata is applied to this model"
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
          {!editing && model.creator_ref && (
            <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
              Creator ID: {model.creator_ref}
            </Typography>
          )}
          {!editing && model.model_version && (
            <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
              Version: {model.model_version}
            </Typography>
          )}
          {!editing &&
            model.custom_fields.map((entry) => (
              <CustomFieldReadout key={entry.field.id} entry={entry} />
            ))}
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
      <ModelPatchDialog
        modelId={model.id}
        open={patchOpen}
        initialFile={patchFile}
        onApplied={() => {
          // Don't reload here — the dialog still shows its summary, and the
          // reload's navigation would unmount it. Remember it's owed; run on close.
          patchReloadOwed.current = true
          // The in-place editor seeded its fields at mount, so it still shows the
          // pre-patch values; saving them would revert the patch. Leave edit mode.
          setEditing(false)
        }}
        onClose={() => {
          setPatchOpen(false)
          setPatchFile(null)
          if (!patchReloadOwed.current) return
          patchReloadOwed.current = false
          // A patch can rename the model, which moves the slug and so the URL. The
          // old slug no longer resolves, so refetching in place would 404; drop
          // the cache and jump to the stable UUID, and the canonical-slug effect
          // lands us on the new slug with fresh data.
          queryClient.removeQueries({ queryKey: ['model', model.id] })
          navigate(`/models/${model.id}`, { replace: true })
        }}
      />
      <ModelDeleteDialog
        open={deleteOpen}
        onClose={() => setDeleteOpen(false)}
        model={model}
        onDeleted={async () => {
          setDeleteOpen(false)
          await queryClient.invalidateQueries()
          navigate('/')
        }}
      />
      <ModelMergeDialog
        open={mergeOpen}
        onClose={() => setMergeOpen(false)}
        model={model}
        onMerged={async (merged, from) => {
          setMergeOpen(false)
          // The survivor is this model; seed its cache with the merged result,
          // then invalidate broadly — variant lists, file lists and browse
          // counts all shifted, and this model absorbed the other's contents.
          queryClient.setQueryData(['model', id], merged)
          await queryClient.invalidateQueries()
          setToast(`Merged “${from.name}” in`)
        }}
      />
      <ImageLightbox
        open={lightboxOpen}
        srcs={model.images.map((image) => imageUrl(image.id, image.blob_sha256))}
        index={Math.max(
          0,
          model.images.findIndex((image) => image.id === shownImage),
        )}
        alt={model.name}
        onNavigate={(i) => setSelectedImage(model.images[i]?.id ?? null)}
        onClose={() => setLightboxOpen(false)}
      />
      <Snackbar
        open={!!toast}
        autoHideDuration={4000}
        onClose={() => setToast('')}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert
          severity={toast.includes('failed') ? 'error' : 'success'}
          onClose={() => setToast('')}
        >
          {toast}
        </Alert>
      </Snackbar>
    </Container>
  )
}
