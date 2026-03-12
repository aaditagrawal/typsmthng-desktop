/// <reference types="vite/client" />

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
