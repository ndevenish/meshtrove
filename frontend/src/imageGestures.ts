import { useCallback, useEffect, useRef, useState } from 'react'

import { api } from './api'

/// The two ways to put a picture on a model or bundle without going through a
/// file picker: paste it, or drag it onto the page. Both end in the same upload,
/// so they live together.
///
/// `id` must be the owner's **UUID**, not the slug in the address bar: the image
/// routes take `Path<Uuid>`, and a slug is rejected before the handler is
/// reached.
export type ImageOwner = 'models' | 'bundles'

interface Handlers {
  onUploaded: () => void
  onError?: (message: string) => void
}

async function uploadImages(owner: ImageOwner, id: string, images: File[]) {
  for (const image of images) {
    const form = new FormData()
    form.append('file', image)
    await api.uploadImage(owner, id, form)
  }
}

/// While `enabled`, paste (Ctrl/Cmd-V) any image on the clipboard to add it as
/// an image on the given model/bundle. Ignores non-image clipboard content, so
/// pasting text is unaffected.
export function usePasteImage(enabled: boolean, owner: ImageOwner, id: string, handlers: Handlers) {
  const { onUploaded, onError } = handlers
  useEffect(() => {
    if (!enabled || !id) return
    const handler = async (e: ClipboardEvent) => {
      const images: File[] = []
      for (const item of e.clipboardData?.items ?? []) {
        if (item.kind === 'file' && item.type.startsWith('image/')) {
          const file = item.getAsFile()
          if (file) {
            const ext = file.type.split('/')[1] || 'png'
            images.push(file.name ? file : new File([file], `pasted.${ext}`, { type: file.type }))
          }
        }
      }
      if (!images.length) return
      e.preventDefault()
      try {
        await uploadImages(owner, id, images)
        onUploaded()
      } catch (err) {
        onError?.(err instanceof Error ? err.message : String(err))
      }
    }
    window.addEventListener('paste', handler)
    return () => window.removeEventListener('paste', handler)
  }, [enabled, owner, id, onUploaded, onError])
}

/// Is this drag carrying nothing but images? Only then does an image drop make
/// sense — a mixed bag, or a folder, is a file import and belongs to the app-wide
/// handler. `items` exposes each entry's MIME type during the drag (the *data*
/// stays sealed until the drop), which is exactly enough to decide.
function isImageDrag(e: DragEvent): boolean {
  const items = [...(e.dataTransfer?.items ?? [])]
  return (
    items.length > 0 &&
    items.every((item) => item.kind === 'file' && item.type.startsWith('image/'))
  )
}

/// While `enabled`, dropping image files anywhere on the page adds them as
/// images on the given model/bundle. Returns whether such a drag is currently
/// over the page, so the caller can show a target.
///
/// Listens on the **capture** phase, so it runs before AppShell's window-level
/// import handler and can take the event out of its hands with
/// `stopPropagation` — otherwise every image drop is swallowed by "drop anywhere
/// to import", which stages the picture as a new import instead of adding it
/// here. Capture also means the whole page is the target, matching paste: there
/// is no small box to aim at.
///
/// The one exception is an explicit file drop target (`[data-file-drop]`, the
/// "upload files to this model" box). It says what it does, so a picture dropped
/// *into* it is stored as a file — the only way left to attach an image as a
/// file rather than a picture.
export function useDropImage(
  enabled: boolean,
  owner: ImageOwner,
  id: string,
  handlers: Handlers,
): boolean {
  const { onUploaded, onError } = handlers
  const [dragging, setDragging] = useState(false)
  // dragenter/dragleave fire per element crossed, not per page: count them, or
  // moving over a child looks like leaving.
  const depth = useRef(0)

  const claims = useCallback((e: DragEvent) => {
    if (!isImageDrag(e)) return false
    const target = e.target
    return !(target instanceof Element && target.closest('[data-file-drop]'))
  }, [])

  useEffect(() => {
    if (!enabled || !id) {
      setDragging(false)
      return
    }
    const onEnter = (e: DragEvent) => {
      if (!claims(e)) return
      e.stopPropagation()
      depth.current += 1
      setDragging(true)
    }
    // Unconditional, unlike the others: `items` is not reliably populated on
    // dragleave, so asking "is this still an image drag?" here can answer no and
    // leave the target showing forever. Counting every leave can only end at
    // zero, which is the safe direction. It is left to propagate too — AppShell
    // never saw the matching enter, so its own counter is already at zero and
    // decrementing it is a no-op.
    const onLeave = () => {
      depth.current = Math.max(0, depth.current - 1)
      if (depth.current === 0) setDragging(false)
    }
    const onOver = (e: DragEvent) => {
      if (!claims(e)) return
      // Without preventDefault the browser refuses the drop entirely.
      e.preventDefault()
      e.stopPropagation()
    }
    const onDrop = (e: DragEvent) => {
      if (!claims(e)) return
      e.preventDefault()
      e.stopPropagation()
      depth.current = 0
      setDragging(false)
      const images = [...(e.dataTransfer?.files ?? [])].filter((f) => f.type.startsWith('image/'))
      if (!images.length) return
      uploadImages(owner, id, images)
        .then(onUploaded)
        .catch((err) => onError?.(err instanceof Error ? err.message : String(err)))
    }
    const opts = { capture: true }
    window.addEventListener('dragenter', onEnter, opts)
    window.addEventListener('dragleave', onLeave, opts)
    window.addEventListener('dragover', onOver, opts)
    window.addEventListener('drop', onDrop, opts)
    return () => {
      window.removeEventListener('dragenter', onEnter, opts)
      window.removeEventListener('dragleave', onLeave, opts)
      window.removeEventListener('dragover', onOver, opts)
      window.removeEventListener('drop', onDrop, opts)
    }
  }, [enabled, owner, id, claims, onUploaded, onError])

  return dragging
}
