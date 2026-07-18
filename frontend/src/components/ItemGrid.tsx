import { Box } from '@mui/material'

import { type BrowseItem } from '../api'
import ModelCard from './ModelCard'
import BundleCard from './BundleCard'

/// The mixed models-and-bundles card grid. Shared by browse, the front page's
/// liked row and the liked page so a card looks and behaves the same wherever
/// you meet it.
///
/// `singleRow` clips the grid to its first row — the columns are set by the
/// viewport, so how many fit in "one row" is only knowable at render time.
/// Overflow rows collapse to zero height rather than being dropped, because the
/// alternative is guessing a count and being wrong at some window width.
export default function ItemGrid({
  items,
  singleRow = false,
}: {
  items: BrowseItem[]
  singleRow?: boolean
}) {
  return (
    <Box
      sx={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(210px, 1fr))',
        gap: 2,
        // One explicit row, and every *implicit* row after it collapsed to
        // nothing. `gridTemplateRows` is what makes row one explicit — without
        // it every row is implicit and the whole grid collapses.
        ...(singleRow && { gridTemplateRows: 'auto', gridAutoRows: 0, overflow: 'hidden' }),
      }}
    >
      {items.map((item) =>
        item.type === 'bundle' ? (
          <BundleCard key={item.id} bundle={{ ...item, model_count: item.count }} />
        ) : (
          <ModelCard
            key={item.id}
            model={{ ...item, variant_count: item.count, matched_variant_ids: null }}
          />
        ),
      )}
    </Box>
  )
}
