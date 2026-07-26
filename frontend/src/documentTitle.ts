import { useEffect } from 'react'

const DEFAULT_TITLE = 'MeshTrove'

// Reflect the thing you're looking at in the browser tab. Pass a falsy title
// (the page's data hasn't loaded yet) to hold the default; the tab settles onto
// the real name once it arrives, and returns to the default on unmount.
export function useDocumentTitle(title: string | null | undefined) {
  useEffect(() => {
    document.title = title ? `${title} — ${DEFAULT_TITLE}` : DEFAULT_TITLE
    return () => {
      document.title = DEFAULT_TITLE
    }
  }, [title])
}
