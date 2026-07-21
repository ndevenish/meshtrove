import { useEffect, useRef, useState } from 'react'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  Box,
  Button,
  Stack,
  Typography,
  IconButton,
  Alert,
} from '@mui/material'
import { useTheme } from '@mui/material/styles'
import CloseIcon from '@mui/icons-material/Close'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import * as THREE from 'three'
import { STLLoader } from 'three/examples/jsm/loaders/STLLoader.js'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'

import { downloadUrl, renderPreviewUrl } from '../api'

// Above this size, downloading the whole mesh and building the interactive
// scene is slow enough that we show a server-rendered still first and let the
// user opt into the live viewer. Live preview streams the file end-to-end, so
// the cost scales with the file.
const LIVE_PREVIEW_THRESHOLD = 5 * 1024 * 1024

/// An interactive three.js viewer for a single STL file. The mesh is streamed
/// straight from the file's download endpoint (same-origin, so the session
/// cookie rides along — staged import files stay gated to editors). The whole
/// scene lives and dies with the dialog: opening loads and builds it, closing
/// disposes every GPU resource so a big mesh doesn't linger.
///
/// Large meshes don't build the scene until asked: they open to an f3d-rendered
/// still (see `renderPreviewUrl`) with a button to load the live viewer.
export default function StlPreviewDialog({
  open,
  fileId,
  filename,
  size,
  onClose,
}: {
  open: boolean
  fileId: string | null
  filename: string
  /** File size in bytes; drives whether we defer to a rendered still first. */
  size: number
  onClose: () => void
}) {
  const theme = useTheme()
  // The canvas and scene backdrop follow the app theme. Kept in a ref so a
  // theme toggle mid-view can recolour the live scene without tearing it down
  // and re-downloading the mesh.
  const bg = theme.palette.background.default
  const sceneRef = useRef<THREE.Scene | null>(null)

  // A callback ref held in state, not a plain ref: MUI's dialog transition mounts
  // this Box *after* the effect would first fire, so gating on a ref read would
  // bail before the node exists. Storing the node in state re-runs the effect the
  // moment it attaches.
  const [mount, setMount] = useState<HTMLDivElement | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // Large files start on the rendered still; small ones go straight to live.
  // Recomputed each time a new file opens.
  const [live, setLive] = useState(size <= LIVE_PREVIEW_THRESHOLD)
  const [stillError, setStillError] = useState(false)
  useEffect(() => {
    setLive(size <= LIVE_PREVIEW_THRESHOLD)
    setStillError(false)
  }, [fileId, size])

  useEffect(() => {
    if (!open || !fileId || !mount || !live) return

    setLoading(true)
    setError(null)

    const scene = new THREE.Scene()
    scene.background = new THREE.Color(bg)
    sceneRef.current = scene

    const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100000)
    const renderer = new THREE.WebGLRenderer({ antialias: true })
    renderer.setPixelRatio(window.devicePixelRatio)
    mount.appendChild(renderer.domElement)

    // Two lights that follow the camera-ish rig plus fill, so the surface reads
    // as solid from any angle rather than going flat-black on the far side.
    scene.add(new THREE.AmbientLight(0xffffff, 0.6))
    const key = new THREE.DirectionalLight(0xffffff, 0.8)
    key.position.set(1, 1, 1)
    scene.add(key)
    const fill = new THREE.DirectionalLight(0xffffff, 0.4)
    fill.position.set(-1, -0.5, -1)
    scene.add(fill)

    const controls = new OrbitControls(camera, renderer.domElement)
    controls.enableDamping = true

    let mesh: THREE.Mesh | null = null
    let disposed = false
    let frame = 0

    const resize = () => {
      const w = mount.clientWidth
      const h = mount.clientHeight
      if (w === 0 || h === 0) return
      renderer.setSize(w, h)
      camera.aspect = w / h
      camera.updateProjectionMatrix()
    }
    const observer = new ResizeObserver(resize)
    observer.observe(mount)
    resize()

    const animate = () => {
      if (disposed) return
      frame = requestAnimationFrame(animate)
      controls.update()
      renderer.render(scene, camera)
    }
    animate()

    const loader = new STLLoader()
    loader.load(
      downloadUrl(fileId),
      (geometry) => {
        if (disposed) {
          geometry.dispose()
          return
        }
        geometry.computeVertexNormals()
        geometry.center()
        const material = new THREE.MeshStandardMaterial({
          color: 0xb0b7c3,
          metalness: 0.1,
          roughness: 0.75,
          flatShading: false,
        })
        mesh = new THREE.Mesh(geometry, material)
        scene.add(mesh)

        // Frame the model: point the camera at the bounding sphere from a
        // three-quarter angle, far enough back that it fills the view.
        geometry.computeBoundingSphere()
        const radius = geometry.boundingSphere?.radius ?? 1
        const dist = radius / Math.sin((camera.fov * Math.PI) / 360)
        camera.position.set(dist * 0.7, dist * 0.5, dist * 0.9)
        camera.near = Math.max(radius / 100, 0.01)
        camera.far = dist * 10
        camera.updateProjectionMatrix()
        controls.target.set(0, 0, 0)
        controls.update()
        setLoading(false)
      },
      undefined,
      (err) => {
        if (disposed) return
        setError(err instanceof Error ? err.message : 'Could not load this STL.')
        setLoading(false)
      },
    )

    return () => {
      disposed = true
      sceneRef.current = null
      cancelAnimationFrame(frame)
      observer.disconnect()
      controls.dispose()
      if (mesh) {
        scene.remove(mesh)
        mesh.geometry.dispose()
        ;(mesh.material as THREE.Material).dispose()
      }
      renderer.dispose()
      if (renderer.domElement.parentNode === mount) mount.removeChild(renderer.domElement)
    }
  }, [open, fileId, mount, live, bg])

  // Recolour a live scene on a theme toggle without rebuilding it.
  useEffect(() => {
    if (sceneRef.current) sceneRef.current.background = new THREE.Color(bg)
  }, [bg])

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle sx={{ pr: 6 }}>
        <Typography component="span" noWrap sx={{ display: 'block', fontWeight: 600 }}>
          {filename}
        </Typography>
        <IconButton
          onClick={onClose}
          sx={{ position: 'absolute', right: 8, top: 8 }}
          aria-label="Close"
        >
          <CloseIcon />
        </IconButton>
      </DialogTitle>
      <DialogContent>
        {error ? (
          <Alert severity="error">{error}</Alert>
        ) : live ? (
          <Box sx={{ position: 'relative' }}>
            <Box
              ref={setMount}
              sx={{
                width: '100%',
                height: { xs: 320, sm: 480 },
                borderRadius: 1,
                overflow: 'hidden',
                bgcolor: 'background.default',
                '& canvas': { display: 'block' },
              }}
            />
            {loading && (
              <Typography
                variant="body2"
                color="text.secondary"
                sx={{ position: 'absolute', top: '50%', left: 0, right: 0, textAlign: 'center' }}
              >
                Loading preview…
              </Typography>
            )}
            <Typography variant="caption" color="text.secondary" sx={{ mt: 1, display: 'block' }}>
              Drag to rotate · scroll to zoom · right-drag to pan
            </Typography>
          </Box>
        ) : (
          // Large mesh: a rendered still first, live viewer on request. The
          // still is rendered on demand server-side and not persisted.
          <Box>
            <Box
              sx={{
                position: 'relative',
                width: '100%',
                height: { xs: 320, sm: 480 },
                borderRadius: 1,
                overflow: 'hidden',
                bgcolor: 'background.default',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              {stillError ? (
                <Typography
                  variant="body2"
                  color="text.secondary"
                  sx={{ px: 2, textAlign: 'center' }}
                >
                  Couldn't render a preview image. Load the interactive preview below.
                </Typography>
              ) : (
                fileId && (
                  <Box
                    component="img"
                    src={renderPreviewUrl(fileId)}
                    alt={`Rendered preview of ${filename}`}
                    onError={() => setStillError(true)}
                    sx={{ maxWidth: '100%', maxHeight: '100%', objectFit: 'contain' }}
                  />
                )
              )}
            </Box>
            <Stack
              direction="row"
              spacing={1}
              sx={{ mt: 1, alignItems: 'center', justifyContent: 'space-between' }}
            >
              <Typography variant="caption" color="text.secondary">
                Large file — showing a rendered still.
              </Typography>
              <Button
                size="small"
                variant="outlined"
                startIcon={<ViewInArIcon />}
                onClick={() => setLive(true)}
              >
                Load interactive preview
              </Button>
            </Stack>
          </Box>
        )}
      </DialogContent>
    </Dialog>
  )
}
