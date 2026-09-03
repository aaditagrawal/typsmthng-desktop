import { createRoot } from 'react-dom/client'
import './index.css'
import { perfMark, perfMeasure } from './lib/perf'

window.onerror = (msg, src, line, col, err) => {
  document.body.innerHTML = `<pre style="color:red;padding:20px">${msg}\n${src}:${line}:${col}\n${err?.stack ?? ''}</pre>`
}
window.onunhandledrejection = (e) => {
  document.body.innerHTML = `<pre style="color:red;padding:20px">Unhandled rejection: ${e.reason}\n${e.reason?.stack ?? ''}</pre>`
}

const bootstrapStart = perfMark()
const root = createRoot(document.getElementById('root')!)

// The audience window loads the same bundle with a hash route so it needs no
// extra Vite entry or Electrobun view; it never touches vaults or the compiler.
const isAudienceWindow = window.location.hash.startsWith('#/audience')

const appModule = isAudienceWindow
  ? import('./components/presentation/audience-app.tsx')
  : import('./App.tsx')

void appModule
  .then(({ default: App }) => {
    perfMeasure('renderer.app_module', bootstrapStart)
    root.render(<App />)
  })
  .catch((error) => {
    document.body.innerHTML = `<pre style="color:red;padding:20px">Failed to load ${isAudienceWindow ? 'audience view' : 'App.tsx'}\n${error?.stack ?? error}</pre>`
    console.error('Failed to bootstrap renderer', error)
  })
