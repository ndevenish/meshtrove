import { useState } from 'react'
import {
  Alert,
  Box,
  Button,
  LinearProgress,
  Paper,
  Stack,
  Tooltip,
  Typography,
} from '@mui/material'
import { useQuery } from '@tanstack/react-query'

import { api, formatBytes, type CompressionReport } from '../api'

/// Admin-only: how full the disk under the store is, and — on request — how
/// much the filesystem is compressing the blobs. The compression figure is
/// behind a button because it stats every blob in the store.
export default function StoragePanel() {
  const [compression, setCompression] = useState<CompressionReport | null>(null)
  const [measuring, setMeasuring] = useState(false)
  const [error, setError] = useState('')

  // Disk fills while imports run, so this is worth refreshing on its own.
  const { data } = useQuery({
    queryKey: ['storage'],
    queryFn: () => api.storage(),
    refetchInterval: 60_000,
  })

  const measure = async () => {
    setError('')
    setMeasuring(true)
    try {
      setCompression(await api.storageCompression())
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setMeasuring(false)
    }
  }

  if (!data) return null

  const usedPercent = data.total_bytes > 0 ? (data.used_bytes / data.total_bytes) * 100 : 0
  // Amber approaching full, red when an import would be at real risk.
  const severity = usedPercent >= 95 ? 'error' : usedPercent >= 85 ? 'warning' : 'primary'
  const saved = compression ? compression.apparent_bytes - compression.allocated_bytes : 0

  return (
    <Paper variant="outlined" sx={{ p: 3, mt: 3 }}>
      <Typography variant="h6" sx={{ mb: 0.5 }}>
        Filesystem
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Space on the volume holding the blob store, <code>{data.path}</code>.
      </Typography>

      <Box sx={{ mb: 1 }}>
        <LinearProgress
          variant="determinate"
          value={Math.min(usedPercent, 100)}
          color={severity}
          sx={{ height: 10, borderRadius: 5 }}
        />
      </Box>
      <Stack direction="row" spacing={3} sx={{ mb: 2, flexWrap: 'wrap' }}>
        <Stat label="Used" value={`${formatBytes(data.used_bytes)} (${usedPercent.toFixed(1)}%)`} />
        <Stat label="Available" value={formatBytes(data.available_bytes)} />
        <Stat label="Capacity" value={formatBytes(data.total_bytes)} />
        <Stat
          label="Stored blobs"
          value={`${data.blob_count.toLocaleString()} — ${formatBytes(data.blob_bytes)}`}
          hint="Total logical size of the blobs this archive has recorded, before any filesystem compression. Anything on the volume beyond this belongs to something else."
        />
      </Stack>

      {usedPercent >= 85 && (
        <Alert severity={usedPercent >= 95 ? 'error' : 'warning'} sx={{ mb: 2 }}>
          {formatBytes(data.available_bytes)} left. Large imports write the whole archive before
          unpacking it, so they need room for both.
        </Alert>
      )}
      {error && (
        <Alert severity="error" sx={{ mb: 2 }}>
          {error}
        </Alert>
      )}

      {compression && (
        <Alert severity="info" sx={{ mb: 2 }}>
          {compression.ratio && compression.ratio > 1.01 ? (
            <>
              Compressing <strong>{compression.ratio.toFixed(2)}x</strong> —{' '}
              {formatBytes(compression.apparent_bytes)} of blobs occupying{' '}
              {formatBytes(compression.allocated_bytes)} on disk, saving {formatBytes(saved)} across{' '}
              {compression.blobs.toLocaleString()} blob(s).
            </>
          ) : (
            <>
              No compression detected — {compression.blobs.toLocaleString()} blob(s) take
              {' ' + formatBytes(compression.allocated_bytes)} on disk for{' '}
              {formatBytes(compression.apparent_bytes)} of data. Expected unless the volume is ZFS,
              btrfs or similar with compression enabled.
            </>
          )}
        </Alert>
      )}

      <Tooltip title="Reads the allocated block count of every blob, which on a compressing filesystem is its post-compression size. Takes a moment on a large store.">
        <span>
          <Button variant="outlined" onClick={measure} disabled={measuring}>
            {measuring ? 'Measuring…' : 'Measure compression'}
          </Button>
        </span>
      </Tooltip>
    </Paper>
  )
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  const body = (
    <Box>
      <Typography variant="caption" color="text.secondary" sx={{ display: 'block' }}>
        {label}
      </Typography>
      <Typography variant="body1">{value}</Typography>
    </Box>
  )
  return hint ? <Tooltip title={hint}>{body}</Tooltip> : body
}
