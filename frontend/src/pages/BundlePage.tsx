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

import { api, imageUrl } from '../api'
import { useAuth } from '../main'
import { usePasteImage } from '../usePasteImage'
import ModelCard from '../components/ModelCard'
import BundleDetailsEditor from '../components/BundleDetailsEditor'
import BundleUnsortedSection from '../components/BundleUnsortedSection'
import DescriptionHistoryDialog from '../components/DescriptionHistoryDialog'
import ImportErrorDialog from '../components/ImportErrorDialog'

export default function BundlePage() {
  const { id } = useParams<{ id: string }>()
  const { user } = useAuth()
  const queryClient = useQueryClient()
  // Edit *mode*: the fields become editable in place, and the destructive
  // controls — remove a model from the bundle, delete a file, delete an image —
  // appear only here.
  const [editing, setEditing] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [selectedImage, setSelectedImage] = useState<string | null>(null)
  const [uploadError, setUploadError] = useState('')
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
          </Stack>

          {editing && (
            <BundleDetailsEditor key={bundle.id} bundle={bundle} onDone={() => setEditing(false)} />
          )}
          {!editing && bundle.creator_name && (
            <Typography color="text.secondary" sx={{ mb: 1 }}>
              by {bundle.creator_name}
            </Typography>
          )}
          <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap', gap: 1, mb: 2 }}>
            {!editing && bundle.tags.map((tag) => <Chip key={tag} label={tag} size="small" />)}
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

          <Divider sx={{ mb: 2 }} />
          <BundleUnsortedSection
            bundle={bundle}
            canEdit={!!canEdit}
            editing={editing}
            onChange={refresh}
          />
          <MembersSection
            bundleId={bundle.id}
            models={bundle.models}
            canEdit={!!canEdit}
            editing={editing}
            onChange={refresh}
          />
        </Box>
      </Stack>

      <DescriptionHistoryDialog
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        owner="bundles"
        entity={bundle}
        canEdit={!!canEdit}
        onChange={refresh}
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
  models,
  canEdit,
  editing,
  onChange,
}: {
  bundleId: string
  models: import('../api').ModelSummary[]
  canEdit: boolean
  /** Edit mode: only here can a model be pulled out of the bundle. */
  editing: boolean
  onChange: () => void
}) {
  const queryClient = useQueryClient()

  const refreshAll = async () => {
    await queryClient.invalidateQueries({ queryKey: ['bundle', bundleId] })
    onChange()
  }

  return (
    <Box>
      <Stack direction="row" sx={{ alignItems: 'center', mb: 1 }} spacing={1}>
        <Typography variant="h6">Models</Typography>
        <Typography variant="body2" color="text.secondary">
          {models.length}
        </Typography>
      </Stack>

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
          {models.map((model) => (
            <Box key={model.id} sx={{ position: 'relative' }}>
              <ModelCard model={model} />
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
