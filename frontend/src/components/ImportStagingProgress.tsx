import Box from '@mui/material/Box'
import List from '@mui/material/List'
import ListItem from '@mui/material/ListItem'
import ListItemText from '@mui/material/ListItemText'
import Typography from '@mui/material/Typography'
import FolderIcon from '@mui/icons-material/Folder'

import { formatBytes, type ImportFolder } from '../api'

/// What an import shows while it is still staging: the folders that have
/// arrived so far and how much is in each.
///
/// Deliberately not the file tree. A dropbox pickup can spend hours staging
/// tens of thousands of files, and asking the server to list every one of them
/// on a timer is what makes a large import crawl — so while it fills up the
/// page watches counts, and swaps to the tree once there is a settled import to
/// draw. Committing is refused until then anyway, so there is nothing here an
/// admin could act on file by file.
export default function ImportStagingProgress({ folders }: { folders: ImportFolder[] }) {
  if (folders.length === 0) {
    return (
      <Typography variant="body2" color="text.secondary">
        Nothing staged yet.
      </Typography>
    )
  }

  const files = folders.reduce((n, f) => n + f.files, 0)
  const bytes = folders.reduce((n, f) => n + f.bytes, 0)

  return (
    <Box>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
        {files.toLocaleString()} file{files === 1 ? '' : 's'} in {folders.length.toLocaleString()}{' '}
        folder{folders.length === 1 ? '' : 's'} · {formatBytes(bytes)} so far. The full contents are
        listed once staging finishes.
      </Typography>
      <List dense disablePadding>
        {folders.map((folder) => (
          <ListItem key={folder.path} disableGutters sx={{ alignItems: 'flex-start', py: 0.25 }}>
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
    </Box>
  )
}
