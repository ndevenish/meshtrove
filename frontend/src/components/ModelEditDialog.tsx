import { useEffect, useState } from 'react'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  TextField,
  Stack,
  Autocomplete,
  Alert,
} from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'

import { api, type ModelDetail } from '../api'
import Dropzone from './Dropzone'
import { deriveModelName } from '../upload'

/// Create (no `model`) or edit (with `model`) a model's metadata and tags.
export default function ModelEditDialog({
  open,
  onClose,
  model,
}: {
  open: boolean
  onClose: () => void
  model?: ModelDetail
}) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const [creatorName, setCreatorName] = useState('')
  const [sourceUrl, setSourceUrl] = useState('')
  const [license, setLicense] = useState('')
  const [price, setPrice] = useState('')
  const [tags, setTags] = useState<string[]>([])
  const [description, setDescription] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [archive, setArchive] = useState<File | null>(null)

  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })

  useEffect(() => {
    if (open) {
      setName(model?.name ?? '')
      setCreatorName(model?.creator_name ?? '')
      setSourceUrl(model?.source_url ?? '')
      setLicense(model?.license ?? '')
      setPrice(model?.purchase_price != null ? String(model.purchase_price) : '')
      setTags(model?.tags ?? [])
      setDescription(model?.description_md ?? '')
      setError('')
      setArchive(null)
    }
  }, [open, model])

  const submit = async () => {
    setBusy(true)
    setError('')
    try {
      // Resolve or create the creator by name.
      let creator_id: string | null = model?.creator_id ?? null
      const trimmed = creatorName.trim()
      if (!trimmed) {
        creator_id = null
      } else {
        const existing = creators?.find((c) => c.name.toLowerCase() === trimmed.toLowerCase())
        creator_id = existing ? existing.id : (await api.createCreator({ name: trimmed })).id
      }

      const body = {
        name,
        creator_id,
        source_url: sourceUrl || null,
        license: license || null,
        purchase_price: price ? Number(price) : null,
        tags,
        description_md: model ? undefined : description || null,
      }
      let saved: ModelDetail
      if (model) {
        saved = await api.updateModel(model.id, body)
        if (description !== (model.description_md ?? '')) {
          await api.updateDescription(model.id, description)
        }
      } else {
        saved = await api.createModel(body)
        // File-first: a dropped archive unpacks into the model's unsorted
        // bucket in the background; the model page shows unpack progress.
        if (archive) {
          const form = new FormData()
          form.append('file', archive)
          await api.uploadModelFiles(saved.id, form)
        }
      }
      await queryClient.invalidateQueries()
      onClose()
      navigate(`/models/${saved.id}`)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>{model ? 'Edit model' : 'New model'}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          {!model && (
            <Dropzone
              label={archive ? archive.name : 'Drop an archive to import'}
              hint={archive ? 'Will unpack after Create' : '.zip auto-unpacks · or click to browse'}
              accept=".zip"
              onFiles={(files) => {
                const file = files[0]
                setArchive(file)
                if (!name.trim()) setName(deriveModelName(file.name))
              }}
            />
          )}
          <TextField
            label="Name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
            required
          />
          <Autocomplete
            freeSolo
            options={(creators ?? []).map((c) => c.name)}
            value={creatorName}
            onInputChange={(_, value) => setCreatorName(value)}
            renderInput={(params) => (
              <TextField {...params} label="Creator (author / company / site)" />
            )}
          />
          <Autocomplete
            multiple
            freeSolo
            options={(allTags ?? []).map((t) => t.name)}
            value={tags}
            onChange={(_, value) => setTags(value)}
            renderInput={(params) => <TextField {...params} label="Tags" placeholder="add tag…" />}
          />
          <TextField
            label="Source URL"
            value={sourceUrl}
            onChange={(e) => setSourceUrl(e.target.value)}
          />
          <Stack direction="row" spacing={2}>
            <TextField
              label="License"
              value={license}
              onChange={(e) => setLicense(e.target.value)}
              fullWidth
            />
            <TextField
              label="Purchase price"
              value={price}
              onChange={(e) => setPrice(e.target.value)}
              type="number"
              fullWidth
            />
          </Stack>
          <TextField
            label="Description (markdown)"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            multiline
            minRows={4}
          />
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" onClick={submit} disabled={busy || !name.trim()}>
          {model ? 'Save' : 'Create'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
