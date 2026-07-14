import { useState } from 'react'
import { useParams } from 'react-router-dom'
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
  Autocomplete,
  TextField,
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

import { api, imageUrl, uploadWithProgress, type FileRecord } from '../api'
import { useAuth } from '../main'
import { usePasteImage } from '../usePasteImage'
import ModelCard from '../components/ModelCard'
import BundleEditDialog from '../components/BundleEditDialog'
import BundleUnsortedSection from '../components/BundleUnsortedSection'
import Dropzone from '../components/Dropzone'
import DescriptionHistoryDialog from '../components/DescriptionHistoryDialog'

export default function BundlePage() {
  const { id } = useParams<{ id: string }>()
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const [editOpen, setEditOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [selectedImage, setSelectedImage] = useState<string | null>(null)
  const [uploading, setUploading] = useState(false)
  const [uploadPct, setUploadPct] = useState(0)
  const [toast, setToast] = useState('')

  const { data: bundle } = useQuery({
    queryKey: ['bundle', id],
    queryFn: () => api.bundle(id!),
    enabled: !!id,
  })

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
            {canEdit && (
              <Button startIcon={<EditIcon />} onClick={() => setEditOpen(true)}>
                Edit
              </Button>
            )}
          </Stack>
          {bundle.creator_name && (
            <Typography color="text.secondary" sx={{ mb: 1 }}>
              by {bundle.creator_name}
            </Typography>
          )}
          <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap', gap: 1, mb: 2 }}>
            {bundle.tags.map((tag) => (
              <Chip key={tag} label={tag} size="small" />
            ))}
          </Stack>

          {bundle.source_url && (
            <Paper variant="outlined" sx={{ p: 1.5, mb: 2 }}>
              <Typography variant="body2">
                <a href={bundle.source_url} target="_blank" rel="noreferrer">
                  Source page
                </a>
              </Typography>
            </Paper>
          )}

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

          <Divider sx={{ mb: 2 }} />
          {canEdit && (
            <Box sx={{ mb: 2 }}>
              <Dropzone
                label={
                  uploading
                    ? uploadPct < 100
                      ? `Uploading ${uploadPct}%…`
                      : 'Unpacking…'
                    : 'Drop an archive to add its contents'
                }
                hint=".zip unpacks into this bundle, then sort into member models"
                accept=".zip"
                busy={uploading}
                progress={uploading && uploadPct < 100 ? uploadPct : undefined}
                onFiles={async (droppedFiles) => {
                  setUploading(true)
                  setUploadPct(0)
                  try {
                    const form = new FormData()
                    form.append('file', droppedFiles[0])
                    await uploadWithProgress<FileRecord[]>(
                      `/api/bundles/${bundle.id}/files`,
                      form,
                      (f) => setUploadPct(Math.round(f * 100)),
                    )
                    await queryClient.invalidateQueries({ queryKey: ['bundle-files', bundle.id] })
                  } finally {
                    setUploading(false)
                  }
                }}
              />
            </Box>
          )}
          <BundleUnsortedSection bundle={bundle} canEdit={!!canEdit} onChange={refresh} />
          <MembersSection
            bundleId={bundle.id}
            models={bundle.models}
            canEdit={!!canEdit}
            onChange={refresh}
          />
        </Box>
      </Stack>

      <BundleEditDialog open={editOpen} onClose={() => setEditOpen(false)} bundle={bundle} />
      <DescriptionHistoryDialog
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        owner="bundles"
        entity={bundle}
        canEdit={!!canEdit}
        onChange={refresh}
      />
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

function MembersSection({
  bundleId,
  models,
  canEdit,
  onChange,
}: {
  bundleId: string
  models: import('../api').ModelSummary[]
  canEdit: boolean
  onChange: () => void
}) {
  const queryClient = useQueryClient()
  const [search, setSearch] = useState('')
  const { data: candidates } = useQuery({
    queryKey: ['model-search', search],
    queryFn: () => api.searchModels(new URLSearchParams({ q: search, per_page: '10' })),
    enabled: canEdit && search.trim().length > 0,
  })

  const refreshAll = async () => {
    await queryClient.invalidateQueries({ queryKey: ['bundle', bundleId] })
    onChange()
  }

  const memberIds = new Set(models.map((m) => m.id))
  const options = (candidates?.models ?? []).filter((m) => !memberIds.has(m.id))

  return (
    <Box>
      <Stack direction="row" sx={{ alignItems: 'center', mb: 1 }} spacing={1}>
        <Typography variant="h6">Models</Typography>
        <Typography variant="body2" color="text.secondary">
          {models.length}
        </Typography>
      </Stack>

      {canEdit && (
        <Autocomplete
          sx={{ mb: 2, maxWidth: 420 }}
          options={options}
          getOptionLabel={(m) => m.name}
          filterOptions={(x) => x}
          onInputChange={(_, value) => setSearch(value)}
          onChange={async (_, value) => {
            if (value) {
              await api.addModelToBundle(bundleId, value.id)
              setSearch('')
              await refreshAll()
            }
          }}
          renderInput={(params) => (
            <TextField {...params} size="small" label="Add an existing model…" />
          )}
        />
      )}

      {models.length === 0 ? (
        <Typography color="text.secondary" variant="body2">
          No models in this bundle yet
          {canEdit ? ' — add one above, or drop an archive to unpack and sort' : ''}.
        </Typography>
      ) : (
        <Box
          sx={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))',
            gap: 2,
          }}
        >
          {models.map((model) => (
            <Box key={model.id} sx={{ position: 'relative' }}>
              <ModelCard model={model} />
              {canEdit && (
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
