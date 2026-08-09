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

void import('./App.tsx')
  .then(({ default: App }) => {
    perfMeasure('renderer.app_module', bootstrapStart)
    root.render(<App />)
  })
  .catch((error) => {
    document.body.innerHTML = `<pre style="color:red;padding:20px">Failed to load App.tsx\n${error?.stack ?? error}</pre>`
    console.error('Failed to bootstrap renderer', error)
  })
