import { Box, IconButton, Tooltip } from '@mui/material'
import FavoriteIcon from '@mui/icons-material/Favorite'
import FavoriteBorderIcon from '@mui/icons-material/FavoriteBorder'
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { api } from '../api'
import { useAuth } from '../main'
import { LIKE_COLOR } from '../theme'

/// The heart on a card. Renders nothing for a logged-out visitor: the endpoint
/// would 401, and a control that always fails is worse than no control.
///
/// Positioned absolutely by the card, which is also why this sits *outside* the
/// card's CardActionArea — nested inside, every click would also follow the
/// link to the model.
export default function LikeButton({
  kind,
  id,
  liked,
}: {
  kind: 'model' | 'bundle'
  id: string
  liked: boolean
}) {
  const { user } = useAuth()
  const queryClient = useQueryClient()

  const mutation = useMutation({
    mutationFn: (next: boolean) => api.setLike(kind, id, next),
    // Every list that carries a `liked` flag or a like count is now stale: the
    // browse grid, the liked row, a bundle's member cards. Refetching all of
    // them is cheap next to getting one of them wrong.
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['browse'] })
      void queryClient.invalidateQueries({ queryKey: ['likes'] })
      void queryClient.invalidateQueries({ queryKey: ['bundle'] })
      void queryClient.invalidateQueries({ queryKey: ['models'] })
    },
  })

  if (!user) return null

  // The click is answered by the icon before the server has: a like is a low
  // stakes toggle, and waiting a round trip to fill the heart feels broken.
  const shown = mutation.isPending ? (mutation.variables ?? liked) : liked

  return (
    // An overlay covering the card's square artwork, laid over the link rather
    // than inside it — a <button> nested in an <a> is invalid HTML, and the
    // browser's own click handling on the anchor is hard to suppress reliably.
    // The overlay itself is click-through so the rest of the image still opens
    // the card; only the heart takes pointer events back.
    <Box
      sx={{
        position: 'absolute',
        top: 0,
        left: 0,
        right: 0,
        aspectRatio: '1',
        pointerEvents: 'none',
      }}
    >
      <Tooltip title={shown ? 'Remove from favourites' : 'Add to favourites'}>
        <IconButton
          size="small"
          aria-label={shown ? 'Remove from favourites' : 'Add to favourites'}
          onClick={(e) => {
            // The card behind is a link; without this the browser navigates away
            // the moment the heart is clicked.
            e.preventDefault()
            e.stopPropagation()
            mutation.mutate(!shown)
          }}
          sx={{
            position: 'absolute',
            bottom: 8,
            right: 8,
            // Its own disc rather than a bare icon on the artwork: a grey outline
            // heart is invisible against half the preview images in an archive.
            backgroundColor: 'background.paper',
            boxShadow: 2,
            pointerEvents: 'auto',
            color: shown ? LIKE_COLOR : 'text.disabled',
            '&:hover': { backgroundColor: 'background.paper', color: LIKE_COLOR },
          }}
        >
          {shown ? <FavoriteIcon fontSize="small" /> : <FavoriteBorderIcon fontSize="small" />}
        </IconButton>
      </Tooltip>
    </Box>
  )
}
