import { Box, IconButton, Tooltip, Typography } from '@mui/material'
import FavoriteIcon from '@mui/icons-material/Favorite'
import FavoriteBorderIcon from '@mui/icons-material/FavoriteBorder'
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { api } from '../api'
import { useAuth } from '../main'
import { LIKE_COLOR } from '../theme'

/// The heart in a card's foot: one control carrying both what *you* think and
/// what everyone thinks. The count sits inside the button rather than beside it
/// so clicking the number does what clicking a number next to a heart looks
/// like it should. Hidden at zero — a row of honest zeroes tells nobody
/// anything.
///
/// Lives outside the card's CardActionArea: a <button> nested in an <a> is
/// invalid HTML, and the anchor would swallow the click.
export default function LikeButton({
  kind,
  id,
  liked,
  likeCount,
}: {
  kind: 'model' | 'bundle'
  id: string
  liked: boolean
  likeCount: number
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

  // A logged-out visitor still gets the tally — they just can't add to it.
  // Liking would only 401, and a control that always fails is worse than none.
  if (!user) {
    if (likeCount === 0) return null
    return (
      <Box sx={{ display: 'flex', alignItems: 'center', gap: 0.5, px: 0.5 }}>
        <FavoriteIcon sx={{ fontSize: 18, color: 'text.disabled' }} />
        <Typography variant="caption" color="text.secondary">
          {likeCount}
        </Typography>
      </Box>
    )
  }

  // The click is answered by the icon before the server has: a like is a low
  // stakes toggle, and waiting a round trip to fill the heart feels broken.
  // The tally moves with it, or the heart and the number briefly disagree.
  const shown = mutation.isPending ? (mutation.variables ?? liked) : liked
  const count = likeCount + (shown === liked ? 0 : shown ? 1 : -1)

  return (
    <Tooltip title={shown ? 'Remove from favourites' : 'Add to favourites'}>
      <IconButton
        size="small"
        aria-label={shown ? 'Remove from favourites' : 'Add to favourites'}
        onClick={() => mutation.mutate(!shown)}
        sx={{
          borderRadius: 5,
          gap: 0.5,
          px: count > 0 ? 1 : undefined,
          color: shown ? LIKE_COLOR : 'text.disabled',
          '&:hover': { color: LIKE_COLOR },
        }}
      >
        {shown ? <FavoriteIcon fontSize="small" /> : <FavoriteBorderIcon fontSize="small" />}
        {count > 0 && (
          <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1 }}>
            {count}
          </Typography>
        )}
      </IconButton>
    </Tooltip>
  )
}
