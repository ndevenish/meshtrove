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

import { api, type BundleDetail, type BundleSummary, type OtherBundleDisposition } from '../api'

/// Pick a bundle to absorb into this one, and say what becomes of it.
///
/// Membership is many-to-many, so "merged" has two honest meanings and the
/// dialog makes you choose: the other bundle can stay standing with its models
/// now in both places, or it can empty itself into this one and go.
export default function BundleMergeDialog({
  open,
  onClose,
  bundle,
  onMerged,
}: {
  open: boolean
  onClose: () => void
  bundle: BundleDetail
  onMerged: (merged: BundleDetail, from: BundleSummary, other: OtherBundleDisposition) => void
}) {
  const [from, setFrom] = useState<BundleSummary | null>(null)
  const [query, setQuery] = useState('')
  const [other, setOther] = useState<OtherBundleDisposition>('delete')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Reopening starts clean: the bundle picked last time is not a default, and
  // neither is the disposition chosen for it.
  useEffect(() => {
    if (open) {
      setFrom(null)
      setQuery('')
      setOther('delete')
      setError('')
    }
  }, [open])

  // `useDeferredValue` is the debounce, as on the member picker: the input stays
  // responsive while the search lags a keystroke behind it.
  const search = useDeferredValue(query.trim())
  const { data: found, isFetching } = useQuery({
    queryKey: ['bundle-search', search],
    enabled: open,
    queryFn: () =>
      api.searchBundles(new URLSearchParams({ q: search, per_page: '20' })).then((r) => r.bundles),
  })
  // Merging a bundle into itself is the one thing this can't do.
  const candidates = (found ?? []).filter((b) => b.id !== bundle.id)

  const confirm = async () => {
    if (!from) return setError('Pick a bundle to merge in')
    setBusy(true)
    setError('')
    try {
      onMerged(await api.mergeBundle(bundle.id, from.id, other), from, other)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={busy ? undefined : onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Merge another bundle into this one</DialogTitle>
      <DialogContent>
        {error && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {error}
          </Alert>
        )}
        <Typography sx={{ mb: 2 }}>
          Every model in the bundle you pick becomes a member of <strong>{bundle.name}</strong>.
        </Typography>
        <Autocomplete
          size="small"
          options={candidates}
          loading={isFetching}
          value={from}
          inputValue={query}
          disabled={busy}
          filterOptions={(x) => x}
          getOptionLabel={(b) => `${b.name} (${b.model_count})`}
          noOptionsText={search ? 'No matching bundles' : 'Type to search bundles'}
          onInputChange={(_, value, reason) => {
            if (reason !== 'reset') setQuery(value)
          }}
          onChange={(_, value) => setFrom(value)}
          renderInput={(props) => <TextField {...props} label="Bundle to merge in" autoFocus />}
          sx={{ mb: 2 }}
        />
        <Typography variant="subtitle2" sx={{ mb: 1 }}>
          And then?
        </Typography>
        <RadioGroup
          value={other}
          onChange={(e) => setOther(e.target.value as OtherBundleDisposition)}
        >
          <Option
            value="delete"
            title={from ? `Delete “${from.name}”` : 'Delete the other bundle'}
            caption="Everything it has comes across — loose files, pictures, provenance, tags, categories, and any custom field this bundle hasn’t answered — and the emptied bundle goes."
          />
          <Option
            value="keep"
            title={from ? `Keep “${from.name}”` : 'Keep the other bundle'}
            caption="It stays exactly as it is; its models simply belong to both bundles from now on. Nothing else moves."
          />
        </RadioGroup>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button variant="contained" onClick={confirm} disabled={busy || !from}>
          {from
            ? `Merge ${from.model_count} model${from.model_count === 1 ? '' : 's'} in`
            : 'Merge'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}

/// A radio row with a title and an explanatory caption beneath it (as in
/// BundleDeleteDialog).
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
