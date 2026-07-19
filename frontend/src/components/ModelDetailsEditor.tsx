import { forwardRef, useImperativeHandle, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Alert, Autocomplete, Stack, TextField } from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, uploadWithProgress, type FileRecord, type ModelDetail } from '../api'
import { changeTags, pasteTags } from '../tags'
import { useSuppressGlobalDrop } from '../globalDrop'
import Dropzone from './Dropzone'

/// What the page can ask of the editor. Save and Cancel live in the page header,
/// where the Edit button was — leaving the mode should be where entering it was,
/// not buried at the bottom of a form — so the buttons are up there and the state
/// is down here, and this is the seam between them.
export type DetailsEditorHandle = {
  save: () => Promise<void>
}

/// The model's fields, edited in place on the page they live on. The dialog that
/// used to do this still exists — it is how a model is *created*, where there is
/// no page to edit yet — but for one that already exists, editing it a modal away
/// from the thing you are editing was always a strange way round.
const ModelDetailsEditor = forwardRef<
  DetailsEditorHandle,
  {
    model: ModelDetail
    /** Saved, or cancelled: either way, edit mode is over. */
    onDone: () => void
    onBusyChange?: (busy: boolean) => void
  }
>(function ModelDetailsEditor({ model, onDone, onBusyChange }, ref) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  // This editor is only ever mounted in edit mode, and it carries the "Upload
  // files to this model" box — so while it is up, the app-wide drop overlay must
  // stand aside or it swallows the drop and stages an import instead.
  useSuppressGlobalDrop()
  const [name, setName] = useState(model.name)
  const [creatorName, setCreatorName] = useState(model.creator_name ?? '')
  const [creatorRef, setCreatorRef] = useState(model.creator_ref ?? '')
  const [tags, setTags] = useState<string[]>(model.tags)
  const [sourceUrl, setSourceUrl] = useState(model.source_url ?? '')
  const [description, setDescription] = useState(model.description_md ?? '')
  const [error, setError] = useState('')
  const [uploadPct, setUploadPct] = useState<number | null>(null)

  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })

  const busy = (value: boolean) => onBusyChange?.(value)

  const save = async () => {
    if (!name.trim()) {
      setError('A model needs a name')
      throw new Error('A model needs a name')
    }
    busy(true)
    setError('')
    try {
      // A creator typed by hand may not exist yet — get-or-create, so the library
      // ends up with one row per creator rather than one per spelling.
      const typed = creatorName.trim()
      let creator_id: string | null = null
      if (typed) {
        const existing = (creators ?? []).find((c) => c.name.toLowerCase() === typed.toLowerCase())
        creator_id = existing ? existing.id : (await api.createCreator({ name: typed })).id
      }
      const saved = await api.updateModel(model.id, {
        name: name.trim(),
        creator_id,
        creator_ref: creatorRef.trim() || null,
        source_url: sourceUrl.trim() || null,
        tags,
      })
      // Descriptions are immutable revisions: an edit inserts a new one, so only
      // write when it actually changed — otherwise every save grows the history
      // with a copy of what was already there.
      if (description !== (model.description_md ?? '')) {
        await api.updateDescription('models', model.id, description)
      }
      // A rename moves the slug, and with it the URL: go to the canonical slug
      // and refetch there. The old slug in the address bar no longer resolves,
      // so we must navigate rather than refetch it. When the slug is unchanged
      // this is a no-op navigation and the invalidate just refreshes in place.
      await queryClient.invalidateQueries({ queryKey: ['model', saved.slug] })
      await queryClient.invalidateQueries({ queryKey: ['creators'] })
      await queryClient.invalidateQueries({ queryKey: ['tags'] })
      navigate(`/models/${saved.slug}`, { replace: true })
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      throw err
    } finally {
      busy(false)
    }
  }

  useImperativeHandle(ref, () => ({ save }))

  /// Files dropped here go straight onto *this* model — no import to stage and
  /// commit, because the question an import exists to ask ("model or bundle?") is
  /// already answered: you are standing on the model. They land in its unsorted
  /// bucket with their folders intact, and a .zip unpacks in the background.
  const uploadStraightIn = async (files: { file: File; path: string }[]) => {
    setUploadPct(0)
    setError('')
    try {
      const form = new FormData()
      for (const { file, path } of files) {
        form.append('path', path) // applies to the file part that follows
        form.append('file', file)
      }
      await uploadWithProgress<FileRecord[]>(`/api/models/${model.id}/files`, form, (f) =>
        setUploadPct(Math.round(f * 100)),
      )
      await queryClient.invalidateQueries({ queryKey: ['model', model.id] })
      await queryClient.invalidateQueries({ queryKey: ['model-files', model.id] })
      await queryClient.invalidateQueries({ queryKey: ['jobs', 'all'] })
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setUploadPct(null)
    }
  }

  return (
    <Stack spacing={2} sx={{ mb: 2 }}>
      {error && <Alert severity="error">{error}</Alert>}
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
        renderInput={(props) => <TextField {...props} label="Creator (author / company / site)" />}
      />
      <TextField
        label="Creator ID"
        value={creatorRef}
        onChange={(e) => setCreatorRef(e.target.value)}
        placeholder="the creator's own id / SKU for this model"
      />
      <Autocomplete
        multiple
        freeSolo
        options={(allTags ?? []).map((t) => t.name)}
        value={tags}
        onChange={changeTags(setTags)}
        renderInput={(props) => (
          <TextField
            {...props}
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
    </Stack>
  )
})

export default ModelDetailsEditor
