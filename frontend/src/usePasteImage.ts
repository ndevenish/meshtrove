import { useEffect } from 'react'

import { api } from './api'

/// While `enabled`, paste (Ctrl/Cmd-V) any image on the clipboard to add it as
/// an image on the given model/bundle. Ignores non-image clipboard content, so
/// pasting text is unaffected.
export function usePasteImage(
  enabled: boolean,
  owner: 'models' | 'bundles',
  id: string,
  handlers: { onUploaded: () => void; onError?: (message: string) => void },
) {
  const { onUploaded, onError } = handlers
  useEffect(() => {
    if (!enabled) return
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
        for (const image of images) {
          const form = new FormData()
          form.append('file', image)
          await api.uploadImage(owner, id, form)
        }
        onUploaded()
      } catch (err) {
        onError?.(err instanceof Error ? err.message : String(err))
      }
    }
    window.addEventListener('paste', handler)
    return () => window.removeEventListener('paste', handler)
  }, [enabled, owner, id, onUploaded, onError])
}
