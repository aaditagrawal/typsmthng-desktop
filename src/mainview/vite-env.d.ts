/// <reference types="vite/client" />

declare const __APP_VERSION__: string

declare module '*.wasm?url' {
  const url: string
  export default url
}

interface LocalFontData {
  family: string
  fullName: string
  postscriptName: string
  style: string
  blob: () => Promise<Blob>
}

interface Window {
  queryLocalFonts?: () => Promise<LocalFontData[]>
}
