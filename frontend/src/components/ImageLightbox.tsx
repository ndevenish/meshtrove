import { Dialog, Box, IconButton } from '@mui/material'
import CloseIcon from '@mui/icons-material/Close'

/// A full-bleed popout for the gallery's current image. The gallery itself pens
/// the picture into a ~460px square; clicking it opens this so the image can
/// grow to fill the viewport for a closer look.
///
/// The same `imageUrl` bytes the gallery already shows are reused — no new fetch
/// size — the win is purely the room to render them. Clicking anywhere (the
/// backdrop or the image) closes, matching the lightbox convention.
export default function ImageLightbox({
  open,
  src,
  alt,
  onClose,
}: {
  open: boolean
  src: string | null
  alt: string
  onClose: () => void
}) {
  return (
    <Dialog
      open={open && src != null}
      onClose={onClose}
      maxWidth={false}
      onClick={onClose}
      slotProps={{
        paper: {
          sx: {
            bgcolor: 'transparent',
            boxShadow: 'none',
            m: 2,
            maxWidth: '96vw',
            maxHeight: '96vh',
          },
        },
      }}
    >
      <IconButton
        onClick={onClose}
        aria-label="Close"
        sx={(theme) => ({
          position: 'absolute',
          right: 8,
          top: 8,
          color: theme.palette.common.white,
          bgcolor: 'rgba(0, 0, 0, 0.5)',
          '&:hover': { bgcolor: 'rgba(0, 0, 0, 0.7)' },
        })}
      >
        <CloseIcon />
      </IconButton>
      {src && (
        <Box
          component="img"
          src={src}
          alt={alt}
          sx={{
            display: 'block',
            maxWidth: '96vw',
            maxHeight: '96vh',
            objectFit: 'contain',
            borderRadius: 1,
          }}
        />
      )}
    </Dialog>
  )
}
