import type { KeyBinding } from '@codemirror/view'
import { useProjectStore } from '@/stores/project-store'
import { useEditorStore } from '@/stores/editor-store'
import { useSettingsStore } from '@/stores/settings-store'
import { forceCompile, compileCurrentToPdf } from './compile-manager'
import { toggleTypstLineComment } from './commenting'
import { downloadBlob } from './download-blob'

export const typstKeymap: KeyBinding[] = [
  {
    key: 'Mod-/',
    run: (view) => toggleTypstLineComment(view),
  },
  {
    key: 'Mod-s',
    run: () => {
      const projectStore = useProjectStore.getState()
      const currentPath = projectStore.currentFilePath
      if (currentPath) {
        projectStore.updateFileContent(currentPath, useEditorStore.getState().source)
      }
      projectStore.saveCurrentProject()
      return true
    },
  },
  {
    key: 'Mod-Enter',
    run: (view) => {
      const currentPath = useProjectStore.getState().currentFilePath
      forceCompile(view.state.doc.toString(), currentPath)
      return true
    },
  },
  {
    key: 'Mod-Shift-Enter',
    run: (view) => {
      useEditorStore.setState({ source: view.state.doc.toString() })
      void compileCurrentToPdf()
        .then(async (pdf) => {
          if (pdf) {
            const blob = new Blob([new Uint8Array(pdf)], { type: 'application/pdf' })
            await downloadBlob('document.pdf', blob)
          }
        })
        .catch((err) => {
          console.error('Failed to export PDF:', err)
          window.alert('Failed to export PDF. Please try again.')
        })
      return true
    },
  },
  {
    key: 'Mod-j',
    run: () => {
      const { theme, setTheme } = useSettingsStore.getState()
      const next = theme === 'system' ? 'light' : theme === 'light' ? 'dark' : 'system'
      setTheme(next)
      return true
    },
  },
]
