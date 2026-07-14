import { useState } from 'react'
import { Alert, Autocomplete, Button, Stack, TextField } from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type ModelDetail } from '../api'

/// The model's fields, edited in place on the page they live on. The dialog that
/// used to do this still exists — it is how a model is *created*, where there is
/// no page to edit yet — but for one that already exists, editing it a modal away
/// from the thing you are editing was always a strange way round.
export default function ModelDetailsEditor({
  model,
  onDone,
}: {
  model: ModelDetail
  /** Left edit mode: saved, or cancelled. */
  onDone: () => void
}) {
  const queryClient = useQueryClient()
  const [name, setName] = useState(model.name)
  const [creatorName, setCreatorName] = useState(model.creator_name ?? '')
  const [tags, setTags] = useState<string[]>(model.tags)
  const [sourceUrl, setSourceUrl] = useState(model.source_url ?? '')
  const [description, setDescription] = useState(model.description_md ?? '')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })

  const save = async () => {
    if (!name.trim()) return setError('A model needs a name')
    setBusy(true)
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
      await api.updateModel(model.id, {
        name: name.trim(),
        creator_id,
        source_url: sourceUrl.trim() || null,
        tags,
      })
      // Descriptions are immutable revisions: an edit inserts a new one, so only
      // write when it actually changed — otherwise every save grows the history
      // with a copy of what was already there.
      if (description !== (model.description_md ?? '')) {
        await api.updateDescription('models', model.id, description)
      }
      await queryClient.invalidateQueries({ queryKey: ['model', model.id] })
      await queryClient.invalidateQueries({ queryKey: ['creators'] })
      await queryClient.invalidateQueries({ queryKey: ['tags'] })
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setBusy(false)
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
      <Autocomplete
        multiple
        freeSolo
        options={(allTags ?? []).map((t) => t.name)}
        value={tags}
        onChange={(_, value) => setTags(value)}
        renderInput={(props) => <TextField {...props} label="Tags" placeholder="add tag…" />}
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
      <Stack direction="row" spacing={1}>
        <Button variant="contained" onClick={save} disabled={busy}>
          Save
        </Button>
        <Button onClick={onDone} disabled={busy}>
          Cancel
        </Button>
      </Stack>
    </Stack>
  )
}
