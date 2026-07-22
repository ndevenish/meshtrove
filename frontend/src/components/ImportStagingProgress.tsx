import Box from '@mui/material/Box'
import LinearProgress from '@mui/material/LinearProgress'
import List from '@mui/material/List'
import ListItem from '@mui/material/ListItem'
import ListItemText from '@mui/material/ListItemText'
import Typography from '@mui/material/Typography'
import FolderIcon from '@mui/icons-material/Folder'

import { formatBytes, type ImportFolder, type ImportSummary } from '../api'

/// What an import shows while it is still staging: how far the running job has
/// got, what is still queued behind it, and the folders that have arrived so
/// far.
///
/// Deliberately not the file tree. A dropbox pickup can spend hours staging
/// tens of thousands of files, and asking the server to list every one of them
/// on a timer is what makes a large import crawl — so while it fills up the
/// page watches counts, and swaps to the tree once there is a settled import to
/// draw. Committing is refused until then anyway, so there is nothing here an
/// admin could act on file by file.
export default function ImportStagingProgress({
  staged,
  folders,
}: {
  staged: ImportSummary
  folders: ImportFolder[]
}) {
  const files = folders.reduce((n, f) => n + f.files, 0)
  const bytes = folders.reduce((n, f) => n + f.bytes, 0)

  return (
    <Box>
      <StagingBar staged={staged} />
      {folders.length === 0 ? (
        <Typography variant="body2" color="text.secondary">
          Nothing staged yet.
        </Typography>
      ) : (
        <>
          <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
            {files.toLocaleString()} file{files === 1 ? '' : 's'} in{' '}
            {folders.length.toLocaleString()} folder{folders.length === 1 ? '' : 's'} ·{' '}
            {formatBytes(bytes)} so far. The full contents are listed once staging finishes.
          </Typography>
          <List dense disablePadding>
            {folders.map((folder) => (
              <ListItem
                key={folder.path}
                disableGutters
                sx={{ alignItems: 'flex-start', py: 0.25 }}
              >
                <FolderIcon fontSize="small" sx={{ mr: 1, mt: 0.25, color: 'text.disabled' }} />
                <ListItemText
                  // A file at the import's root has an empty path; it has no folder
                  // to name, so say where it is rather than render a blank row.
                  primary={folder.path || '(top level)'}
                  secondary={`${folder.files.toLocaleString()} file${
                    folder.files === 1 ? '' : 's'
                  } · ${formatBytes(folder.bytes)}`}
                  slotProps={{
                    primary: { variant: 'body2', sx: { wordBreak: 'break-word' } },
                    secondary: { variant: 'caption' },
                  }}
                />
              </ListItem>
            ))}
          </List>
        </>
      )}
    </Box>
  )
}

/// The bar itself: what the running job is getting through, and how many
/// archives are still to be opened after it.
///
/// There is no import-wide total to count towards and there cannot be one —
/// every archive opened can reveal more archives, so the only honest
/// denominator is the one job's own file count, which it knows exactly once it
/// has extracted. Between jobs, and while an archive is still being
/// decompressed, nothing has reported a total: the bar goes indeterminate
/// rather than guessing or sitting at a stale figure.
function StagingBar({ staged }: { staged: ImportSummary }) {
  const { staging_done: done, staging_total: total, archives_left: left } = staged
  const known = total > 0
  const percent = known ? Math.min(100, (done / total) * 100) : 0
  return (
    <Box sx={{ mb: 2 }}>
      <Typography variant="body2" sx={{ mb: 0.5 }}>
        {/* Nothing has reported a total yet: an archive being decompressed, or
            a drop being walked, neither of which can say how much is coming. */}
        {known
          ? `Staging ${done.toLocaleString()} of ${total.toLocaleString()} files`
          : 'Reading what was dropped…'}
        {/* `left` counts the archive being opened right now as well as the ones
            behind it, so this is "still to unpack", not "after this one". */}
        {left > 0 && ` · ${left.toLocaleString()} archive${left === 1 ? '' : 's'} still to unpack`}
      </Typography>
      <LinearProgress
        variant={known ? 'determinate' : 'indeterminate'}
        value={percent}
        sx={{ height: 6, borderRadius: 3 }}
      />
      {left > 0 && (
        <Typography variant="caption" color="text.secondary">
          The count can still grow: an archive is only known to hold more archives once it has been
          opened.
        </Typography>
      )}
    </Box>
  )
}
