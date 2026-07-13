import {
  Container,
  Typography,
  Table,
  TableHead,
  TableRow,
  TableCell,
  TableBody,
  Chip,
  Button,
  Paper,
} from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type Job } from '../api'
import { useAuth } from '../main'

const STATUS_COLOR: Record<Job['status'], 'default' | 'info' | 'success' | 'error' | 'warning'> = {
  queued: 'default',
  running: 'info',
  succeeded: 'success',
  failed: 'error',
  cancelled: 'warning',
}

export default function JobsPage() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const { data: jobs } = useQuery({
    queryKey: ['jobs'],
    queryFn: () => api.jobs(),
    // Live-ish view while imports/renders run
    refetchInterval: (query) =>
      query.state.data?.some((job) => job.status === 'queued' || job.status === 'running')
        ? 2000
        : 10000,
  })

  return (
    <Container maxWidth="md" sx={{ py: 3 }}>
      <Typography variant="h5" sx={{ mb: 2 }}>
        Background jobs
      </Typography>
      <Paper variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>#</TableCell>
              <TableCell>Kind</TableCell>
              <TableCell>Status</TableCell>
              <TableCell>Attempts</TableCell>
              <TableCell>Created</TableCell>
              <TableCell>Error</TableCell>
              <TableCell />
            </TableRow>
          </TableHead>
          <TableBody>
            {(jobs ?? []).map((job) => (
              <TableRow key={job.id}>
                <TableCell>{job.id}</TableCell>
                <TableCell>{job.kind}</TableCell>
                <TableCell>
                  <Chip label={job.status} size="small" color={STATUS_COLOR[job.status]} />
                </TableCell>
                <TableCell>{job.attempts}</TableCell>
                <TableCell>{new Date(job.created_at).toLocaleString()}</TableCell>
                <TableCell sx={{ maxWidth: 280 }}>
                  <Typography variant="caption" sx={{ wordBreak: 'break-word' }}>
                    {job.last_error ?? ''}
                  </Typography>
                </TableCell>
                <TableCell>
                  {user && user.role !== 'viewer' && job.status === 'failed' && (
                    <Button
                      size="small"
                      onClick={async () => {
                        await api.retryJob(job.id)
                        void queryClient.invalidateQueries({ queryKey: ['jobs'] })
                      }}
                    >
                      Retry
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            ))}
            {jobs?.length === 0 && (
              <TableRow>
                <TableCell colSpan={7}>
                  <Typography color="text.secondary">No jobs yet.</Typography>
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </Paper>
    </Container>
  )
}
