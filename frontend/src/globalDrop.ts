import { createContext, useContext, useEffect } from 'react'

/// AppShell mounts a window-wide "drop a file anywhere to import it" overlay. A
/// page that shows its own inline drop target — a model's file-upload box, the
/// bundle patch importer — has to switch that overlay off while its own box is up,
/// or the window handler swallows the drop and navigates away before the box ever
/// sees it. This is the seam: AppShell provides the registrar, and an inline drop
/// target calls the hook for as long as it wants the overlay to stand down.
export type SuppressGlobalDrop = () => () => void

export const GlobalDropContext = createContext<SuppressGlobalDrop>(() => () => {})

/// Suppress the app-wide drop overlay while `active` (default true) — typically
/// for as long as the calling component is mounted, or while it is in edit mode.
/// The effect's cleanup restores the overlay.
export function useSuppressGlobalDrop(active = true) {
  const suppress = useContext(GlobalDropContext)
  useEffect(() => {
    if (!active) return
    return suppress()
  }, [active, suppress])
}
