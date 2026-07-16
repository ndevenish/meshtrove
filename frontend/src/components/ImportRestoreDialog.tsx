import { useState } from 'react'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  Typography,
  Stack,
  Alert,
  Chip,
  Checkbox,
  FormControlLabel,
  Divider,
  CircularProgress,
  Box,
} from '@mui/material'
import Inventory2Icon from '@mui/icons-material/Inventory2'
import ViewInArIcon from '@mui/icons-material/ViewInAr'

import {
  api,
  formatBytes,
  type ImportArchivePreview,
  type ImportArchiveEntity,
  type RestoreSummary,
} from '../api'
import Dropzone from './Dropzone'

type Phase = 'pick' | 'uploading' | 'preview' | 'committing' | 'done'

/// Restore an export archive: upload → preview what it holds (flagging entities
/// already present) → confirm. Existing entities are skipped unless ticked for a
/// fresh copy; new ones always come in.
export default function ImportRestoreDialog({
  open,
  onClose,
  onImported,
}: {
  open: boolean
  onClose: () => void
  /** fired after a successful restore, so the caller can refresh its data */
  onImported?: () => void
}) {
  const [phase, setPhase] = useState<Phase>('pick')
  const [progress, setProgress] = useState(0)
  const [error, setError] = useState('')
  const [preview, setPreview] = useState<ImportArchivePreview | null>(null)
  const [fresh, setFresh] = useState<Set<string>>(new Set())
  const [summary, setSummary] = useState<RestoreSummary | null>(null)

  const reset = () => {
    setPhase('pick')
    setProgress(0)
    setError('')
    setPreview(null)
    setFresh(new Set())
    setSummary(null)
  }

  const close = () => {
    onClose()
    // Let the dialog finish animating out before wiping its contents.
    setTimeout(reset, 200)
  }

  const upload = async (file: File) => {
    setError('')
    setProgress(0)
    setPhase('uploading')
    const form = new FormData()
    form.append('file', file)
    try {
      const result = await api.previewImportArchive(form, (f) => setProgress(f))
      setPreview(result)
      setFresh(new Set())
      setPhase('preview')
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setPhase('pick')
    }
  }

  const toggleFresh = (id: string) => {
    setFresh((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const commit = async () => {
    if (!preview) return
    setError('')
    setPhase('committing')
    try {
      const s = await api.commitImportArchive(preview.token, [...fresh])
      setSummary(s)
      setPhase('done')
      onImported?.()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setPhase('preview')
    }
  }

  return (
    <Dialog open={open} onClose={close} maxWidth="sm" fullWidth>
      <DialogTitle>Import archive</DialogTitle>
      <DialogContent>
        {error && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {error}
          </Alert>
        )}

        {(phase === 'pick' || phase === 'uploading') && (
          <>
            <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
              Drop a MeshTrove export (<code>.zip</code>) to restore the models and bundles it
              holds. You&rsquo;ll see what&rsquo;s inside before anything is written.
            </Typography>
            <Dropzone
              label="Drop an export archive"
              hint="or click to browse — .zip"
              accept=".zip,application/zip"
              busy={phase === 'uploading'}
              progress={Math.round(progress * 100)}
              onDrop={(drop) => {
                const file = drop.files[0]?.file
                if (file) void upload(file)
              }}
            />
          </>
        )}

        {phase === 'preview' && preview && (
          <Stack spacing={2}>
            <Typography variant="body2" color="text.secondary">
              Exported {new Date(preview.exported_at).toLocaleString()} — {preview.blob_count}{' '}
              file(s), {formatBytes(preview.total_size)}. Entities already here are skipped unless
              you ask for a fresh copy.
            </Typography>
            {preview.models.length > 0 && (
              <EntityList
                title="Models"
                icon={<ViewInArIcon fontSize="small" />}
                entities={preview.models}
                fresh={fresh}
                onToggle={toggleFresh}
              />
            )}
            {preview.bundles.length > 0 && (
              <EntityList
                title="Bundles"
                icon={<Inventory2Icon fontSize="small" />}
                entities={preview.bundles}
                fresh={fresh}
                onToggle={toggleFresh}
              />
            )}
          </Stack>
        )}

        {phase === 'committing' && (
          <Stack spacing={2} sx={{ alignItems: 'center', py: 4 }}>
            <CircularProgress />
            <Typography color="text.secondary">Restoring…</Typography>
          </Stack>
        )}

        {phase === 'done' && summary && (
          <Alert severity="success">
            Restored {summary.models_created} model(s) and {summary.bundles_created} bundle(s) —{' '}
            {summary.files} file(s), {summary.images} image(s).
            {(summary.models_skipped > 0 || summary.bundles_skipped > 0) && (
              <>
                {' '}
                Skipped {summary.models_skipped} existing model(s) and {summary.bundles_skipped}{' '}
                bundle(s).
              </>
            )}
          </Alert>
        )}
      </DialogContent>
      <DialogActions>
        {phase === 'done' ? (
          <Button onClick={close} variant="contained">
            Done
          </Button>
        ) : (
          <>
            <Button onClick={close} disabled={phase === 'committing'}>
              Cancel
            </Button>
            {phase === 'preview' && preview && (
              <Button
                onClick={() => void commit()}
                variant="contained"
                disabled={preview.models.length === 0 && preview.bundles.length === 0}
              >
                Confirm restore
              </Button>
            )}
          </>
        )}
      </DialogActions>
    </Dialog>
  )
}

function EntityList({
  title,
  icon,
  entities,
  fresh,
  onToggle,
}: {
  title: string
  icon: React.ReactNode
  entities: ImportArchiveEntity[]
  fresh: Set<string>
  onToggle: (id: string) => void
}) {
  return (
    <Box>
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 0.5 }}>
        {icon}
        <Typography variant="subtitle2">
          {title} ({entities.length})
        </Typography>
      </Stack>
      <Divider sx={{ mb: 1 }} />
      <Stack spacing={0.5}>
        {entities.map((e) => (
          <Stack
            key={e.id}
            direction="row"
            spacing={1}
            sx={{ alignItems: 'center', flexWrap: 'wrap' }}
          >
            <Typography sx={{ flexGrow: 1, minWidth: 0 }} noWrap>
              {e.name}
              {e.members !== undefined && (
                <Typography component="span" variant="body2" color="text.secondary">
                  {' '}
                  · {e.members} model(s)
                </Typography>
              )}
            </Typography>
            {e.exists ? (
              <>
                <Chip label="already here" size="small" color="warning" variant="outlined" />
                <FormControlLabel
                  sx={{ mr: 0 }}
                  control={
                    <Checkbox
                      size="small"
                      checked={fresh.has(e.id)}
                      onChange={() => onToggle(e.id)}
                    />
                  }
                  label={<Typography variant="body2">fresh copy</Typography>}
                />
              </>
            ) : (
              <Chip label="new" size="small" color="success" variant="outlined" />
            )}
          </Stack>
        ))}
      </Stack>
    </Box>
  )
}
