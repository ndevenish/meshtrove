import { useEffect } from 'react'
import { Dialog, Box, IconButton } from '@mui/material'
import CloseIcon from '@mui/icons-material/Close'
import ChevronLeftIcon from '@mui/icons-material/ChevronLeft'
import ChevronRightIcon from '@mui/icons-material/ChevronRight'

/// A full-bleed popout for the gallery's current image. The gallery itself pens
/// the picture into a ~460px square; clicking it opens this so the image can
/// grow to fill the viewport for a closer look.
///
/// The same `imageUrl` bytes the gallery already shows are reused — no new fetch
/// size — the win is purely the room to render them. Clicking anywhere (the
/// backdrop or the image) closes, matching the lightbox convention.
///
/// The whole gallery comes along: prev/next arrows and the ←/→ keys cycle
/// through `srcs`, wrapping at the ends, and each move is reported through
/// `onNavigate` so the gallery's selected thumbnail follows — the picture the
/// user lands on is the one still showing when they close.
export default function ImageLightbox({
  open,
  srcs,
  index,
  alt,
  onNavigate,
  onClose,
}: {
  open: boolean
  srcs: string[]
  index: number
  alt: string
  onNavigate: (index: number) => void
  onClose: () => void
}) {
  const count = srcs.length
  const hasMultiple = count > 1
  const src = srcs[index] ?? null
  const go = (delta: number) => onNavigate((index + delta + count) % count)

  // The Dialog already turns Escape into onClose; ←/→ are ours to wire up. Bound
  // only while open and only when there's somewhere to move, so the listener is
  // gone the moment it has nothing to do.
  useEffect(() => {
    if (!open || !hasMultiple) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'ArrowLeft') go(-1)
      else if (e.key === 'ArrowRight') go(1)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // `index` is read through `go`, so the handler must be rebound as it moves.
  }, [open, hasMultiple, index, count])

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
      {hasMultiple && (
        <>
          <IconButton
            // Stop the click reaching the backdrop's onClose, or every arrow
            // press would also shut the lightbox it's trying to page through.
            onClick={(e) => {
              e.stopPropagation()
              go(-1)
            }}
            aria-label="Previous image"
            sx={(theme) => ({
              position: 'absolute',
              left: 8,
              top: '50%',
              transform: 'translateY(-50%)',
              color: theme.palette.common.white,
              bgcolor: 'rgba(0, 0, 0, 0.5)',
              '&:hover': { bgcolor: 'rgba(0, 0, 0, 0.7)' },
            })}
          >
            <ChevronLeftIcon />
          </IconButton>
          <IconButton
            onClick={(e) => {
              e.stopPropagation()
              go(1)
            }}
            aria-label="Next image"
            sx={(theme) => ({
              position: 'absolute',
              right: 8,
              top: '50%',
              transform: 'translateY(-50%)',
              color: theme.palette.common.white,
              bgcolor: 'rgba(0, 0, 0, 0.5)',
              '&:hover': { bgcolor: 'rgba(0, 0, 0, 0.7)' },
            })}
          >
            <ChevronRightIcon />
          </IconButton>
        </>
      )}
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
