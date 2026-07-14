import { useState } from 'react'
import { useParams, Link, useNavigate } from 'react-router-dom'
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
import Inventory2Icon from '@mui/icons-material/Inventory2'
import ReactMarkdown from 'react-markdown'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, imageUrl } from '../api'
import { useAuth } from '../main'
import { usePasteImage } from '../usePasteImage'
import ModelEditDialog from '../components/ModelEditDialog'
import VariantSection from '../components/VariantSection'
import UnsortedSection from '../components/UnsortedSection'
import DescriptionHistoryDialog from '../components/DescriptionHistoryDialog'

export default function ModelPage() {
  const { id } = useParams<{ id: string }>()
  const { user } = useAuth()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [editOpen, setEditOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [selectedImage, setSelectedImage] = useState<string | null>(null)
  const [toast, setToast] = useState('')
  const [promoting, setPromoting] = useState(false)

  const { data: model } = useQuery({
    queryKey: ['model', id],
    queryFn: () => api.model(id!),
    enabled: !!id,
  })

  const canEditModel =
    !!model &&
    !!user &&
    (user.role === 'admin' || (user.role === 'editor' && user.id === model.created_by))
  usePasteImage(canEditModel, 'models', id ?? '', {
    onUploaded: () => {
      void queryClient.invalidateQueries({ queryKey: ['model', id] })
      setToast('Image added from clipboard')
    },
    onError: (m) => setToast(`Paste failed: ${m}`),
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
                alt={model.name}
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
            {model.images.map((image) => (
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
            <Typography variant="h4" sx={{ fontWeight: 700, flexGrow: 1 }}>
              {model.name}
            </Typography>
            {canEdit && (
              <Stack direction="row" spacing={1}>
                {model.bundles.length === 0 && (
                  <Button
                    startIcon={<Inventory2Icon />}
                    disabled={promoting}
                    onClick={async () => {
                      setPromoting(true)
                      try {
                        const bundle = await api.createBundle({
                          name: model.name,
                          kind: 'collection',
                          creator_id: model.creator_id,
                        })
                        await api.addModelToBundle(bundle.id, model.id)
                        await queryClient.invalidateQueries()
                        navigate(`/bundles/${bundle.id}`)
                      } finally {
                        setPromoting(false)
                      }
                    }}
                  >
                    Promote to bundle
                  </Button>
                )}
                <Button startIcon={<EditIcon />} onClick={() => setEditOpen(true)}>
                  Edit
                </Button>
              </Stack>
            )}
          </Stack>
          {model.creator_name && (
            <Typography color="text.secondary" sx={{ mb: 1 }}>
              by{' '}
              <Link to={`/creators?q=${encodeURIComponent(model.creator_name)}`}>
                {model.creator_name}
              </Link>
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
                  to={`/bundles/${b.id}`}
                  clickable
                />
              ))}
            </Stack>
          )}
          <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap', gap: 1, mb: 2 }}>
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

          {(model.license || model.purchase_price != null || model.source_url) && (
            <Paper variant="outlined" sx={{ p: 1.5, mb: 2 }}>
              <Stack direction="row" spacing={3} sx={{ flexWrap: 'wrap', gap: 1 }}>
                {model.source_url && (
                  <Typography variant="body2">
                    <a href={model.source_url} target="_blank" rel="noreferrer">
                      Source page
                    </a>
                  </Typography>
                )}
                {model.license && <Typography variant="body2">License: {model.license}</Typography>}
                {model.purchase_price != null && (
                  <Typography variant="body2">Purchased: {model.purchase_price}</Typography>
                )}
              </Stack>
            </Paper>
          )}

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

          <Divider sx={{ mb: 2 }} />
          <UnsortedSection model={model} canEdit={!!canEdit} onChange={refresh} />
          <VariantSection model={model} canEdit={!!canEdit} onChange={refresh} />
        </Box>
      </Stack>

      <ModelEditDialog open={editOpen} onClose={() => setEditOpen(false)} model={model} />
      <DescriptionHistoryDialog
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        owner="models"
        entity={model}
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
