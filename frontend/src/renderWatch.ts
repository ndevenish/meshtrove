import { useEffect, useRef } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api } from './api'

/// Cards carry a summary's `primary_image_id`. Regenerating a preview *replaces*
/// that image — the renderer's replace mode deletes the old row (UUID and all)
/// and inserts a fresh one — so the old id the card holds is now dead. The detail
/// page refetches itself when a render lands (see ModelPage), but the list and
/// summary queries behind the cards have no such trigger: browse, the likes row,
/// and a bundle's member cards keep pointing at the stale image until a manual
/// reload. Mounted once in AppShell, this closes that gap.
///
/// Detection mirrors ModelPage's job watcher: watch for jobs that *have finished*
/// rather than for the queue going idle — an f3d render often completes inside a
/// single poll, so the "was running, now isn't" edge is easy to miss entirely.
/// Remember which finished jobs have been accounted for; any new one means a
/// preview may have changed, so invalidate the card-bearing queries.
export function useRenderRefresh() {
  const queryClient = useQueryClient()
  // Shares the ['jobs','all'] cache with any page also watching it, so this adds
  // no extra polling; and the interval only runs while a render is in flight.
  const { data: jobs } = useQuery({
    queryKey: ['jobs', 'all'],
    queryFn: () => api.jobs(''),
    refetchInterval: (query) =>
      (query.state.data ?? []).some(
        (j) => j.kind === 'render_preview' && (j.status === 'queued' || j.status === 'running'),
      )
        ? 1500
        : false,
  })
  const settled = useRef<Set<number> | null>(null)
  useEffect(() => {
    if (!jobs) return
    const finished = jobs
      .filter(
        (j) => j.kind === 'render_preview' && (j.status === 'succeeded' || j.status === 'failed'),
      )
      .map((j) => j.id)
    // The first payload is the baseline: everything already finished when the app
    // opened is old news, and invalidating for it would be a pointless refetch.
    if (settled.current === null) {
      settled.current = new Set(finished)
      return
    }
    const seen = settled.current
    const fresh = finished.filter((id) => !seen.has(id))
    if (fresh.length === 0) return
    for (const id of fresh) seen.add(id)
    // Every query prefix that renders a card off a summary's primary image:
    // browse's grid and liked row, the likes page, and a bundle's member cards.
    // (A failed render changes no picture, but these only refetch the active
    // page's queries, so invalidating on any settle keeps the rule simple.)
    void queryClient.invalidateQueries({ queryKey: ['browse'] })
    void queryClient.invalidateQueries({ queryKey: ['likes'] })
    void queryClient.invalidateQueries({ queryKey: ['bundle'] })
  }, [jobs, queryClient])
}
