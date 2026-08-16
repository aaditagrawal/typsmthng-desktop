import { create } from 'zustand'

interface PreviewState {
  zoom: number
  fitMode: 'width' | 'page' | 'custom'
  currentPage: number
  mode: 'svg' | 'live'
  setZoom: (zoom: number) => void
  setFitMode: (mode: 'width' | 'page' | 'custom') => void
  setCurrentPage: (page: number) => void
  setMode: (mode: 'svg' | 'live') => void
}

export const usePreviewStore = create<PreviewState>((set, get) => ({
  zoom: 100,
  fitMode: 'width',
  currentPage: 1,
  mode: 'svg',

  setZoom: (zoom) => {
    const nextZoom = Math.max(10, Math.min(500, zoom))
    const state = get()
    if (state.zoom === nextZoom && state.fitMode === 'custom') return
    set({ zoom: nextZoom, fitMode: 'custom' })
  },
  setFitMode: (fitMode) => {
    if (get().fitMode === fitMode) return
    set({ fitMode })
  },
  setCurrentPage: (currentPage) => {
    if (get().currentPage === currentPage) return
    set({ currentPage })
  },
  setMode: (mode) => {
    if (get().mode === mode) return
    set({ mode })
  },
}))
