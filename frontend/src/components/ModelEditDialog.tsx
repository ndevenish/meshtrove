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

import { api, uploadWithProgress, type FileRecord, type ModelDetail } from '../api'
import { pasteTags, splitTags } from '../tags'
import Dropzone from './Dropzone'
import type { Drop } from '../upload'

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
  const [tags, setTags] = useState<string[]>([])
  const [description, setDescription] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [drop, setDrop] = useState<Drop | null>(null)
  const [uploadPct, setUploadPct] = useState(0)

  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })

  useEffect(() => {
    if (open) {
      setName(model?.name ?? '')
      setCreatorName(model?.creator_name ?? '')
      setSourceUrl(model?.source_url ?? '')
      setTags(model?.tags ?? [])
      setDescription(model?.description_md ?? '')
      setError('')
      setDrop(null)
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
        tags,
        description_md: model ? undefined : description || null,
      }
      let saved: ModelDetail
      if (model) {
        saved = await api.updateModel(model.id, body)
        if (description !== (model.description_md ?? '')) {
          await api.updateDescription('models', model.id, description)
        }
      } else {
        saved = await api.createModel(body)
        // File-first: dropped files land in the model's unsorted bucket, keeping
        // their folders; an archive among them unpacks in the background, and the
        // model page shows that progress.
        if (drop) {
          const form = new FormData()
          for (const { file, path } of drop.files) {
            form.append('path', path) // applies to the file part that follows
            form.append('file', file)
          }
          await uploadWithProgress<FileRecord[]>(`/api/models/${saved.id}/files`, form, (f) =>
            setUploadPct(Math.round(f * 100)),
          )
        }
      }
      await queryClient.invalidateQueries()
      onClose()
      navigate(`/models/${saved.slug}`)
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
              label={
                busy && drop
                  ? uploadPct < 100
                    ? `Uploading ${uploadPct}%…`
                    : 'Unpacking…'
                  : drop
                    ? drop.files.length === 1
                      ? drop.files[0].file.name
                      : `${drop.name} — ${drop.files.length} files`
                    : 'Drop an archive or folder to import'
              }
              hint={drop ? 'Uploads after Create' : '.zip auto-unpacks · or click to browse'}
              busy={busy && !!drop}
              progress={busy && drop && uploadPct < 100 ? uploadPct : undefined}
              onDrop={(dropped) => {
                setDrop(dropped)
                if (!name.trim()) setName(dropped.name)
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
            onChange={(_, value) => setTags(splitTags(value))}
            renderInput={(params) => (
              <TextField
                {...params}
                onPaste={pasteTags(tags, setTags)}
                label="Tags"
                placeholder="add tag…"
              />
            )}
          />
          <TextField
            label="Source URL"
            value={sourceUrl}
            onChange={(e) => setSourceUrl(e.target.value)}
          />
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
