import { useDeferredValue, useEffect, useState } from 'react'
import {
  Alert,
  Autocomplete,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  Radio,
  RadioGroup,
  TextField,
  Typography,
} from '@mui/material'
import { useQuery } from '@tanstack/react-query'

import { api, type ModelDetail, type ModelSummary, type OtherModelDisposition } from '../api'

/// Pick a model to fold into this one, and say what becomes of it.
///
/// The model being edited survives. The other can be discarded — everything it
/// has moves across and it's deleted — or kept, in which case it stays whole and
/// this model gains a copy of its contents instead (the blobs are shared, so a
/// copy is cheap).
export default function ModelMergeDialog({
  open,
  onClose,
  model,
  onMerged,
}: {
  open: boolean
  onClose: () => void
  model: ModelDetail
  onMerged: (merged: ModelDetail, from: ModelSummary, other: OtherModelDisposition) => void
}) {
  const [from, setFrom] = useState<ModelSummary | null>(null)
  const [query, setQuery] = useState('')
  const [other, setOther] = useState<OtherModelDisposition>('delete')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Reopening starts clean: the model picked last time is not a default, and
  // neither is the disposition chosen for it.
  useEffect(() => {
    if (open) {
      setFrom(null)
      setQuery('')
      setOther('delete')
      setError('')
    }
  }, [open])

  // `useDeferredValue` is the debounce, as on the bundle merge picker: the input
  // stays responsive while the search lags a keystroke behind it.
  const search = useDeferredValue(query.trim())
  const { data: found, isFetching } = useQuery({
    queryKey: ['model-merge-search', search],
    enabled: open,
    queryFn: () =>
      api.searchModels(new URLSearchParams({ q: search, per_page: '20' })).then((r) => r.models),
  })
  // Merging a model into itself is the one thing this can't do.
  const candidates = (found ?? []).filter((m) => m.id !== model.id)

  const confirm = async () => {
    if (!from) return setError('Pick a model to merge in')
    setBusy(true)
    setError('')
    try {
      onMerged(await api.mergeModel(model.id, from.id, other), from, other)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={busy ? undefined : onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Merge another model into this one</DialogTitle>
      <DialogContent>
        {error && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {error}
          </Alert>
        )}
        <Typography sx={{ mb: 2 }}>
          The model you pick is folded into <strong>{model.name}</strong>, which keeps its own name,
          pictures and description.
        </Typography>
        <Autocomplete
          size="small"
          options={candidates}
          loading={isFetching}
          value={from}
          inputValue={query}
          disabled={busy}
          filterOptions={(x) => x}
          getOptionLabel={(m) => (m.creator_name ? `${m.name} — ${m.creator_name}` : m.name)}
          noOptionsText={search ? 'No matching models' : 'Type to search models'}
          onInputChange={(_, value, reason) => {
            if (reason !== 'reset') setQuery(value)
          }}
          onChange={(_, value) => setFrom(value)}
          renderInput={(props) => <TextField {...props} label="Model to merge in" autoFocus />}
          sx={{ mb: 2 }}
        />
        <Typography variant="subtitle2" sx={{ mb: 1 }}>
          And then?
        </Typography>
        <RadioGroup
          value={other}
          onChange={(e) => setOther(e.target.value as OtherModelDisposition)}
        >
          <Option
            value="delete"
            title={from ? `Discard “${from.name}”` : 'Discard the other model'}
            caption="Everything it has moves across — files, variants, pictures, tags, provenance, likes, bundle memberships, and any custom field this model hasn’t answered — and the emptied model is deleted."
          />
          <Option
            value="keep"
            title={from ? `Keep “${from.name}”` : 'Keep the other model'}
            caption="It stays exactly as it is; this model gains a copy of its files, variants and pictures. The blobs are shared, so nothing is duplicated on disk."
          />
        </RadioGroup>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button variant="contained" onClick={confirm} disabled={busy || !from}>
          Merge in
        </Button>
      </DialogActions>
    </Dialog>
  )
}

/// A radio row with a title and an explanatory caption beneath it (as in
/// BundleMergeDialog).
function Option({ value, title, caption }: { value: string; title: string; caption: string }) {
  return (
    <FormControlLabel
      value={value}
      control={<Radio sx={{ alignSelf: 'flex-start', pt: 0.5 }} />}
      sx={{ alignItems: 'flex-start', mb: 1 }}
      label={
        <Box sx={{ py: 0.5 }}>
          <Typography variant="body2">{title}</Typography>
          <Typography variant="caption" color="text.secondary">
            {caption}
          </Typography>
        </Box>
      }
    />
  )
}
