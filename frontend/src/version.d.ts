/// The build's version stamp, substituted at build time by Vite's `define`
/// (see vite.config.ts). Not a runtime lookup — it is baked into the bundle,
/// which is exactly what makes it usable to detect that the *server* has since
/// been redeployed out from under this loaded page.
declare const __APP_VERSION__: string
