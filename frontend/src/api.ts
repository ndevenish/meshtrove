// Thin typed fetch layer over the backend API (same origin, cookie auth).

export interface User {
  id: string
  username: string
  role: 'admin' | 'editor' | 'viewer'
}

export interface Creator {
  id: string
  name: string
  kind: 'author' | 'company' | 'site'
  url: string | null
  notes: string | null
  model_count: number
}

/** A tag from the flat variant vocabulary ("32mm", "supported"). */
export interface VariantTag {
  id: string
  name: string
  description: string | null
  variant_count: number
}

export interface Tag {
  id: string
  name: string
  model_count: number
}

export interface ModelSummary {
  id: string
  name: string
  slug: string
  creator_id: string | null
  creator_name: string | null
  primary_image_id: string | null
  tags: string[]
  like_count: number
  variant_count: number
  matched_variant_ids: string[] | null
  updated_at: string
}

export interface SearchResults {
  models: ModelSummary[]
  total: number
  page: number
  per_page: number
}

export interface FileRecord {
  id: string
  blob_sha256: string
  path: string
  filename: string
  mime: string | null
  kind: 'model' | 'project' | 'raw' | 'document' | 'archive' | 'other'
  size: number
  created_at: string
}

export interface VariantDetail {
  id: string
  model_id: string
  /** Optional display label; null for the anonymous variant */
  name: string | null
  /** The tag set that identifies this variant; empty = anonymous */
  tags: string[]
  print_notes: string | null
  derived_from_variant_id: string | null
  file_count: number
  total_size: number
}

export interface ImageRecord {
  id: string
  kind: string
  is_primary: boolean
  /** set when the image belongs to a variant of the model, not the model itself */
  variant_id?: string | null
}

export interface ModelDetail {
  id: string
  name: string
  slug: string
  creator_id: string | null
  creator_name: string | null
  source_url: string | null
  license: string | null
  purchase_price: number | null
  purchase_date: string | null
  order_ref: string | null
  tags: string[]
  description_md: string | null
  variants: VariantDetail[]
  images: ImageRecord[]
  /** bundles this model belongs to */
  bundles: BundleRef[]
  created_by: string
}

export interface BundleRef {
  id: string
  name: string
}

export interface Revision {
  id: string
  body_md: string
  label: string | null
  author: string
  created_at: string
}

export type DescOwner = 'models' | 'bundles'

export interface BundleSummary {
  id: string
  name: string
  slug: string
  kind: string
  creator_id: string | null
  creator_name: string | null
  primary_image_id: string | null
  tags: string[]
  model_count: number
  updated_at: string
}

export interface BundleDetail {
  id: string
  name: string
  slug: string
  kind: string
  creator_id: string | null
  creator_name: string | null
  source_url: string | null
  tags: string[]
  description_md: string | null
  models: ModelSummary[]
  images: ImageRecord[]
  created_by: string
}

export interface BundleResults {
  bundles: BundleSummary[]
  total: number
  page: number
  per_page: number
}

/// One row in the unified browse (models + bundles mixed). `count` is
/// variant_count for models, model_count for bundles.
export interface BrowseItem {
  type: 'model' | 'bundle'
  id: string
  name: string
  slug: string
  creator_id: string | null
  creator_name: string | null
  primary_image_id: string | null
  tags: string[]
  like_count: number | null
  count: number
  updated_at: string
}

export interface BrowseResults {
  items: BrowseItem[]
  total: number
  page: number
  per_page: number
}

/// A dropped archive, staged. Neither a model nor a bundle: it stays out of
/// browse until it is committed to one (see `commitImport`).
export interface ImportSummary {
  id: string
  name: string
  created_by: string
  created_at: string
  file_count: number
  /** its archive is still unpacking; committing is refused until this clears */
  unpacking: boolean
}

// --- Import layout templates (regex-driven carve; see docs/plan.md) ---------

/// What a capture group means. No "variant" role: a variant IS its tag set,
/// so the union of a file's variant-tag captures is its variant.
export type GroupRole = 'model_name' | 'model_tag' | 'variant_tag' | 'ignore'

/// The user-editable layout definition. The pattern is a backend (fancy-regex)
/// pattern — an opaque string here; the frontend never runs it.
export interface LayoutSpec {
  pattern: string
  /** capture group number (as a string key) -> role */
  roles: Record<string, GroupRole>
  /** lowercased raw capture -> variant tag names ([] = maps to nothing) */
  value_map: Record<string, string[]>
  /** drop the folders once the carve has read them: files land with no path */
  flatten?: boolean
}

export interface ImportLayout extends LayoutSpec {
  id: string
  name: string
  creator_id: string | null
}

/// A slice of a file's path; `group` set = highlight it by that group's role.
export interface PathPart {
  text: string
  group?: number
}

export interface FileAnnotation {
  id: string
  matched: boolean
  parts: PathPart[]
  model_name?: string
  model_tags: string[]
  variant_tags: string[]
  /** raw variant-tag captures with no resolution — fill the value map in */
  unmapped: string[]
}

export interface PlanVariant {
  /** empty = the model's unsorted bucket */
  tags: string[]
  file_count: number
  example: string
}

export interface PlanModel {
  name: string
  tags: string[]
  file_count: number
  variants: PlanVariant[]
}

export interface GroupInfo {
  index: number
  examples: string[]
}

export interface CapturedValue {
  raw: string
  /** resolved variant tag names; null = unmapped */
  tags: string[] | null
}

/// The dry run of a layout over an import's staged files. Commit executes the
/// same computation, so this preview is the result.
export interface LayoutPlan {
  total: number
  matched: number
  carved: number
  groups: GroupInfo[]
  models: PlanModel[]
  values: CapturedValue[]
  model_names: string[]
  annotations: FileAnnotation[]
}

/// One-model imports pool everything into variants; bundle imports split
/// member models by the model-name capture.
export type PlanTarget = 'model' | 'bundle'

/// The single decision an import exists to defer: what is this archive?
/// An attached `layout` carves the files into models/variants as it commits.
/// Metadata typed once on the import page. Flattened into the commit body; on a
/// bundle commit it lands on the bundle *and* on every member model the carve
/// creates. A null/absent field says nothing and overwrites nothing.
export interface ImportMeta {
  creator_id?: string | null
  source_url?: string | null
  license?: string | null
  purchase_price?: number | null
  purchase_date?: string | null
  order_ref?: string | null
  tags?: string[]
  description_md?: string | null
}

export type CommitTarget = ImportMeta &
  (
    | { target: 'new_model'; name?: string; layout?: LayoutSpec }
    | { target: 'new_bundle'; name?: string; kind?: string; layout?: LayoutSpec }
    | { target: 'bundle'; bundle_id: string; layout?: LayoutSpec }
  )

export interface CommitResult {
  type: 'model' | 'bundle'
  id: string
  slug: string
}

export interface Job {
  id: number
  kind: string
  status: 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled'
  attempts: number
  last_error: string | null
  created_at: string
  payload?: unknown
}

export interface FileUpdate {
  kind?: FileRecord['kind']
  variant_id?: string
  /** carve a bundle-owned file into a member model */
  model_id?: string
  /** push a model-owned file up into a bundle it belongs to */
  bundle_id?: string
  unsorted?: boolean
  filename?: string
  path?: string
}

export interface RendererConfig {
  tool: string
  args: string[]
}

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, init)
  if (!response.ok) {
    throw new ApiError(response.status, (await response.text()) || response.statusText)
  }
  if (response.status === 204) return undefined as T
  return response.json()
}

const json = (body: unknown): RequestInit => ({
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(body),
})

/// Multipart upload with progress. `fetch` cannot report upload progress, so
/// big archive uploads go through XHR to drive a real percentage bar.
export function uploadWithProgress<T>(
  path: string,
  form: FormData,
  onProgress: (fraction: number) => void,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    xhr.open('POST', path)
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && e.total > 0) onProgress(e.loaded / e.total)
    }
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        onProgress(1)
        resolve(xhr.responseText ? JSON.parse(xhr.responseText) : (undefined as T))
      } else {
        reject(new ApiError(xhr.status, xhr.responseText || xhr.statusText))
      }
    }
    xhr.onerror = () => reject(new ApiError(0, 'network error during upload'))
    xhr.send(form)
  })
}

export const api = {
  me: () => request<User>('/api/me'),
  login: (username: string, password: string) =>
    request<User>('/auth/login', json({ username, password })),
  register: (username: string, password: string) =>
    request<User>('/auth/register', json({ username, password })),
  logout: () => request<void>('/auth/logout', { method: 'POST' }),

  searchModels: (params: URLSearchParams) => request<SearchResults>(`/api/models?${params}`),
  model: (id: string) => request<ModelDetail>(`/api/models/${id}`),
  createModel: (body: unknown) => request<ModelDetail>('/api/models', json(body)),
  updateModel: (id: string, body: unknown) =>
    request<ModelDetail>(`/api/models/${id}`, { ...json(body), method: 'PUT' }),
  deleteModel: (id: string) => request<void>(`/api/models/${id}`, { method: 'DELETE' }),

  // Description revisions work identically for models and bundles.
  updateDescription: (owner: DescOwner, id: string, body_md: string) =>
    request<Revision>(`/api/${owner}/${id}/description`, { ...json({ body_md }), method: 'PUT' }),
  revisions: (owner: DescOwner, id: string) =>
    request<Revision[]>(`/api/${owner}/${id}/description/revisions`),
  labelRevision: (owner: DescOwner, id: string, revId: string, label: string | null) =>
    request<void>(`/api/${owner}/${id}/description/revisions/${revId}/label`, {
      ...json({ label }),
      method: 'PUT',
    }),

  browse: (params: URLSearchParams) => request<BrowseResults>(`/api/browse?${params}`),
  searchBundles: (params: URLSearchParams) => request<BundleResults>(`/api/bundles?${params}`),
  bundle: (id: string) => request<BundleDetail>(`/api/bundles/${id}`),
  createBundle: (body: unknown) => request<BundleDetail>('/api/bundles', json(body)),
  updateBundle: (id: string, body: unknown) =>
    request<BundleDetail>(`/api/bundles/${id}`, { ...json(body), method: 'PUT' }),
  deleteBundle: (id: string) => request<void>(`/api/bundles/${id}`, { method: 'DELETE' }),
  addModelToBundle: (bundleId: string, modelId: string) =>
    request<void>(`/api/bundles/${bundleId}/models`, json({ model_id: modelId })),
  removeModelFromBundle: (bundleId: string, modelId: string) =>
    request<void>(`/api/bundles/${bundleId}/models/${modelId}`, { method: 'DELETE' }),
  bundleFiles: (id: string) => request<FileRecord[]>(`/api/bundles/${id}/files`),
  uploadBundleFiles: (id: string, form: FormData) =>
    request<FileRecord[]>(`/api/bundles/${id}/files`, { method: 'POST', body: form }),

  imports: () => request<ImportSummary[]>('/api/imports'),
  import: (id: string) => request<ImportSummary>(`/api/imports/${id}`),
  createImport: (name: string) => request<ImportSummary>('/api/imports', json({ name })),
  renameImport: (id: string, name: string) =>
    request<ImportSummary>(`/api/imports/${id}`, { ...json({ name }), method: 'PUT' }),
  deleteImport: (id: string) => request<void>(`/api/imports/${id}`, { method: 'DELETE' }),
  importFiles: (id: string) => request<FileRecord[]>(`/api/imports/${id}/files`),
  commitImport: (id: string, target: CommitTarget) =>
    request<CommitResult>(`/api/imports/${id}/commit`, json(target)),
  planImport: (id: string, spec: LayoutSpec, target: PlanTarget) =>
    request<LayoutPlan>(`/api/imports/${id}/plan`, json({ ...spec, target })),
  importLayouts: () => request<ImportLayout[]>('/api/import-layouts'),
  createImportLayout: (body: { name: string; creator_id?: string | null } & LayoutSpec) =>
    request<ImportLayout>('/api/import-layouts', json(body)),
  deleteImportLayout: (id: string) =>
    request<void>(`/api/import-layouts/${id}`, { method: 'DELETE' }),

  createVariant: (modelId: string, body: unknown) =>
    request<VariantDetail>(`/api/models/${modelId}/variants`, json(body)),
  updateVariant: (id: string, body: unknown) =>
    request<VariantDetail>(`/api/variants/${id}`, { ...json(body), method: 'PUT' }),
  deleteVariant: (id: string) => request<void>(`/api/variants/${id}`, { method: 'DELETE' }),
  variantFiles: (id: string) => request<FileRecord[]>(`/api/variants/${id}/files`),
  uploadVariantFiles: (id: string, form: FormData) =>
    request<FileRecord[]>(`/api/variants/${id}/files`, { method: 'POST', body: form }),
  modelFiles: (id: string) => request<FileRecord[]>(`/api/models/${id}/files`),
  uploadModelFiles: (id: string, form: FormData) =>
    request<FileRecord[]>(`/api/models/${id}/files`, { method: 'POST', body: form }),
  updateFile: (id: string, body: FileUpdate) =>
    request<FileRecord>(`/api/files/${id}`, { ...json(body), method: 'PATCH' }),
  deleteFile: (id: string) => request<void>(`/api/files/${id}`, { method: 'DELETE' }),

  uploadImage: (owner: 'models' | 'variants' | 'bundles', id: string, form: FormData) =>
    request<ImageRecord>(`/api/${owner}/${id}/images`, { method: 'POST', body: form }),
  markPrimary: (imageId: string) =>
    request<void>(`/api/images/${imageId}/primary`, { method: 'PUT' }),
  /// Favourite a variant's picture *for the model*: the model takes a copy of the
  /// same blob as its own primary, and the variant keeps its thumbnail.
  promoteImage: (modelId: string, imageId: string) =>
    request<void>(`/api/models/${modelId}/images/${imageId}/promote`, { method: 'PUT' }),
  deleteImage: (imageId: string) => request<void>(`/api/images/${imageId}`, { method: 'DELETE' }),
  /// Force a render of this file, whatever the automatic pass chose. Returns the
  /// job, so the caller can wait for *its* picture rather than watching the queue.
  renderFile: (fileId: string) =>
    request<{ job_id: number }>(`/api/files/${fileId}/render`, { method: 'POST' }),
  job: (jobId: number) => request<Job>(`/api/jobs/${jobId}`),

  creators: (q = '') => request<Creator[]>(`/api/creators?q=${encodeURIComponent(q)}`),
  creator: (id: string) => request<Creator>(`/api/creators/${id}`),
  createCreator: (body: unknown) => request<Creator>('/api/creators', json(body)),
  updateCreator: (id: string, body: unknown) =>
    request<Creator>(`/api/creators/${id}`, { ...json(body), method: 'PUT' }),

  tags: (q = '') => request<Tag[]>(`/api/tags?q=${encodeURIComponent(q)}`),
  variantTags: (q = '') => request<VariantTag[]>(`/api/variant-tags?q=${encodeURIComponent(q)}`),

  jobs: (status = '') => request<Job[]>(`/api/jobs?status=${status}`),
  retryJob: (id: number) => request<void>(`/api/jobs/${id}/retry`, { method: 'POST' }),

  rendererConfig: () => request<RendererConfig>('/api/admin/settings/renderer'),
  setRendererConfig: (config: RendererConfig) =>
    request<RendererConfig>('/api/admin/settings/renderer', { ...json(config), method: 'PUT' }),
  rerender: (scope: 'stale' | 'all', mode: 'add' | 'replace') =>
    request<{ jobs_queued: number }>('/api/admin/rerender', json({ scope, mode })),

  previewBundlePatch: (bundleId: string, zip: File) => {
    const form = new FormData()
    form.append('file', zip)
    return request<PatchPreview>(`/api/bundles/${bundleId}/patch/preview`, {
      method: 'POST',
      body: form,
    })
  },
  applyBundlePatch: (bundleId: string, zip: File, options: PatchApplyOptions) => {
    const form = new FormData()
    form.append('options', JSON.stringify(options))
    form.append('file', zip)
    return request<PatchApplyResult>(`/api/bundles/${bundleId}/patch`, {
      method: 'POST',
      body: form,
    })
  },
}

// --- Bundle metadata patch --------------------------------------------------

export interface PatchPreview {
  bundle_has_description: boolean
  bundle_cover_count: number
  matched: {
    patch_name: string
    model_id: string
    model_name: string
    add_tags: string[]
    has_image: boolean
  }[]
  ambiguous: { patch_name: string; candidates: { id: string; name: string }[] }[]
  unmatched_patch: string[]
  unmatched_members: string[]
  members: { id: string; name: string }[]
}

export interface PatchApplyOptions {
  rename_models: boolean
  model_tags: 'merge' | 'replace' | 'skip'
  model_images: 'replace_generated' | 'add' | 'skip'
  bundle_cover: boolean
  bundle_description: boolean
  /** patch model label -> chosen member id; resolves ambiguous / adopts unmatched */
  matches: Record<string, string>
}

export interface PatchApplyResult {
  models_updated: number
  images_added: number
  tags_added: number
}

export const imageUrl = (id: string) => `/api/images/${id}`
export const downloadUrl = (fileId: string) => `/api/files/${fileId}/download`

/// How to refer to a variant in prose: its label if it has one, else its tags,
/// else the fact that it is the model's untagged bucket of files.
export function variantLabel(variant: VariantDetail): string {
  if (variant.name) return variant.name
  if (variant.tags.length) return variant.tags.join(' + ')
  return 'Untagged'
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = ''
  for (const u of units) {
    value /= 1024
    unit = u
    if (value < 1024) break
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${unit}`
}
